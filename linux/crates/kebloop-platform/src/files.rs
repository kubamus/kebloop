//! Background file indexer and in-memory query.
//!
//! Walks the configured roots with the `ignore` crate (respecting `.gitignore`,
//! global and `.git/info/exclude` rules and skipping hidden entries), applies a
//! caller supplied exclusion predicate, and publishes an immutable
//! [`FileIndex`] snapshot that answers case- and diacritic-insensitive
//! substring queries from memory.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use ignore::{WalkBuilder, WalkState};

use crate::text::fold;

/// Default maximum number of indexed entries.
pub const DEFAULT_MAX_ENTRIES: usize = 300_000;

/// Folders in the home directory that rank higher (mirrors the macOS ranking).
const PREFERRED_FOLDERS: [&str; 4] = ["Documents", "Desktop", "Downloads", "Public"];

/// Exclusion predicate: return `true` to exclude a path (directories excluded
/// here are not descended into).
pub type ExcludeFn = Arc<dyn Fn(&Path) -> bool + Send + Sync>;

#[derive(Debug, thiserror::Error)]
pub enum FilesError {
    #[error("indexing cancelled")]
    Cancelled,
}

/// Cooperative cancellation flag shared between a rebuild and its owner.
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// Indexer configuration.
#[derive(Clone)]
pub struct FileIndexConfig {
    /// Folders walked recursively.
    pub roots: Vec<PathBuf>,
    /// Folders whose direct children (files and folders) are indexed without
    /// recursion, e.g. the home directory.
    pub shallow_roots: Vec<PathBuf>,
    /// Home directory, used for ranking bonuses.
    pub home: PathBuf,
    pub max_entries: usize,
    /// Walker threads (0 = automatic, capped at 8).
    pub threads: usize,
    pub exclude: ExcludeFn,
}

impl FileIndexConfig {
    /// Defaults for `home`: XDG user directories (recursive) plus the home
    /// top level (shallow).
    pub fn defaults(home: &Path, user_dirs_file: &Path, exclude: ExcludeFn) -> Self {
        Self {
            roots: default_roots(home, user_dirs_file),
            shallow_roots: vec![home.to_path_buf()],
            home: home.to_path_buf(),
            max_entries: DEFAULT_MAX_ENTRIES,
            threads: 0,
            exclude,
        }
    }

    /// Explicit roots, no shallow roots, nothing excluded (tests, custom folders).
    pub fn with_roots(roots: Vec<PathBuf>, home: PathBuf) -> Self {
        Self {
            roots,
            shallow_roots: Vec::new(),
            home,
            max_entries: DEFAULT_MAX_ENTRIES,
            threads: 0,
            exclude: Arc::new(|_| false),
        }
    }
}

/// Parses `user-dirs.dirs` content into `XDG_*_DIR` → absolute path.
pub fn parse_user_dirs(content: &str, home: &Path) -> HashMap<String, PathBuf> {
    let mut out = HashMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if !(key.starts_with("XDG_") && key.ends_with("_DIR")) {
            continue;
        }
        let value = value.trim();
        let Some(value) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) else {
            continue;
        };
        let unescaped = unescape_shell(value);
        let path = if let Some(rest) = unescaped.strip_prefix("$HOME") {
            home.join(rest.trim_start_matches('/'))
        } else if unescaped.starts_with('/') {
            PathBuf::from(unescaped)
        } else {
            continue;
        };
        out.insert(key.to_owned(), path);
    }
    out
}

