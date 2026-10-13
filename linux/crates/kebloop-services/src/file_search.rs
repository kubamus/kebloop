//! File search policy: exclusion rules, ranking and the query snapshot cache. The index itself
//! (directory walking, `.gitignore` handling) lives in `kebloop-platform`.

use std::collections::BTreeSet;
use std::fs;

use crate::personalization::{PersonalizationSettings, contains};
use crate::util::{fold, natural_compare, normalize_path};

/// Maximum results kept per search.
pub const MAX_RESULTS: usize = 40;
/// Results shown in the root (mixed) search.
pub const ROOT_RESULTS: usize = 12;
/// Delay before a search starts, in milliseconds.
pub const DEBOUNCE_MS: u64 = 35;
/// Number of remembered query snapshots.
pub const CACHE_CAPACITY: usize = 12;

const TEMPORARY_SUFFIXES: &[&str] = &[
    "tmp",
    "temp",
    "swp",
    "swo",
    "bak",
    "orig",
    "rej",
    "part",
    "partial",
    "crdownload",
    "download",
];
const PACKAGE_SUFFIXES: &[&str] = &["app", "bundle", "framework", "plugin", "appex"];

/// A file found by the indexer.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FileSearchResult {
    /// File name.
    pub name: String,
    /// Absolute path.
    pub path: String,
}

impl FileSearchResult {
    /// Construct a result.
    pub fn new(name: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            path: path.into(),
        }
    }
}

/// Exclusion policy (everything that is not decided by `.gitignore`).
#[derive(Debug, Clone)]
pub struct FileSearchRules {
    temporary_roots: Vec<String>,
    library_roots: Vec<String>,
    package_suffixes: Vec<String>,
}

impl FileSearchRules {
    /// Rules for a home directory and the system temporary directory. Temporary roots are `/tmp`,
    /// `/var/tmp`, `/dev/shm` and `temp_dir`; the `~/Library` equivalents are `~/.cache`,
    /// `~/.local/share/Trash` and `~/snap`.
    pub fn new(home: &str, temp_dir: &str) -> Self {
        let home = home.trim_end_matches('/');
        let mut temporary_roots: Vec<String> = ["/tmp", "/var/tmp", "/dev/shm"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        if !temp_dir.is_empty() {
            temporary_roots.push(normalize_path(temp_dir));
        }
        Self {
            temporary_roots,
            library_roots: [".cache", ".local/share/Trash", "snap"]
                .iter()
                .map(|s| format!("{home}/{s}"))
                .collect(),
            package_suffixes: PACKAGE_SUFFIXES.iter().map(|s| (*s).to_string()).collect(),
        }
    }

    /// Replace the temporary roots (tests use this to keep fixtures searchable).
    pub fn with_temporary_roots(mut self, roots: Vec<String>) -> Self {
        self.temporary_roots = roots.iter().map(|r| normalize_path(r)).collect();
        self
    }

    /// Replace the suffixes of directories treated as opaque packages.
    pub fn with_package_suffixes(mut self, suffixes: Vec<String>) -> Self {
        self.package_suffixes = suffixes;
        self
    }

    /// Whether the path is a temporary, backup, swap or partial-download file or lives in a
    /// temporary directory. Only whole directory names (`tmp`, `temp`) qualify.
    pub fn is_temporary(&self, path: &str) -> bool {
        let normalized = normalize_path(path);
        if self.temporary_roots.iter().any(|root| {
            normalized == *root
                || normalized.starts_with(&format!("{}/", root.trim_end_matches('/')))
        }) {
            return true;
        }
        let components: Vec<&str> = normalized.split('/').filter(|c| !c.is_empty()).collect();
        for (index, component) in components.iter().enumerate() {
            let lowercase = component.to_lowercase();
            if index + 1 < components.len() && (lowercase == "tmp" || lowercase == "temp") {
                return true;
            }
            if lowercase.starts_with("~$") || lowercase.ends_with('~') {
                return true;
            }
            if let Some(suffix) = extension(&lowercase)
                && TEMPORARY_SUFFIXES.contains(&suffix)
            {
                return true;
            }
        }
        false
    }

    /// Visible files are not temporary, not under the cache/trash roots and have no hidden,
    /// `node_modules` or package component.
    pub fn is_visible_file(&self, path: &str) -> bool {
        if self.is_temporary(path) {
            return false;
        }
        let normalized = normalize_path(path);
        if self
            .library_roots
            .iter()
            .any(|root| normalized == *root || normalized.starts_with(&format!("{root}/")))
        {
            return false;
        }
        !normalized
            .split('/')
            .filter(|c| !c.is_empty())
            .any(|component| {
                if component.starts_with('.') || component == "node_modules" {
                    return true;
                }
                component.rsplit_once('.').is_some_and(|(_, suffix)| {
                    self.package_suffixes
                        .iter()
                        .any(|s| s.eq_ignore_ascii_case(suffix))
                })
            })
    }

    /// Drop temporary files and directories named `tmp`/`temp`. Stops early (returning nothing)
    /// when `should_cancel` turns true.
    pub fn filter(
        &self,
        files: Vec<FileSearchResult>,
        should_cancel: &dyn Fn() -> bool,
    ) -> Vec<FileSearchResult> {
        if should_cancel() {
            return Vec::new();
        }
        let mut kept = Vec::with_capacity(files.len());
        for file in files {
            if should_cancel() {
                return Vec::new();
            }
            if self.is_temporary(&file.path) {
                continue;
            }
            let last = file.path.rsplit('/').next().unwrap_or("").to_lowercase();
            if (last == "tmp" || last == "temp")
                && fs::metadata(&file.path).is_ok_and(|m| m.is_dir())
            {
                continue;
            }
            kept.push(file);
        }
        kept
    }
}

/// Extension of a path component the way `NSString.pathExtension` treats hidden names: leading
/// dots are not an extension separator.
fn extension(component: &str) -> Option<&str> {
    let stripped = component.trim_start_matches('.');
    stripped
        .rsplit_once('.')
        .map(|(_, ext)| ext)
        .filter(|e| !e.is_empty())
}

/// Whether the personalization settings allow this file (inside a folder, outside exclusions).
pub fn allowed_by_settings(
    file: &FileSearchResult,
    settings: &PersonalizationSettings,
    home: &str,
) -> bool {
    settings.files_enabled
        && settings
            .folders
            .iter()
            .any(|f| contains(&file.path, f, home))
        && !settings
            .exclusions
            .iter()
            .any(|f| contains(&file.path, f, home))
}

/// Ranks results: exact name (or stem) > prefix > substring, with a boost for user folders.
#[derive(Debug, Clone)]
pub struct FileRanker {
    priority_dirs: Vec<String>,
}

impl FileRanker {
    /// Boost `Documents`, `Desktop`, `Downloads` and `Public` below `home`.
    pub fn new(home: &str) -> Self {
        let home = home.trim_end_matches('/');
        Self::with_priority_dirs(
            ["Documents", "Desktop", "Downloads", "Public"]
                .iter()
                .map(|d| format!("{home}/{d}"))
                .collect(),
        )
    }

