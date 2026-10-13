//! File index wiring: roots and exclusions from the settings, rebuilt on change and every thirty
//! minutes. Queries run on worker threads from the launcher.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gtk::glib;
use kebloop_platform::files::{ExcludeFn, FileIndexConfig, FileIndexEvent, default_roots};
use kebloop_services::personalization::contains;
use kebloop_services::util::{expand_tilde, normalize_path};

use crate::context::{AppContext, Change};

/// Periodic rebuild interval.
const REBUILD_SECONDS: u32 = 30 * 60;

/// The part of the settings that decides what is indexed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IndexPolicy {
    /// Searching files is switched on.
    pub enabled: bool,
    /// Searchable folders as stored.
    pub folders: Vec<String>,
    /// Excluded folders as stored.
    pub exclusions: Vec<String>,
}

impl IndexPolicy {
    /// Read from the active profile.
    pub fn of(ctx: &AppContext) -> Self {
        ctx.with_settings(|s| Self {
            enabled: s.personalization.files_enabled,
            folders: s.personalization.folders.clone(),
            exclusions: s.personalization.exclusions.clone(),
        })
    }
}

/// Recursive roots and shallow roots for the configured folders. `~/` selects the XDG user
/// directories plus the top level of the home directory, like the default configuration.
pub fn roots_for(
    folders: &[String],
    home: &str,
    user_dirs_file: &Path,
) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let home_path = PathBuf::from(normalize_path(home));
    let mut roots: Vec<PathBuf> = Vec::new();
    let mut shallow: Vec<PathBuf> = Vec::new();
    for folder in folders {
        let expanded = PathBuf::from(normalize_path(&expand_tilde(folder, home)));
        if expanded == home_path {
            for root in default_roots(&home_path, user_dirs_file) {
                if !roots.contains(&root) {
                    roots.push(root);
                }
            }
            if !shallow.contains(&home_path) {
                shallow.push(home_path.clone());
            }
        } else if !roots.contains(&expanded) {
            roots.push(expanded);
        }
    }
    (roots, shallow)
}

/// Start listening for index updates, build the first index and schedule rebuilds.
pub fn init(ctx: &Rc<AppContext>, events: async_channel::Receiver<FileIndexEvent>) {
    let listening = ctx.clone();
    glib::spawn_future_local(async move {
        while events.recv().await.is_ok() {
            listening.emit(Change::Files);
        }
    });
    let periodic = ctx.clone();
    glib::timeout_add_seconds_local(REBUILD_SECONDS, move || {
        rebuild(&periodic);
        glib::ControlFlow::Continue
    });
    rebuild(ctx);
}

/// Rebuild the index from the current settings (or drop it when file search is off).
pub fn rebuild(ctx: &AppContext) {
    let policy = IndexPolicy::of(ctx);
    if !policy.enabled || policy.folders.is_empty() {
        ctx.file_indexer.cancel();
        return;
    }
    let (roots, shallow_roots) = roots_for(&policy.folders, &ctx.home, &ctx.xdg.user_dirs_file());
    let home = ctx.home.clone();
    let rules = ctx.file_rules.clone();
    let exclusions = policy.exclusions;
    let exclude: ExcludeFn = Arc::new(move |path: &Path| {
        let text = path.to_string_lossy();
        !rules.is_visible_file(&text) || exclusions.iter().any(|e| contains(&text, e, &home))
    });
    ctx.file_indexer.rebuild(FileIndexConfig {
        roots,
        shallow_roots,
        home: PathBuf::from(&ctx.home),
        max_entries: kebloop_platform::files::DEFAULT_MAX_ENTRIES,
        threads: 0,
        exclude,
    });
}