fn unescape_shell(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(n) = chars.next() {
                out.push(n);
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Default recursive roots: Documents, Desktop, Downloads, Pictures, Music,
/// Videos, Public from `user-dirs.dirs` (falling back to `$HOME/<English name>`
/// when unset). The home directory itself is never a root; entries equal to
/// `home` are dropped, as are non-existent folders.
pub fn default_roots(home: &Path, user_dirs_file: &Path) -> Vec<PathBuf> {
    let content = std::fs::read_to_string(user_dirs_file).unwrap_or_default();
    let dirs = parse_user_dirs(&content, home);
    let mut roots: Vec<PathBuf> = Vec::new();
    for (key, fallback) in [
        ("XDG_DOCUMENTS_DIR", "Documents"),
        ("XDG_DESKTOP_DIR", "Desktop"),
        ("XDG_DOWNLOAD_DIR", "Downloads"),
        ("XDG_PICTURES_DIR", "Pictures"),
        ("XDG_MUSIC_DIR", "Music"),
        ("XDG_VIDEOS_DIR", "Videos"),
        ("XDG_PUBLICSHARE_DIR", "Public"),
    ] {
        let path = dirs
            .get(key)
            .cloned()
            .unwrap_or_else(|| home.join(fallback));
        if path != home && path.is_dir() && !roots.contains(&path) {
            roots.push(path);
        }
    }
    roots
}

// ---------------------------------------------------------------------------
// Index
// ---------------------------------------------------------------------------

/// One query hit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileMatch {
    pub path: String,
    pub name: String,
    pub is_dir: bool,
}

/// Immutable snapshot of indexed files. Paths are stored in one arena and the
/// folded file names in another (`\n`-separated) so queries are a single
/// vectorised substring scan.
#[derive(Debug, Default)]
pub struct FileIndex {
    paths: String,
    /// End offset of each path in `paths`.
    path_end: Vec<u32>,
    /// Offset of the file name inside each path.
    name_start: Vec<u32>,
    is_dir: Vec<bool>,
    /// `"\n" + folded_name + "\n" + ...`
    folded: String,
    /// Start offset of each folded name inside `folded`.
    fold_start: Vec<u32>,
    home_prefixes: Vec<String>,
    /// True when the entry cap truncated the walk.
    pub truncated: bool,
}

const MAX_CANDIDATES: usize = 4096;

impl FileIndex {
    pub fn len(&self) -> usize {
        self.path_end.len()
    }

    pub fn is_empty(&self) -> bool {
        self.path_end.is_empty()
    }

    fn path(&self, i: usize) -> &str {
        let start = if i == 0 {
            0
        } else {
            self.path_end[i - 1] as usize
        };
        &self.paths[start..self.path_end[i] as usize]
    }

    fn name(&self, i: usize) -> &str {
        let start = if i == 0 {
            0
        } else {
            self.path_end[i - 1] as usize
        };
        &self.paths[start + self.name_start[i] as usize..self.path_end[i] as usize]
    }

    fn folded_name(&self, i: usize) -> &str {
        let start = self.fold_start[i] as usize;
        let end = self
            .fold_start
            .get(i + 1)
            .map_or(self.folded.len(), |&n| n as usize - 1);
        &self.folded[start..end]
    }

    fn entry_at_folded_offset(&self, offset: usize) -> usize {
        self.fold_start
            .partition_point(|&s| (s as usize) <= offset)
            .saturating_sub(1)
    }

    fn match_of(&self, i: usize) -> FileMatch {
        FileMatch {
            path: self.path(i).to_owned(),
            name: self.name(i).to_owned(),
            is_dir: self.is_dir[i],
        }
    }

    /// Case- and diacritic-insensitive substring query on file names, ranked
    /// (exact name or stem, then prefix, then substring; preferred home folders
    /// first; then natural name order) and limited to `limit` results.
    pub fn query(&self, text: &str, limit: usize) -> Vec<FileMatch> {
        let needle = fold(text.trim()).replace('\n', " ");
        if needle.is_empty() || limit == 0 || self.is_empty() {
            return Vec::new();
        }
        let mut candidates: Vec<usize> = Vec::new();
        // Prefix matches first: search "\n" + needle.
        let prefix_pat = format!("\n{needle}");
        let mut from = 0;
        while candidates.len() < MAX_CANDIDATES {
            let Some(pos) = self.folded[from..].find(&prefix_pat) else {
                break;
            };
            let offset = from + pos + 1;
            let idx = self.entry_at_folded_offset(offset);
            candidates.push(idx);
            from = self.folded[offset..]
                .find('\n')
                .map_or(self.folded.len(), |e| offset + e);
        }
        // Then any substring, skipping entries already collected.
        let mut seen_prefix: Vec<usize> = candidates.clone();
        seen_prefix.sort_unstable();
        let mut from = 0;
        while candidates.len() < MAX_CANDIDATES {
            let Some(pos) = self.folded[from..].find(&needle) else {
                break;
            };
            let offset = from + pos;
            let idx = self.entry_at_folded_offset(offset);
            if seen_prefix.binary_search(&idx).is_err() {
                candidates.push(idx);
            }
            let next_entry = idx + 1;
            from = if next_entry < self.fold_start.len() {
                self.fold_start[next_entry] as usize
            } else {
                break;
            };
        }

        let mut scored: Vec<(u32, usize)> = candidates
            .into_iter()
            .map(|i| (self.score(i, &needle), i))
            .collect();
        scored.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| {
                    crate::apps::natural_cmp_folded(self.folded_name(a.1), self.folded_name(b.1))
                })
                .then_with(|| self.path(a.1).cmp(self.path(b.1)))
        });
        scored.truncate(limit);
        scored.into_iter().map(|(_, i)| self.match_of(i)).collect()
    }

    fn score(&self, i: usize, needle: &str) -> u32 {
        let name = self.folded_name(i);
        let stem = name.rsplit_once('.').map_or(name, |(s, _)| s);
        let relevance = if name == needle || stem == needle {
            3
        } else if name.starts_with(needle) {
            2
        } else {
            1
        };
        let path = self.path(i);
        let preferred = self
            .home_prefixes
            .iter()
            .any(|p| path.starts_with(p.as_str()));
        relevance * 10 + u32::from(preferred)
    }

    fn from_paths(mut entries: Vec<(String, bool)>, home: &Path, truncated: bool) -> Self {
        entries.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        entries.dedup_by(|a, b| a.0 == b.0);
        let mut index = Self {
            home_prefixes: PREFERRED_FOLDERS
                .iter()
                .map(|f| format!("{}/{f}/", home.display()))
                .collect(),
            truncated,
            folded: String::from("\n"),
            ..Self::default()
        };
        for (path, is_dir) in entries {
            if index.paths.len() + path.len() >= u32::MAX as usize - 1 {
                index.truncated = true;
                break;
            }
            let name_start = path.rfind('/').map_or(0, |p| p + 1);
            index.fold_start.push(index.folded.len() as u32);
            index
                .folded
                .push_str(&fold(&path[name_start..]).replace('\n', " "));
            index.folded.push('\n');
            index.name_start.push(name_start as u32);
            index.paths.push_str(&path);
            index.path_end.push(index.paths.len() as u32);
            index.is_dir.push(is_dir);
        }
        index
    }
}