    /// Boost specific directories (e.g. resolved XDG user directories).
    pub fn with_priority_dirs(dirs: Vec<String>) -> Self {
        Self {
            priority_dirs: dirs
                .into_iter()
                .map(|d| format!("{}/", d.trim_end_matches('/')))
                .collect(),
        }
    }

    fn score(&self, file: &FileSearchResult, query: &str) -> u32 {
        let name = fold(&file.name);
        let stem = name
            .rsplit_once('.')
            .filter(|(s, _)| !s.is_empty())
            .map_or(name.as_str(), |(s, _)| s);
        let relevance = if name == query || stem == query {
            3
        } else if name.starts_with(query) {
            2
        } else {
            1
        };
        let boost = u32::from(
            self.priority_dirs
                .iter()
                .any(|d| file.path.starts_with(d.as_str())),
        );
        relevance * 10 + boost
    }

    /// Sort by score (descending), then natural name order, then path.
    pub fn ranked(&self, files: &[FileSearchResult], text: &str) -> Vec<FileSearchResult> {
        let query = fold(text);
        let mut scored: Vec<(u32, &FileSearchResult)> =
            files.iter().map(|f| (self.score(f, &query), f)).collect();
        scored.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| natural_compare(&a.1.name, &b.1.name))
                .then_with(|| a.1.path.cmp(&b.1.path))
        });
        scored.into_iter().map(|(_, f)| f.clone()).collect()
    }

    /// Combine earlier and new results (deduplicated by path), rank and cut to `limit`.
    pub fn combine(
        &self,
        accumulated: &[FileSearchResult],
        visible: &[FileSearchResult],
        text: &str,
        limit: usize,
    ) -> Vec<FileSearchResult> {
        let mut seen = BTreeSet::new();
        let combined: Vec<FileSearchResult> = accumulated
            .iter()
            .chain(visible)
            .filter(|f| seen.insert(f.path.clone()))
            .cloned()
            .collect();
        let mut ranked = self.ranked(&combined, text);
        ranked.truncate(limit);
        ranked
    }
}

/// Small snapshots of recent queries so narrowing or backspacing is instant. They are
/// provisional: the real search still runs and replaces them.
#[derive(Debug, Clone, Default)]
pub struct FileSearchCache {
    entries: Vec<(String, Vec<FileSearchResult>)>,
}

