//! XDG desktop-entry application index.
//!
//! Scans `$XDG_DATA_HOME/applications` and every `$XDG_DATA_DIRS/*/applications`
//! (recursively, with `foo/bar.desktop` becoming id `foo-bar.desktop`), applies
//! first-wins precedence per desktop-file id and the visibility rules of the
//! Desktop Entry Specification (`Type`, `NoDisplay`, `Hidden`, `OnlyShowIn`,
//! `NotShowIn`, `TryExec`). Launching is done by the GTK crate through
//! `gio::DesktopAppInfo` using [`DesktopApp::id`].

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};

use crate::paths::{Env, XdgDirs};
use crate::text::fold;

const MAX_DEPTH: usize = 8;
const MAX_DESKTOP_FILE_BYTES: u64 = 1024 * 1024;

/// Where an application entry came from (used for duplicate-name subtitles).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AppOrigin {
    /// `$XDG_DATA_HOME/applications`
    User,
    #[default]
    System,
    Flatpak,
    Snap,
    Nix,
}

impl AppOrigin {
    /// English label; the GUI may localise by matching on the enum instead.
    pub fn label(self) -> &'static str {
        match self {
            Self::User => "User",
            Self::System => "System",
            Self::Flatpak => "Flatpak",
            Self::Snap => "Snap",
            Self::Nix => "Nix",
        }
    }
}

/// A `[Desktop Action ...]` group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesktopAction {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub exec: String,
}

/// An application discovered from a desktop entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesktopApp {
    /// Desktop-file id, e.g. `org.gnome.Nautilus.desktop` or `kde4-konsole.desktop`.
    pub id: String,
    /// Absolute path of the winning `.desktop` file.
    pub path: PathBuf,
    /// Localized `Name`.
    pub name: String,
    #[serde(default)]
    pub generic_name: Option<String>,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub comment: Option<String>,
    /// Icon name or absolute path, unresolved.
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub terminal: bool,
    #[serde(default)]
    pub exec: String,
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub startup_wm_class: Option<String>,
    #[serde(default)]
    pub actions: Vec<DesktopAction>,
    #[serde(default)]
    pub origin: AppOrigin,
    /// Set only when another application shares the same display name.
    #[serde(default)]
    pub subtitle: Option<String>,
}

/// Ordered locale candidates for localized keys, most specific first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Locales {
    candidates: Vec<String>,
}

impl Locales {
    /// Builds candidates from locale names such as `pl_PL.UTF-8` or `sr_RS@latin`
    /// (priority order). Per the spec each yields `lang_COUNTRY@MOD`,
    /// `lang_COUNTRY`, `lang@MOD`, `lang`.
    pub fn new<S: AsRef<str>>(locales: &[S]) -> Self {
        let mut candidates: Vec<String> = Vec::new();
        for locale in locales {
            let locale = locale.as_ref();
            if locale.is_empty() || locale == "C" || locale == "POSIX" {
                continue;
            }
            let (rest, modifier) = match locale.split_once('@') {
                Some((r, m)) => (r, Some(m)),
                None => (locale, None),
            };
            let rest = rest.split_once('.').map_or(rest, |(r, _)| r);
            let (lang, country) = match rest.split_once('_') {
                Some((l, c)) => (l, Some(c)),
                None => (rest, None),
            };
            if lang.is_empty() {
                continue;
            }
            let mut push = |s: String| {
                if !candidates.contains(&s) {
                    candidates.push(s);
                }
            };
            if let (Some(c), Some(m)) = (country, modifier) {
                push(format!("{lang}_{c}@{m}"));
            }
            if let Some(c) = country {
                push(format!("{lang}_{c}"));
            }
            if let Some(m) = modifier {
                push(format!("{lang}@{m}"));
            }
            push(lang.to_owned());
        }
        Self { candidates }
    }

    /// Derives the locale from `LANGUAGE`, `LC_ALL`, `LC_MESSAGES`, `LANG`
    /// using gettext priority rules (`LANGUAGE` is ignored for the C locale).
    pub fn from_env(env: &Env) -> Self {
        let base = env
            .get("LC_ALL")
            .or_else(|| env.get("LC_MESSAGES"))
            .or_else(|| env.get("LANG"));
        let Some(base) = base else {
            return Self::default();
        };
        if base == "C" || base == "POSIX" {
            return Self::default();
        }
        let mut list: Vec<&str> = env
            .get("LANGUAGE")
            .map(|l| l.split(':').filter(|s| !s.is_empty()).collect())
            .unwrap_or_default();
        list.push(base);
        Self::new(&list)
    }