// ---------------------------------------------------------------------------
// Building
// ---------------------------------------------------------------------------

struct Batch {
    local: Vec<(String, bool)>,
    sink: Arc<Mutex<Vec<(String, bool)>>>,
}

impl Batch {
    fn flush(&mut self) {
        if self.local.is_empty() {
            return;
        }
        if let Ok(mut sink) = self.sink.lock() {
            sink.append(&mut self.local);
        }
        self.local.clear();
    }
}

impl Drop for Batch {
    fn drop(&mut self) {
        self.flush();
    }
}

fn walker_threads(configured: usize) -> usize {
    if configured > 0 {
        return configured;
    }
    std::thread::available_parallelism().map_or(4, |n| n.get().clamp(2, 8))
}

fn root_builder(root: &Path, exclude: ExcludeFn) -> WalkBuilder {
    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .parents(true)
        .ignore(false)
        // A stray `.gitignore` outside a repository must not hide files.
        .require_git(true)
        .follow_links(false);
    builder.filter_entry(move |e| !exclude(e.path()));
    builder
}

/// Fast parallel walk of every root. Returns `Ok(true)` when the entry cap was hit (the result
/// is then arbitrary and must be redone deterministically).
fn walk_parallel(
    cfg: &FileIndexConfig,
    cancel: &CancelToken,
    count: &Arc<AtomicUsize>,
    sink: &Arc<Mutex<Vec<(String, bool)>>>,
) -> Result<bool, FilesError> {
    let cap = cfg.max_entries;
    for root in cfg.roots.iter().filter(|r| r.is_dir()) {
        if cancel.is_cancelled() {
            return Err(FilesError::Cancelled);
        }
        let mut builder = root_builder(root, cfg.exclude.clone());
        builder.threads(walker_threads(cfg.threads));
        builder.build_parallel().run(|| {
            let mut batch = Batch {
                local: Vec::with_capacity(512),
                sink: sink.clone(),
            };
            let count = count.clone();
            let cancel = cancel.clone();
            Box::new(move |res| {
                if cancel.is_cancelled() {
                    return WalkState::Quit;
                }
                let Ok(entry) = res else {
                    return WalkState::Continue;
                };
                if entry.depth() == 0 {
                    return WalkState::Continue;
                }
                let Some(path) = entry.path().to_str() else {
                    return WalkState::Continue;
                };
                let is_dir = entry.file_type().is_some_and(|t| t.is_dir());
                if count.fetch_add(1, Ordering::Relaxed) >= cap {
                    return WalkState::Quit;
                }
                batch.local.push((path.to_owned(), is_dir));
                if batch.local.len() >= 512 {
                    batch.flush();
                }
                WalkState::Continue
            })
        });
        if count.load(Ordering::Relaxed) >= cap {
            break;
        }
    }
    Ok(count.load(Ordering::Relaxed) > cap)
}