impl FileSearchCache {
    /// Empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Remember the first 40 results of a query (newest snapshot first, 12 snapshots).
    pub fn store(&mut self, results: &[FileSearchResult], text: &str) {
        let key = fold(text);
        self.entries.retain(|(k, _)| *k != key);
        self.entries.insert(
            0,
            (key, results.iter().take(MAX_RESULTS).cloned().collect()),
        );
        self.entries.truncate(CACHE_CAPACITY);
    }

    /// Cached results matching `text`: the exact snapshot, else the newest snapshot of a shorter
    /// query contained in `text`, filtered to names containing `text` and ranked.
    pub fn matching(&self, text: &str, ranker: &FileRanker) -> Vec<FileSearchResult> {
        let key = fold(text);
        if key.is_empty() {
            return Vec::new();
        }
        let entry = self
            .entries
            .iter()
            .find(|(k, _)| *k == key)
            .or_else(|| self.entries.iter().find(|(k, _)| key.contains(k.as_str())));
        let Some((_, results)) = entry else {
            return Vec::new();
        };
        let filtered: Vec<FileSearchResult> = results
            .iter()
            .filter(|f| fold(&f.name).contains(&key))
            .cloned()
            .collect();
        ranker.ranked(&filtered, text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    const HOME: &str = "/home/me";

    fn rules() -> FileSearchRules {
        FileSearchRules::new(HOME, "/run/user/1000/kebloop-temp")
    }

    fn file(name: &str) -> FileSearchResult {
        FileSearchResult::new(name, format!("/home/test/Documents/{name}"))
    }

    #[test]
    fn temporary_files_and_directories() {
        let rules = rules();
        for path in [
            "/tmp/report.txt",
            "/var/tmp/report.txt",
            "/dev/shm/report.txt",
            "/run/user/1000/kebloop-temp/report.txt",
            "/home/person/project/TEMP/report.txt",
            "/home/person/tmp/report.txt",
            "/home/person/report.TMP",
            "/home/person/report.temp",
            "/home/person/report.swp",
            "/home/person/report.swo",
            "/home/person/report.bak",
            "/home/person/report.orig",
            "/home/person/report.rej",
            "/home/person/report.txt~",
            "/home/person/~$report.docx",
            "/home/person/video.part",
            "/home/person/video.partial",
            "/home/person/video.crdownload",
            "/home/person/video.download/content",
        ] {
            assert!(rules.is_temporary(path), "{path}");
        }
        for path in [
            "/home/person/template.txt",
            "/home/person/tempest/readme",
            "/home/person/Backup/report.txt",
            "/home/person/temp",
            "/tmpfoo/report.txt",
        ] {
            assert!(!rules.is_temporary(path), "{path}");
        }
    }

    #[test]
    fn temporary_classification_normalizes_unusual_paths() {
        let rules = rules();
        for path in [
            "/home/person/ordinary/../TEMP/report.txt",
            "/home/person//temp/report.txt",
            "/home/person/./report.TMP",
            "/home/person/report.TmP",
        ] {
            assert!(rules.is_temporary(path), "{path}");
        }
        for path in [
            "/home/person/temp/../report.txt",
            "/home/person/report.tmp-notes.txt",
            "/home/person/folder/../template.txt",
            "/home/person/.tmp",
            "/home/person/...tmp",
        ] {
            assert!(!rules.is_temporary(path), "{path}");
        }
    }

    #[test]
    fn hidden_files_and_packages_are_excluded() {
        let rules = rules();
        for path in [
            "/home/me/.secret",
            "/home/me/.config/file",
            "/opt/Ghostty.app",
            "/opt/Ghostty.app/Contents/Info.plist",
            "/home/me/Tool.APP/file",
            "/home/me/Plug.plugin/Info.plist",
            "/home/me/Test.framework/Header.h",
        ] {
            assert!(!rules.is_visible_file(path), "{path}");
        }
        assert!(rules.is_visible_file("/home/me/Documents/application.txt"));
        assert!(rules.is_visible_file("/home/me/Documents/project.app-notes.txt"));
    }

    #[test]
    fn cache_trash_and_dependency_contents_are_excluded() {
        let rules = rules();
        assert!(!rules.is_visible_file("/home/me/.cache"));
        assert!(!rules.is_visible_file("/home/me/.cache/thumbnails/terminal.txt"));
        assert!(!rules.is_visible_file("/home/me/.local/share/Trash/files/terminal.txt"));
        assert!(!rules.is_visible_file("/home/me/snap/firefox/common/terminal.txt"));
        assert!(!rules.is_visible_file("/home/me/project/node_modules/terminal/index.js"));
        assert!(!rules.is_visible_file("/home/me/project/node_modules"));
        assert!(rules.is_visible_file("/home/me/Documents/Cache-notes.txt"));
        assert!(rules.is_visible_file("/home/me/snapshot/file.txt"));
    }

    #[test]
    fn named_temporary_directories_and_cancellation() {
        let root = tempfile::Builder::new()
            .prefix("kebloop-test-")
            .tempdir_in(env!("CARGO_MANIFEST_DIR"))
            .unwrap();
        fs::create_dir(root.path().join("temp")).unwrap();
        fs::write(root.path().join("tmp"), "ordinary file").unwrap();
        let rules = rules().with_temporary_roots(vec![]);
        let path = |n: &str| root.path().join(n).to_string_lossy().into_owned();
        let files = vec![
            FileSearchResult::new("temp", path("temp")),
            FileSearchResult::new("tmp", path("tmp")),
        ];
        let kept: Vec<_> = rules
            .filter(files.clone(), &|| false)
            .into_iter()
            .map(|f| f.name)
            .collect();
        assert_eq!(kept, ["tmp"]);
        assert!(rules.filter(files, &|| true).is_empty());
        let _ = fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700));
    }