    /// The candidate keys in priority order.
    pub fn candidates(&self) -> &[String] {
        &self.candidates
    }
}

/// Everything the scanner needs; fully injectable for tests.
#[derive(Debug, Clone)]
pub struct ScanConfig {
    /// `applications` directories, highest precedence first.
    pub app_dirs: Vec<PathBuf>,
    /// Components of `XDG_CURRENT_DESKTOP`.
    pub current_desktop: Vec<String>,
    pub locales: Locales,
    /// `PATH` directories for `TryExec`.
    pub path_dirs: Vec<PathBuf>,
    /// `$XDG_DATA_HOME`, used to classify [`AppOrigin::User`].
    pub data_home: PathBuf,
}

/// `applications` search directories: `$XDG_DATA_HOME` first, then `$XDG_DATA_DIRS`.
pub fn application_dirs(xdg: &XdgDirs) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for base in std::iter::once(&xdg.data_home).chain(xdg.data_dirs.iter()) {
        let dir = base.join("applications");
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    dirs
}

impl ScanConfig {
    pub fn from_env(env: &Env) -> Self {
        let xdg = XdgDirs::from_env(env);
        Self {
            app_dirs: application_dirs(&xdg),
            current_desktop: env
                .get("XDG_CURRENT_DESKTOP")
                .map(|v| {
                    v.split(':')
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
            locales: Locales::from_env(env),
            path_dirs: env.path_dirs(),
            data_home: xdg.data_home,
        }
    }

    /// Configuration for the running process.
    pub fn from_process_env() -> Self {
        Self::from_env(&Env::from_process())
    }
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
struct Group {
    name: String,
    entries: HashMap<String, String>,
}

fn parse_groups(content: &str) -> Vec<Group> {
    let mut groups: Vec<Group> = Vec::new();
    for raw in content.trim_start_matches('\u{feff}').lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            groups.push(Group {
                name: name.to_owned(),
                entries: HashMap::new(),
            });
            continue;
        }
        let Some(group) = groups.last_mut() else {
            continue;
        };
        if let Some((key, value)) = line.split_once('=') {
            group
                .entries
                .entry(key.trim().to_owned())
                .or_insert_with(|| value.trim_start().to_owned());
        }
    }
    groups
}

fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => out.push(' '),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some(';') => out.push(';'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Splits a `;`-separated list honouring `\;` escapes.
fn parse_list(value: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut current = String::new();
    let mut escaped = false;
    for c in value.chars() {
        if escaped {
            current.push('\\');
            current.push(c);
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == ';' {
            items.push(std::mem::take(&mut current));
        } else {
            current.push(c);
        }
    }
    if escaped {
        current.push('\\');
    }
    items.push(current);
    items
        .into_iter()
        .map(|i| unescape(&i))
        .map(|i| i.trim().to_owned())
        .filter(|i| !i.is_empty())
        .collect()
}

impl Group {
    fn raw(&self, key: &str) -> Option<&str> {
        self.entries.get(key).map(String::as_str)
    }

    fn string(&self, key: &str) -> Option<String> {
        self.raw(key)
            .map(|v| unescape(v).trim().to_owned())
            .filter(|v| !v.is_empty())
    }

    fn localized_raw(&self, key: &str, locales: &Locales) -> Option<&str> {
        for candidate in locales.candidates() {
            if let Some(v) = self.entries.get(&format!("{key}[{candidate}]"))
                && !v.trim().is_empty()
            {
                return Some(v);
            }
        }
        self.raw(key).filter(|v| !v.trim().is_empty())
    }

    fn localized(&self, key: &str, locales: &Locales) -> Option<String> {
        self.localized_raw(key, locales)
            .map(|v| unescape(v).trim().to_owned())
            .filter(|v| !v.is_empty())
    }

    fn boolean(&self, key: &str) -> bool {
        self.raw(key).is_some_and(|v| v.trim() == "true")
    }

    fn list(&self, key: &str) -> Vec<String> {
        self.raw(key).map(parse_list).unwrap_or_default()
    }
}

fn is_executable_file(path: &Path) -> bool {
    fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// Resolves a `TryExec` value: absolute path or lookup in `path_dirs`.
pub fn try_exec_resolves(try_exec: &str, path_dirs: &[PathBuf]) -> bool {
    if try_exec.contains('/') {
        return is_executable_file(Path::new(try_exec));
    }
    path_dirs
        .iter()
        .any(|d| is_executable_file(&d.join(try_exec)))
}

/// Why an entry was not listed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skip {
    NotApplication,
    NoDisplay,
    Hidden,
    NotShownInDesktop,
    TryExecFailed,
    Invalid,
}

/// Parses one desktop entry. `Err` carries the reason it is not listed.
pub fn parse_desktop_entry(
    id: &str,
    path: &Path,
    content: &str,
    cfg: &ScanConfig,
) -> Result<DesktopApp, Skip> {
    let groups = parse_groups(content);
    let entry = groups
        .iter()
        .find(|g| g.name == "Desktop Entry")
        .ok_or(Skip::Invalid)?;
    if entry.raw("Type").map(str::trim) != Some("Application") {
        return Err(Skip::NotApplication);
    }
    if entry.boolean("Hidden") {
        return Err(Skip::Hidden);
    }
    if entry.boolean("NoDisplay") {
        return Err(Skip::NoDisplay);
    }
    if !cfg.current_desktop.is_empty() {
        let in_desktop = |list: &[String]| {
            list.iter().any(|d| {
                cfg.current_desktop
                    .iter()
                    .any(|c| c.eq_ignore_ascii_case(d))
            })
        };
        let only = entry.list("OnlyShowIn");
        if !only.is_empty() && !in_desktop(&only) {
            return Err(Skip::NotShownInDesktop);
        }
        if in_desktop(&entry.list("NotShowIn")) {
            return Err(Skip::NotShownInDesktop);
        }
    }
    if let Some(try_exec) = entry.string("TryExec")
        && !try_exec_resolves(&try_exec, &cfg.path_dirs)
    {
        return Err(Skip::TryExecFailed);
    }
    let name = entry.localized("Name", &cfg.locales).ok_or(Skip::Invalid)?;
    let exec = entry
        .raw("Exec")
        .map(|e| e.trim().to_owned())
        .unwrap_or_default();
    if exec.is_empty() && !entry.boolean("DBusActivatable") {
        return Err(Skip::Invalid);
    }

    let keywords = entry
        .localized_raw("Keywords", &cfg.locales)
        .map(parse_list)
        .unwrap_or_default();

    let mut actions = Vec::new();
    for action_id in entry.list("Actions") {
        let wanted = format!("Desktop Action {action_id}");
        let Some(group) = groups.iter().find(|g| g.name == wanted) else {
            continue;
        };
        let Some(action_name) = group.localized("Name", &cfg.locales) else {
            continue;
        };
        actions.push(DesktopAction {
            id: action_id,
            name: action_name,
            icon: group.string("Icon"),
            exec: group
                .raw("Exec")
                .map(|e| e.trim().to_owned())
                .unwrap_or_default(),
        });
    }

    Ok(DesktopApp {
        id: id.to_owned(),
        path: path.to_path_buf(),
        name,
        generic_name: entry.localized("GenericName", &cfg.locales),
        keywords,
        comment: entry.localized("Comment", &cfg.locales),
        icon: entry.string("Icon"),
        terminal: entry.boolean("Terminal"),
        exec,
        categories: entry.list("Categories"),
        startup_wm_class: entry.string("StartupWMClass"),
        actions,
        origin: classify_origin(path, &cfg.data_home),
        subtitle: None,
    })
}

/// Classifies where a desktop file lives.
pub fn classify_origin(path: &Path, data_home: &Path) -> AppOrigin {
    let s = path.to_string_lossy();
    if s.contains("/flatpak/exports/") {
        AppOrigin::Flatpak
    } else if s.starts_with("/snap/") || s.contains("/snapd/desktop/") {
        AppOrigin::Snap
    } else if s.starts_with("/nix/")
        || s.contains("/.nix-profile/")
        || s.contains("/etc/profiles/per-user/")
    {
        AppOrigin::Nix
    } else if path.starts_with(data_home) {
        AppOrigin::User
    } else {
        AppOrigin::System
    }
}

// ---------------------------------------------------------------------------
// Scanning
// ---------------------------------------------------------------------------

fn collect_desktop_files(root: &Path) -> Vec<(String, PathBuf)> {
    fn walk(dir: &Path, prefix: &str, depth: usize, out: &mut Vec<(String, PathBuf)>) {
        if depth > MAX_DEPTH {
            return;
        }
        let Ok(read) = fs::read_dir(dir) else {
            return;
        };
        let mut names: Vec<(String, PathBuf)> = read
            .filter_map(Result::ok)
            .filter_map(|e| Some((e.file_name().into_string().ok()?, e.path())))
            .collect();
        names.sort();
        for (name, path) in names {
            // `metadata` follows symlinks (Flatpak/Nix exports are symlinks).
            let Ok(meta) = fs::metadata(&path) else {
                continue;
            };
            if meta.is_dir() {
                walk(&path, &format!("{prefix}{name}-"), depth + 1, out);
            } else if meta.is_file() && name.ends_with(".desktop") {
                out.push((format!("{prefix}{name}"), path));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, "", 0, &mut out);
    out
}

/// Compares names ignoring case and diacritics with digit runs ordered numerically.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    natural_cmp_folded(&fold(a), &fold(b))
}

/// [`natural_cmp`] for strings that are already folded with [`fold`].
pub fn natural_cmp_folded(a: &str, b: &str) -> Ordering {
    let (mut ai, mut bi) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (ai.peek().copied(), bi.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let take = |it: &mut std::iter::Peekable<std::str::Chars<'_>>| {
                    let mut s = String::new();
                    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
                        s.push(c);
                        it.next();
                    }
                    s.trim_start_matches('0').to_owned()
                };
                let (na, nb) = (take(&mut ai), take(&mut bi));
                let ord = na.len().cmp(&nb.len()).then_with(|| na.cmp(&nb));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(x), Some(y)) => {
                if x != y {
                    return x.cmp(&y);
                }
                ai.next();
                bi.next();
            }
        }
    }
}