/// Single-threaded walk in sorted order, stopping after `cap` entries. Used only when the cap
/// truncates, so that the same file tree always yields the same index (results do not flap
/// between rebuilds).
fn walk_sorted(
    cfg: &FileIndexConfig,
    cancel: &CancelToken,
    entries: &mut Vec<(String, bool)>,
) -> Result<(), FilesError> {
    let cap = cfg.max_entries;
    for root in cfg.roots.iter().filter(|r| r.is_dir()) {
        let mut builder = root_builder(root, cfg.exclude.clone());
        builder.sort_by_file_name(|a, b| a.cmp(b));
        for res in builder.build() {
            if cancel.is_cancelled() {
                return Err(FilesError::Cancelled);
            }
            let Ok(entry) = res else {
                continue;
            };
            if entry.depth() == 0 {
                continue;
            }
            let Some(path) = entry.path().to_str() else {
                continue;
            };
            if entries.len() >= cap {
                return Ok(());
            }
            let is_dir = entry.file_type().is_some_and(|t| t.is_dir());
            entries.push((path.to_owned(), is_dir));
        }
    }
    Ok(())
}

/// Builds an index synchronously. Checks `cancel` frequently and returns
/// [`FilesError::Cancelled`] if it fires. When the entry cap truncates the walk the walk is
/// repeated in sorted order so the truncated result is deterministic.
pub fn build_index(cfg: &FileIndexConfig, cancel: &CancelToken) -> Result<FileIndex, FilesError> {
    let count = Arc::new(AtomicUsize::new(0));
    let sink: Arc<Mutex<Vec<(String, bool)>>> = Arc::new(Mutex::new(Vec::new()));
    let cap = cfg.max_entries;

    let mut truncated = walk_parallel(cfg, cancel, &count, &sink)?;
    let sink = if truncated {
        let mut sorted = Vec::new();
        walk_sorted(cfg, cancel, &mut sorted)?;
        count.store(sorted.len(), Ordering::Relaxed);
        Arc::new(Mutex::new(sorted))
    } else {
        sink
    };

    for shallow in cfg.shallow_roots.iter().filter(|r| r.is_dir()) {
        let Ok(read) = std::fs::read_dir(shallow) else {
            continue;
        };
        let mut local = Vec::new();
        let mut listing: Vec<_> = read.filter_map(Result::ok).collect();
        listing.sort_by_key(|e| e.file_name());
        for entry in listing {
            if cancel.is_cancelled() {
                return Err(FilesError::Cancelled);
            }
            let name = entry.file_name();
            let Some(name_str) = name.to_str() else {
                continue;
            };
            if name_str.starts_with('.') {
                continue;
            }
            let path = entry.path();
            if (cfg.exclude)(&path) {
                continue;
            }
            let Some(path_str) = path.to_str() else {
                continue;
            };
            let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
            local.push((path_str.to_owned(), is_dir));
        }
        if count.fetch_add(local.len(), Ordering::Relaxed) < cap
            && let Ok(mut s) = sink.lock()
        {
            s.append(&mut local);
        }
    }

    if cancel.is_cancelled() {
        return Err(FilesError::Cancelled);
    }
    truncated |= count.load(Ordering::Relaxed) > cap;
    let entries = match Arc::try_unwrap(sink) {
        Ok(m) => m.into_inner().unwrap_or_default(),
        Err(arc) => arc
            .lock()
            .map(|mut g| std::mem::take(&mut *g))
            .unwrap_or_default(),
    };
    Ok(FileIndex::from_paths(entries, &cfg.home, truncated))
}