    #[test]
    fn exact_and_prefix_names_rank_before_substring_with_user_folder_boost() {
        let ranker = FileRanker::new(HOME);
        let exact = FileSearchResult::new("Terminal.pdf", "/home/me/Downloads/Terminal.pdf");
        let exact_other = FileSearchResult::new("Terminal.txt", "/home/me/project/Terminal.txt");
        let prefix = FileSearchResult::new(
            "terminal-KebloopVerification.txt",
            "/home/me/Desktop/terminal-KebloopVerification.txt",
        );
        let substring =
            FileSearchResult::new("a-terminal.txt", "/home/me/Documents/a-terminal.txt");
        let files = vec![
            substring.clone(),
            prefix.clone(),
            exact_other.clone(),
            exact.clone(),
        ];
        let expected = vec![
            exact.clone(),
            exact_other.clone(),
            prefix.clone(),
            substring.clone(),
        ];
        assert_eq!(ranker.ranked(&files, "terminal"), expected);
        let reversed: Vec<_> = files.into_iter().rev().collect();
        assert_eq!(ranker.ranked(&reversed, "terminal"), expected);
    }

    #[test]
    fn combine_deduplicates_ranks_and_limits() {
        let ranker = FileRanker::new(HOME);
        let a = FileSearchResult::new("a-x.txt", "/p/a-x.txt");
        let b = FileSearchResult::new("x.txt", "/p/x.txt");
        let combined = ranker.combine(std::slice::from_ref(&a), &[a.clone(), b.clone()], "x", 2);
        assert_eq!(combined, [b, a]);
        assert_eq!(ranker.combine(&[], &[], "x", 5), []);
    }

    #[test]
    fn settings_gate_folders_and_exclusions() {
        let settings = PersonalizationSettings {
            folders: vec!["~/Documents".into()],
            exclusions: vec!["~/Documents/Private".into()],
            ..PersonalizationSettings::default()
        };
        assert!(allowed_by_settings(
            &FileSearchResult::new("a", "/home/me/Documents/a"),
            &settings,
            HOME
        ));
        assert!(!allowed_by_settings(
            &FileSearchResult::new("a", "/home/me/Documents/Private/a"),
            &settings,
            HOME
        ));
        assert!(!allowed_by_settings(
            &FileSearchResult::new("a", "/home/me/Other/a"),
            &settings,
            HOME
        ));
    }

    #[test]
    fn cached_narrowing_only_returns_matching_names() {
        let ranker = FileRanker::new(HOME);
        let mut cache = FileSearchCache::new();
        cache.store(
            &[file("Résumé.pdf"), file("Research.txt"), file("report.txt")],
            "re",
        );
        let names = |v: Vec<FileSearchResult>| v.into_iter().map(|f| f.name).collect::<Vec<_>>();
        assert_eq!(names(cache.matching("resume", &ranker)), ["Résumé.pdf"]);
        assert_eq!(names(cache.matching("rese", &ranker)), ["Research.txt"]);
        assert!(cache.matching("other", &ranker).is_empty());
        assert!(cache.matching("", &ranker).is_empty());
    }

    #[test]
    fn cache_is_bounded_and_updates_existing_queries() {
        let ranker = FileRanker::new(HOME);
        let mut cache = FileSearchCache::new();
        let many: Vec<_> = (0..60).map(|i| file(&format!("alpha-{i}"))).collect();
        cache.store(&many, "alpha");
        assert_eq!(cache.matching("alpha", &ranker).len(), 40);
        cache.store(&[file("alpha-new")], "alpha");
        let names: Vec<_> = cache
            .matching("alpha", &ranker)
            .into_iter()
            .map(|f| f.name)
            .collect();
        assert_eq!(names, ["alpha-new"]);
        for i in 0..12 {
            cache.store(&[file(&format!("query{i}"))], &format!("query{i}"));
        }
        assert!(cache.matching("alpha", &ranker).is_empty());
    }
}