fn sort_apps(apps: &mut [DesktopApp]) {
    apps.sort_by(|a, b| {
        natural_cmp(&a.name, &b.name)
            .then_with(|| a.id.cmp(&b.id))
            .then_with(|| a.path.cmp(&b.path))
    });
}

/// Adds disambiguating subtitles to applications that share a display name.
pub fn disambiguate(apps: &mut [DesktopApp]) {
    let mut groups: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, app) in apps.iter().enumerate() {
        groups.entry(fold(&app.name)).or_default().push(i);
    }
    for indices in groups.values().filter(|g| g.len() > 1) {
        let mut label_counts: HashMap<AppOrigin, usize> = HashMap::new();
        for &i in indices {
            *label_counts.entry(apps[i].origin).or_default() += 1;
        }
        for &i in indices {
            let origin = apps[i].origin;
            apps[i].subtitle = Some(if label_counts[&origin] > 1 {
                format!(
                    "{} ({})",
                    origin.label(),
                    apps[i].id.trim_end_matches(".desktop")
                )
            } else {
                origin.label().to_owned()
            });
        }
    }
}

/// Scans all application directories and returns the sorted application list.
pub fn scan_applications(cfg: &ScanConfig) -> Vec<DesktopApp> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut apps = Vec::new();
    for dir in &cfg.app_dirs {
        for (id, path) in collect_desktop_files(dir) {
            // Precedence: the first file with an id wins, even when it is hidden.
            if !seen.insert(id.clone()) {
                continue;
            }
            if path.to_str().is_none() {
                continue;
            }
            let Ok(meta) = fs::metadata(&path) else {
                continue;
            };
            if meta.len() > MAX_DESKTOP_FILE_BYTES {
                continue;
            }
            let Ok(bytes) = fs::read(&path) else {
                continue;
            };
            let content = String::from_utf8_lossy(&bytes);
            if let Ok(app) = parse_desktop_entry(&id, &path, &content, cfg) {
                apps.push(app);
            }
        }
    }
    sort_apps(&mut apps);
    disambiguate(&mut apps);
    apps
}