// ---------------------------------------------------------------------------
// Background indexer
// ---------------------------------------------------------------------------

/// Notification that a rebuild finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileIndexEvent {
    /// A new snapshot is available through [`FileIndexer::snapshot`].
    Updated { entries: usize },
}

struct IndexerState {
    snapshot: Arc<FileIndex>,
    token: CancelToken,
    generation: u64,
    building: bool,
}

/// Owns the current snapshot and rebuilds it on a background thread.
pub struct FileIndexer {
    state: Arc<Mutex<IndexerState>>,
    events: async_channel::Sender<FileIndexEvent>,
}

impl FileIndexer {
    /// Creates an indexer (empty snapshot) and the channel for update events.
    pub fn new() -> (Self, async_channel::Receiver<FileIndexEvent>) {
        let (tx, rx) = async_channel::unbounded();
        (
            Self {
                state: Arc::new(Mutex::new(IndexerState {
                    snapshot: Arc::new(FileIndex::default()),
                    token: CancelToken::new(),
                    generation: 0,
                    building: false,
                })),
                events: tx,
            },
            rx,
        )
    }

    /// Current snapshot (cheap clone of an `Arc`).
    pub fn snapshot(&self) -> Arc<FileIndex> {
        self.state
            .lock()
            .map(|s| s.snapshot.clone())
            .unwrap_or_default()
    }

    /// Whether a rebuild is currently running.
    pub fn is_building(&self) -> bool {
        self.state.lock().is_ok_and(|s| s.building)
    }

    /// Cancels any running rebuild without starting a new one.
    pub fn cancel(&self) {
        if let Ok(mut s) = self.state.lock() {
            s.token.cancel();
            s.building = false;
            s.generation += 1;
        }
    }