// ---------------------------------------------------------------------------
// Cache
// ---------------------------------------------------------------------------

/// Loads the JSON cache; returns an empty list when missing or corrupt.
pub fn load_cache(path: &Path) -> Vec<DesktopApp> {
    let Ok(data) = fs::read(path) else {
        return Vec::new();
    };
    let mut apps: Vec<DesktopApp> = serde_json::from_slice(&data).unwrap_or_default();
    sort_apps(&mut apps);
    apps
}

/// Atomically writes the JSON cache (temp file + rename).
pub fn save_cache(path: &Path, apps: &[DesktopApp]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        crate::paths::ensure_private_dir(parent)?;
    }
    let data = serde_json::to_vec(apps).map_err(io::Error::other)?;
    let tmp = path.with_extension(format!("json.tmp{}", std::process::id()));
    fs::write(&tmp, data)?;
    fs::rename(&tmp, path).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}

/// Whether a periodic rescan is due (mirrors the macOS 5-minute refresh).
pub fn is_refresh_due(last_completed: Option<Instant>, now: Instant, interval: Duration) -> bool {
    last_completed.is_none_or(|t| now.saturating_duration_since(t) >= interval)
}

/// Default periodic rescan interval.
pub const REFRESH_INTERVAL: Duration = Duration::from_secs(5 * 60);