    /// Cancels a running rebuild and starts a new one in the background.
    pub fn rebuild(&self, cfg: FileIndexConfig) {
        let (token, generation) = {
            let Ok(mut s) = self.state.lock() else {
                return;
            };
            s.token.cancel();
            s.token = CancelToken::new();
            s.generation += 1;
            s.building = true;
            (s.token.clone(), s.generation)
        };
        let state = self.state.clone();
        let events = self.events.clone();
        let spawned = std::thread::Builder::new()
            .name("kebloop-file-index".into())
            .spawn(move || {
                let result = build_index(&cfg, &token);
                let Ok(mut s) = state.lock() else {
                    return;
                };
                if s.generation != generation {
                    return;
                }
                s.building = false;
                if let Ok(index) = result {
                    let entries = index.len();
                    s.snapshot = Arc::new(index);
                    drop(s);
                    let _ = events.try_send(FileIndexEvent::Updated { entries });
                }
            });
        if spawned.is_err()
            && let Ok(mut s) = self.state.lock()
        {
            s.building = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{Duration, Instant};
    use tempfile::TempDir;

    fn touch(root: &Path, rel: &str) {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, "x").unwrap();
    }

    fn names(hits: &[FileMatch]) -> Vec<&str> {
        hits.iter().map(|h| h.name.as_str()).collect()
    }

    fn index_of(root: &Path) -> FileIndex {
        let cfg = FileIndexConfig::with_roots(vec![root.to_path_buf()], root.to_path_buf());
        build_index(&cfg, &CancelToken::new()).unwrap()
    }

    #[test]
    fn user_dirs_parsing() {
        let home = Path::new("/home/u");
        let content = "# c\nXDG_DESKTOP_DIR=\"$HOME/Pulpit\"\nXDG_DOCUMENTS_DIR=\"$HOME/Dokumenty\"\nXDG_MUSIC_DIR=\"/mnt/music\"\nXDG_BAD=\"x\"\nXDG_VIDEOS_DIR=\"relative\"\nXDG_DOWNLOAD_DIR=\"$HOME/Pobrane\\ files\"\n";
        let m = parse_user_dirs(content, home);
        assert_eq!(m["XDG_DESKTOP_DIR"], Path::new("/home/u/Pulpit"));
        assert_eq!(m["XDG_DOCUMENTS_DIR"], Path::new("/home/u/Dokumenty"));
        assert_eq!(m["XDG_MUSIC_DIR"], Path::new("/mnt/music"));
        assert_eq!(m["XDG_DOWNLOAD_DIR"], Path::new("/home/u/Pobrane files"));
        assert!(!m.contains_key("XDG_VIDEOS_DIR"));
        assert!(!m.contains_key("XDG_BAD"));
    }

    #[test]
    fn default_roots_use_user_dirs_and_skip_missing() {
        let tmp = TempDir::new().unwrap();
        let home = tmp.path();
        fs::create_dir_all(home.join("Dokumenty")).unwrap();
        fs::create_dir_all(home.join("Music")).unwrap();
        fs::create_dir_all(home.join(".config")).unwrap();
        let file = home.join(".config/user-dirs.dirs");
        fs::write(
            &file,
            "XDG_DOCUMENTS_DIR=\"$HOME/Dokumenty\"\nXDG_DESKTOP_DIR=\"$HOME\"\n",
        )
        .unwrap();
        let roots = default_roots(home, &file);
        assert_eq!(roots, vec![home.join("Dokumenty"), home.join("Music")]);
    }

    #[test]
    fn indexes_and_queries_with_folding() {
        let tmp = TempDir::new().unwrap();
        touch(tmp.path(), "Dokumenty/Zażółć Gęślą.txt");
        touch(tmp.path(), "Dokumenty/report-2024.pdf");
        touch(tmp.path(), "Dokumenty/sub/Report.md");
        touch(tmp.path(), "Dokumenty/.hidden/report.txt");
        touch(tmp.path(), "Dokumenty/.secret-report");
        let index = index_of(tmp.path());
        let hits = index.query("zazołc", 10);
        assert_eq!(names(&hits), ["Zażółć Gęślą.txt"]);
        let hits = index.query("REPORT", 10);
        // exact stem first, then prefix, hidden entries excluded.
        assert_eq!(names(&hits), ["Report.md", "report-2024.pdf"]);
        assert!(index.query("hidden", 10).is_empty());
        assert!(index.query("   ", 10).is_empty());
        assert!(index.query("nothing-like-this", 10).is_empty());
        let sub = index.query("sub", 10);
        assert_eq!(sub.len(), 1);
        assert!(sub[0].is_dir);
    }

    #[test]
    fn substring_ranks_after_prefix_and_limit_applies() {
        let tmp = TempDir::new().unwrap();
        for n in [
            "xnotes.txt",
            "notes.txt",
            "my-notes.txt",
            "notes-old.txt",
            "notes",
        ] {
            touch(tmp.path(), &format!("d/{n}"));
        }
        let index = index_of(tmp.path());
        let hits = index.query("notes", 10);
        assert_eq!(
            names(&hits),
            [
                "notes",
                "notes.txt",
                "notes-old.txt",
                "my-notes.txt",
                "xnotes.txt"
            ]
        );
        assert_eq!(index.query("notes", 2).len(), 2);
    }

    #[test]
    fn preferred_home_folders_rank_first() {
        let tmp = TempDir::new().unwrap();
        touch(tmp.path(), "Documents/budget.ods");
        touch(tmp.path(), "Other/budget.ods");
        let cfg =
            FileIndexConfig::with_roots(vec![tmp.path().to_path_buf()], tmp.path().to_path_buf());
        let index = build_index(&cfg, &CancelToken::new()).unwrap();
        let hits = index.query("budget", 5);
        assert!(hits[0].path.contains("/Documents/"));
    }

    #[test]
    fn gitignore_nested_negation_and_exclude_predicate() {
        let tmp = TempDir::new().unwrap();
        let p = tmp.path();
        fs::create_dir_all(p.join("proj/.git")).unwrap();
        touch(p, "proj/.gitignore");
        fs::write(p.join("proj/.gitignore"), "build/\n*.log\n!keep.log\n").unwrap();
        touch(p, "proj/build/out.bin");
        touch(p, "proj/main.rs");
        touch(p, "proj/debug.log");
        touch(p, "proj/keep.log");
        touch(p, "proj/node_modules/pkg/index.js");
        touch(p, "proj/sub/.gitignore");
        fs::write(p.join("proj/sub/.gitignore"), "local.txt\n").unwrap();
        touch(p, "proj/sub/local.txt");
        touch(p, "proj/sub/other.txt");
        let mut cfg = FileIndexConfig::with_roots(vec![p.to_path_buf()], p.to_path_buf());
        cfg.exclude = Arc::new(|path: &Path| path.file_name().is_some_and(|n| n == "node_modules"));
        let index = build_index(&cfg, &CancelToken::new()).unwrap();
        for hidden in ["out.bin", "debug.log", "local.txt", "index.js", "pkg"] {
            assert!(
                index.query(hidden, 5).is_empty(),
                "{hidden} must be excluded"
            );
        }
        for shown in ["main.rs", "keep.log", "other.txt"] {
            assert_eq!(index.query(shown, 5).len(), 1, "{shown} must be indexed");
        }
    }

    #[test]
    fn shallow_roots_do_not_recurse_and_skip_hidden() {
        let tmp = TempDir::new().unwrap();
        let p = tmp.path();
        touch(p, "top.txt");
        touch(p, ".dotfile");
        touch(p, "dir/deep.txt");
        let mut cfg = FileIndexConfig::with_roots(vec![], p.to_path_buf());
        cfg.shallow_roots = vec![p.to_path_buf()];
        let index = build_index(&cfg, &CancelToken::new()).unwrap();
        assert_eq!(index.query("top", 5).len(), 1);
        assert_eq!(index.query("dir", 5).len(), 1);
        assert!(index.query("deep", 5).is_empty());
        assert!(index.query("dotfile", 5).is_empty());
    }

    #[test]
    fn entry_cap_truncates() {
        let tmp = TempDir::new().unwrap();
        for i in 0..200 {
            touch(tmp.path(), &format!("d/file{i}.txt"));
        }
        let mut cfg =
            FileIndexConfig::with_roots(vec![tmp.path().to_path_buf()], tmp.path().to_path_buf());
        cfg.max_entries = 50;
        let index = build_index(&cfg, &CancelToken::new()).unwrap();
        assert!(index.truncated);
        assert_eq!(index.len(), 50);
    }

    #[test]
    fn truncated_index_is_deterministic() {
        let tmp = TempDir::new().unwrap();
        for d in ["a", "b", "c", "d"] {
            for i in 0..40 {
                touch(tmp.path(), &format!("{d}/file{i:02}.txt"));
            }
        }
        let mut cfg =
            FileIndexConfig::with_roots(vec![tmp.path().to_path_buf()], tmp.path().to_path_buf());
        cfg.max_entries = 70;
        let snapshot = |cfg: &FileIndexConfig| {
            let index = build_index(cfg, &CancelToken::new()).unwrap();
            assert!(index.truncated);
            (0..index.len())
                .map(|i| index.path(i).to_owned())
                .collect::<Vec<_>>()
        };
        let first = snapshot(&cfg);
        assert_eq!(first.len(), 70);
        for threads in [1, 2, 8, 8, 4] {
            cfg.threads = threads;
            assert_eq!(snapshot(&cfg), first);
        }
        // Sorted walk: "a" is complete before "b" is entered, "c"/"d" are never reached.
        assert!(first.iter().any(|p| p.ends_with("a/file39.txt")));
        assert!(!first.iter().any(|p| p.contains("/c/") || p.contains("/d/")));
    }

    #[test]
    fn stray_gitignore_outside_repository_is_ignored() {
        let tmp = TempDir::new().unwrap();
        let p = tmp.path();
        fs::write(p.join(".gitignore"), "*.txt\n").unwrap();
        touch(p, "plain/notes.txt");
        let index = index_of(p);
        assert_eq!(index.query("notes", 5).len(), 1);
    }

    #[test]
    fn cancelled_build_returns_error() {
        let tmp = TempDir::new().unwrap();
        touch(tmp.path(), "a.txt");
        let token = CancelToken::new();
        token.cancel();
        let cfg =
            FileIndexConfig::with_roots(vec![tmp.path().to_path_buf()], tmp.path().to_path_buf());
        assert!(matches!(
            build_index(&cfg, &token),
            Err(FilesError::Cancelled)
        ));
    }

    #[test]
    fn background_indexer_publishes_and_replaces_snapshot() {
        let tmp = TempDir::new().unwrap();
        touch(tmp.path(), "one.txt");
        let (indexer, rx) = FileIndexer::new();
        assert!(indexer.snapshot().is_empty());
        let cfg =
            FileIndexConfig::with_roots(vec![tmp.path().to_path_buf()], tmp.path().to_path_buf());
        indexer.rebuild(cfg.clone());
        let start = Instant::now();
        loop {
            if rx.try_recv().is_ok() {
                break;
            }
            assert!(start.elapsed() < Duration::from_secs(10), "no update event");
            std::thread::sleep(Duration::from_millis(10));
        }
        let old = indexer.snapshot();
        assert_eq!(old.query("one", 3).len(), 1);
        touch(tmp.path(), "two.txt");
        indexer.rebuild(cfg);
        while rx.try_recv().is_err() {
            assert!(start.elapsed() < Duration::from_secs(20));
            std::thread::sleep(Duration::from_millis(10));
        }
        // The old snapshot stays valid; the new one sees the new file.
        assert!(old.query("two", 3).is_empty());
        assert_eq!(indexer.snapshot().query("two", 3).len(), 1);
        assert!(!indexer.is_building());
    }

    #[test]
    fn benchmark_generated_tree() {
        let tmp = TempDir::new().unwrap();
        let mut total = 0;
        for d in 0..40 {
            for s in 0..10 {
                let dir = tmp.path().join(format!("Documents/dir{d}/sub{s}"));
                fs::create_dir_all(&dir).unwrap();
                for f in 0..50 {
                    fs::write(dir.join(format!("file-{d}-{s}-{f}.dat")), "").unwrap();
                    total += 1;
                }
            }
        }
        let started = Instant::now();
        let cfg =
            FileIndexConfig::with_roots(vec![tmp.path().to_path_buf()], tmp.path().to_path_buf());
        let index = build_index(&cfg, &CancelToken::new()).unwrap();
        let build = started.elapsed();
        assert!(index.len() >= total);

        let started = Instant::now();
        for i in 0..500 {
            let hits = index.query(&format!("file-{}-{}", i % 40, i % 10), 40);
            assert_eq!(hits.len(), 40);
        }
        let per_query = started.elapsed() / 500;
        eprintln!(
            "indexed {} entries in {build:?}, {per_query:?} per query",
            index.len()
        );
        assert!(
            per_query < Duration::from_millis(50),
            "query too slow: {per_query:?}"
        );
        // Worst case: single-character query matching every entry.
        let started = Instant::now();
        let hits = index.query("e", 40);
        assert_eq!(hits.len(), 40);
        assert!(started.elapsed() < Duration::from_millis(200));
    }

    /// Index of the real home directory defaults (read-only).
    #[test]
    #[ignore = "walks the real user directories"]
    fn live_default_index() {
        let xdg = crate::paths::XdgDirs::from_process_env();
        let cfg = FileIndexConfig::defaults(&xdg.home, &xdg.user_dirs_file(), Arc::new(|_| false));
        let started = Instant::now();
        let index = build_index(&cfg, &CancelToken::new()).unwrap();
        eprintln!(
            "roots {:?}: {} entries in {:?}, truncated={}",
            cfg.roots,
            index.len(),
            started.elapsed(),
            index.truncated
        );
        let q = Instant::now();
        let hits = index.query("a", 40);
        eprintln!("query 'a': {} hits in {:?}", hits.len(), q.elapsed());
    }
}