// ---------------------------------------------------------------------------
// Watcher
// ---------------------------------------------------------------------------

/// Signal that the set of desktop entries may have changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppsChanged;

/// inotify watcher over the application directories. Dropping it stops the
/// background threads.
pub struct AppWatcher {
    _watcher: RecommendedWatcher,
}

impl AppWatcher {
    /// Watches every existing directory in `dirs` recursively. Changes are
    /// debounced: one [`AppsChanged`] is sent after `debounce` of quiet.
    /// Directories that do not exist yet are not watched (the periodic rescan
    /// covers them).
    pub fn start(
        dirs: &[PathBuf],
        debounce: Duration,
    ) -> Result<(Self, async_channel::Receiver<AppsChanged>), notify::Error> {
        let (raw_tx, raw_rx) = mpsc::channel::<()>();
        let mut watcher =
            notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                let Ok(event) = res else {
                    return;
                };
                if matches!(
                    event.kind,
                    EventKind::Access(_)
                        | EventKind::Modify(notify::event::ModifyKind::Metadata(_))
                ) {
                    return;
                }
                let relevant = event.paths.iter().any(|p| match p.extension() {
                    Some(ext) => ext == "desktop",
                    None => true,
                });
                if relevant {
                    let _ = raw_tx.send(());
                }
            })?;
        let mut watched = 0usize;
        for dir in dirs.iter().filter(|d| d.is_dir()) {
            if watcher.watch(dir, RecursiveMode::Recursive).is_ok() {
                watched += 1;
            }
        }
        if watched == 0 && !dirs.is_empty() {
            return Err(notify::Error::generic(
                "no application directory could be watched",
            ));
        }

        let (tx, rx) = async_channel::bounded(1);
        std::thread::Builder::new()
            .name("kebloop-apps-watch".into())
            .spawn(move || {
                // Ends when the watcher (and with it the sender) is dropped.
                while raw_rx.recv().is_ok() {
                    let started = Instant::now();
                    loop {
                        match raw_rx.recv_timeout(debounce) {
                            Ok(()) if started.elapsed() < debounce * 20 => {}
                            Ok(()) | Err(mpsc::RecvTimeoutError::Timeout) => break,
                            Err(mpsc::RecvTimeoutError::Disconnected) => return,
                        }
                    }
                    if tx.is_closed() {
                        return;
                    }
                    let _ = tx.try_send(AppsChanged);
                }
            })
            .map_err(notify::Error::io)?;
        Ok((Self { _watcher: watcher }, rx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use tempfile::TempDir;

    fn write(dir: &Path, rel: &str, content: &str) -> PathBuf {
        let p = dir.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, content).unwrap();
        p
    }

    fn entry(name: &str, extra: &str) -> String {
        format!("[Desktop Entry]\nType=Application\nName={name}\nExec=true\n{extra}")
    }

    fn cfg(dirs: Vec<PathBuf>) -> ScanConfig {
        ScanConfig {
            app_dirs: dirs,
            current_desktop: vec!["niri".into()],
            locales: Locales::new(&["pl_PL.UTF-8"]),
            path_dirs: vec![],
            data_home: PathBuf::from("/nonexistent-home/.local/share"),
        }
    }

    #[test]
    fn locale_candidates_follow_spec() {
        let l = Locales::new(&["sr_RS.UTF-8@latin"]);
        assert_eq!(l.candidates(), ["sr_RS@latin", "sr_RS", "sr@latin", "sr"]);
        assert!(Locales::new(&["C"]).candidates().is_empty());
        let env = Env::from_pairs([("LANGUAGE", "de:pl"), ("LANG", "en_US.UTF-8")]);
        assert_eq!(
            Locales::from_env(&env).candidates(),
            ["de", "pl", "en_US", "en"]
        );
        let env = Env::from_pairs([("LANGUAGE", "de"), ("LANG", "C")]);
        assert!(Locales::from_env(&env).candidates().is_empty());
    }

    #[test]
    fn parses_localized_fields_and_actions() {
        let tmp = TempDir::new().unwrap();
        let content = "# comment\n[Desktop Entry]\nType=Application\nName=Files\nName[pl]=Pliki\nGenericName=File Manager\nGenericName[pl]=Menedżer plików\nComment=Browse\nKeywords=folder;manager;a\\;b;\nKeywords[pl]=katalog;menedżer;\nExec=files %U\nIcon=system-file-manager\nTerminal=false\nCategories=System;FileManager;\nActions=new;broken;\n\n[Desktop Action new]\nName=New Window\nName[pl]=Nowe okno\nExec=files --new\n\n[Desktop Action broken]\nExec=x\n";
        let p = write(tmp.path(), "files.desktop", content);
        let app = parse_desktop_entry("files.desktop", &p, content, &cfg(vec![])).unwrap();
        assert_eq!(app.name, "Pliki");
        assert_eq!(app.generic_name.as_deref(), Some("Menedżer plików"));
        assert_eq!(app.keywords, ["katalog", "menedżer"]);
        assert_eq!(app.comment.as_deref(), Some("Browse"));
        assert_eq!(app.icon.as_deref(), Some("system-file-manager"));
        assert!(!app.terminal);
        assert_eq!(app.categories, ["System", "FileManager"]);
        assert_eq!(app.actions.len(), 1);
        assert_eq!(app.actions[0].name, "Nowe okno");
        assert_eq!(app.actions[0].exec, "files --new");

        let mut english = cfg(vec![]);
        english.locales = Locales::default();
        let app = parse_desktop_entry("files.desktop", &p, content, &english).unwrap();
        assert_eq!(app.name, "Files");
        assert_eq!(app.keywords, ["folder", "manager", "a;b"]);
    }

    #[test]
    fn visibility_rules() {
        let c = cfg(vec![]);
        let p = Path::new("/x/a.desktop");
        let parse = |content: String| parse_desktop_entry("a.desktop", p, &content, &c);
        assert_eq!(
            parse(entry("A", "NoDisplay=true\n")).unwrap_err(),
            Skip::NoDisplay
        );
        assert_eq!(
            parse(entry("A", "Hidden=true\n")).unwrap_err(),
            Skip::Hidden
        );
        assert_eq!(
            parse(entry("A", "OnlyShowIn=GNOME;KDE;\n")).unwrap_err(),
            Skip::NotShownInDesktop
        );
        assert!(parse(entry("A", "OnlyShowIn=GNOME;niri;\n")).is_ok());
        assert_eq!(
            parse(entry("A", "NotShowIn=niri;\n")).unwrap_err(),
            Skip::NotShownInDesktop
        );
        assert!(parse(entry("A", "NotShowIn=GNOME;\n")).is_ok());
        assert_eq!(
            parse("[Desktop Entry]\nType=Link\nName=A\nURL=x\n".into()).unwrap_err(),
            Skip::NotApplication
        );
        assert_eq!(
            parse("[Desktop Entry]\nName=A\nExec=x\n".into()).unwrap_err(),
            Skip::NotApplication
        );
        assert_eq!(
            parse("[Desktop Entry]\nType=Application\nExec=x\n".into()).unwrap_err(),
            Skip::Invalid
        );
        assert_eq!(
            parse("[Desktop Entry]\nType=Application\nName=A\n".into()).unwrap_err(),
            Skip::Invalid
        );
        // No current desktop: OnlyShowIn cannot be evaluated, so the entry stays.
        let mut none = cfg(vec![]);
        none.current_desktop.clear();
        assert!(
            parse_desktop_entry("a.desktop", p, &entry("A", "OnlyShowIn=GNOME;\n"), &none).is_ok()
        );
    }

    #[test]
    fn try_exec_uses_path() {
        let tmp = TempDir::new().unwrap();
        let bin = write(tmp.path(), "bin/tool", "#!/bin/sh\n");
        fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
        write(tmp.path(), "bin/plain", "data");
        let mut c = cfg(vec![]);
        c.path_dirs = vec![tmp.path().join("bin")];
        let p = Path::new("/x/a.desktop");
        let ok = entry("A", "TryExec=tool\n");
        assert!(parse_desktop_entry("a.desktop", p, &ok, &c).is_ok());
        let abs = entry("A", &format!("TryExec={}\n", bin.display()));
        assert!(parse_desktop_entry("a.desktop", p, &abs, &c).is_ok());
        for bad in [
            "TryExec=missing\n",
            "TryExec=plain\n",
            "TryExec=/nonexistent/bin\n",
        ] {
            assert_eq!(
                parse_desktop_entry("a.desktop", p, &entry("A", bad), &c).unwrap_err(),
                Skip::TryExecFailed
            );
        }
    }

    #[test]
    fn precedence_subdirs_and_hidden_shadowing() {
        let tmp = TempDir::new().unwrap();
        let user = tmp.path().join("user/applications");
        let sys = tmp.path().join("sys/applications");
        write(&user, "alpha.desktop", &entry("Alpha User", ""));
        write(&sys, "alpha.desktop", &entry("Alpha System", ""));
        write(&sys, "beta.desktop", &entry("Beta", ""));
        write(
            &user,
            "beta.desktop",
            &entry("Beta Hidden", "Hidden=true\n"),
        );
        write(&sys, "foo/bar.desktop", &entry("Foo Bar", ""));
        write(&sys, "notes.txt", "ignored");
        let apps = scan_applications(&cfg(vec![user, sys]));
        let ids: Vec<&str> = apps.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["alpha.desktop", "foo-bar.desktop"]);
        assert_eq!(apps[0].name, "Alpha User");
    }

    #[test]
    fn symlinked_entries_and_dirs_are_followed() {
        let tmp = TempDir::new().unwrap();
        let real = write(tmp.path(), "real/app.desktop", &entry("Linked", ""));
        let dir = tmp.path().join("apps");
        fs::create_dir_all(&dir).unwrap();
        symlink(&real, dir.join("linked.desktop")).unwrap();
        symlink(real.parent().unwrap(), dir.join("sub")).unwrap();
        let apps = scan_applications(&cfg(vec![dir]));
        let ids: Vec<&str> = apps.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["linked.desktop", "sub-app.desktop"]);
    }

    #[test]
    fn duplicate_names_get_origin_subtitles() {
        let tmp = TempDir::new().unwrap();
        let home = tmp.path().join("home/.local/share");
        let user = home.join("applications");
        let flat = tmp
            .path()
            .join("var/lib/flatpak/exports/share/applications");
        let sys = tmp.path().join("usr/share/applications");
        write(&user, "ff.desktop", &entry("Firefox", ""));
        write(&flat, "org.mozilla.firefox.desktop", &entry("firefox", ""));
        write(&sys, "ff-esr.desktop", &entry("Firefox", ""));
        write(&sys, "other.desktop", &entry("Other", ""));
        let mut c = cfg(vec![user, flat, sys]);
        c.data_home = home;
        let apps = scan_applications(&c);
        let sub = |id: &str| apps.iter().find(|a| a.id == id).unwrap().subtitle.clone();
        assert_eq!(sub("ff.desktop").as_deref(), Some("User"));
        assert_eq!(
            sub("org.mozilla.firefox.desktop").as_deref(),
            Some("Flatpak")
        );
        assert_eq!(sub("ff-esr.desktop").as_deref(), Some("System"));
        assert_eq!(sub("other.desktop"), None);
    }

    #[test]
    fn same_origin_duplicates_fall_back_to_id() {
        let tmp = TempDir::new().unwrap();
        let sys = tmp.path().join("usr/share/applications");
        write(&sys, "a.desktop", &entry("Same", ""));
        write(&sys, "b.desktop", &entry("Same", ""));
        let apps = scan_applications(&cfg(vec![sys]));
        assert_eq!(apps[0].subtitle.as_deref(), Some("System (a)"));
        assert_eq!(apps[1].subtitle.as_deref(), Some("System (b)"));
    }

    #[test]
    fn natural_ordering() {
        let mut v = vec!["App 10", "app 9", "Ärgern", "Zed", "app 2"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, ["app 2", "app 9", "App 10", "Ärgern", "Zed"]);
    }

    #[test]
    fn cache_roundtrip_and_corruption() {
        let tmp = TempDir::new().unwrap();
        let sys = tmp.path().join("applications");
        write(&sys, "z.desktop", &entry("Zed", "Keywords=editor;\n"));
        write(&sys, "a.desktop", &entry("Alpha", ""));
        let apps = scan_applications(&cfg(vec![sys]));
        let cache = tmp.path().join("cache/kebloop/application-index.json");
        save_cache(&cache, &apps).unwrap();
        assert_eq!(load_cache(&cache), apps);
        fs::write(&cache, "{not json").unwrap();
        assert!(load_cache(&cache).is_empty());
        assert!(load_cache(&tmp.path().join("missing.json")).is_empty());
    }

    #[test]
    fn refresh_due() {
        let now = Instant::now();
        assert!(is_refresh_due(None, now, REFRESH_INTERVAL));
        assert!(!is_refresh_due(Some(now), now, REFRESH_INTERVAL));
        assert!(is_refresh_due(
            Some(now),
            now + REFRESH_INTERVAL,
            REFRESH_INTERVAL
        ));
    }

    #[test]
    fn application_dirs_order() {
        let env = Env::from_pairs([("HOME", "/h"), ("XDG_DATA_DIRS", "/a:/b:/a")]);
        let dirs = application_dirs(&XdgDirs::from_env(&env));
        assert_eq!(
            dirs,
            [
                PathBuf::from("/h/.local/share/applications"),
                PathBuf::from("/a/applications"),
                PathBuf::from("/b/applications")
            ]
        );
    }

    fn wait_for_signal(rx: &async_channel::Receiver<AppsChanged>) -> bool {
        for _ in 0..100 {
            if rx.try_recv().is_ok() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }

    #[test]
    fn watcher_signals_on_changes_and_debounces() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("applications");
        fs::create_dir_all(dir.join("sub")).unwrap();
        let (_w, rx) =
            AppWatcher::start(std::slice::from_ref(&dir), Duration::from_millis(100)).unwrap();
        write(&dir, "sub/new.desktop", &entry("New", ""));
        write(&dir, "sub/new2.desktop", &entry("New2", ""));
        assert!(wait_for_signal(&rx));
        std::thread::sleep(Duration::from_millis(400));
        assert!(
            rx.try_recv().is_err(),
            "burst must collapse into one signal"
        );
        // Unrelated files do not trigger a signal.
        write(&dir, "mimeinfo.cache", "x");
        std::thread::sleep(Duration::from_millis(400));
        assert!(rx.try_recv().is_err());
        fs::remove_file(dir.join("sub/new.desktop")).unwrap();
        assert!(wait_for_signal(&rx));
    }

    /// Read-only scan of the real machine.
    #[test]
    #[ignore = "scans the real system application directories"]
    fn live_scan() {
        let cfg = ScanConfig::from_process_env();
        let started = Instant::now();
        let apps = scan_applications(&cfg);
        eprintln!(
            "{} applications in {:?} (dirs: {:?}, desktop: {:?}, locales: {:?})",
            apps.len(),
            started.elapsed(),
            cfg.app_dirs,
            cfg.current_desktop,
            cfg.locales.candidates()
        );
        for app in apps.iter().take(8) {
            eprintln!(
                "  {} [{}] {:?} {:?}",
                app.name, app.id, app.subtitle, app.origin
            );
        }
        eprintln!(
            "duplicates: {:?}",
            apps.iter()
                .filter(|a| a.subtitle.is_some())
                .map(|a| &a.name)
                .collect::<Vec<_>>()
        );
        assert!(!apps.is_empty());
    }
}
