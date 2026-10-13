//! Search / appearance preferences, pinned results and usage statistics.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::ConfigError;
use crate::util::{expand_tilde, normalize_path};

/// Maximum number of remembered recent results.
pub const MAX_RECENT_RESULTS: usize = 20;
/// Maximum number of remembered recent emoji.
pub const MAX_EMOJI_RECENTS: usize = 24;

fn yes() -> bool {
    true
}
fn default_folders() -> Vec<String> {
    vec!["~/".to_string()]
}
fn default_empty_mode() -> String {
    "frequent".into()
}
fn default_appearance() -> String {
    "system".into()
}
fn default_text_size() -> f64 {
    14.0
}
fn default_panel_width() -> f64 {
    560.0
}
fn default_placement() -> String {
    "pointer".into()
}

/// Per-profile search and appearance preferences. Every field has a default so older or partial
/// documents (and documents from newer macOS builds) still load; unknown keys are preserved.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonalizationSettings {
    /// Search applications.
    #[serde(default = "yes")]
    pub apps_enabled: bool,
    /// Search files.
    #[serde(default = "yes")]
    pub files_enabled: bool,
    /// Search quicklinks.
    #[serde(default = "yes")]
    pub links_enabled: bool,
    /// Search commands.
    #[serde(default = "yes")]
    pub commands_enabled: bool,
    /// Indexed folders (absolute or `~/` paths). Defaults to the whole home directory.
    #[serde(default = "default_folders")]
    pub folders: Vec<String>,
    /// Excluded folders (absolute or `~/` paths).
    #[serde(default)]
    pub exclusions: Vec<String>,
    /// Hidden applications (desktop ids on Linux, bundle paths from macOS).
    #[serde(default)]
    pub hidden_apps: Vec<String>,
    /// Empty-launcher behaviour: `frequent`, `recent` or `favorites`.
    #[serde(default = "default_empty_mode")]
    pub empty_mode: String,
    /// `system`, `light` or `dark`.
    #[serde(default = "default_appearance")]
    pub appearance: String,
    /// Text size in points (11 to 24).
    #[serde(default = "default_text_size")]
    pub text_size: f64,
    /// Panel width (420 to 1000).
    #[serde(default = "default_panel_width")]
    pub panel_width: f64,
    /// Compact rows.
    #[serde(default)]
    pub compact: bool,
    /// Translucent background.
    #[serde(default = "yes")]
    pub transparency: bool,
    /// Show shortcut hints.
    #[serde(default = "yes")]
    pub show_shortcut_hints: bool,
    /// `pointer`, `active` or `display`.
    #[serde(default = "default_placement")]
    pub placement: String,
    /// Output / display identifier used when `placement` is `display`.
    #[serde(default, rename = "displayID")]
    pub display_id: String,
    /// Unknown keys (kept so a Linux round trip does not drop newer macOS fields).
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl Default for PersonalizationSettings {
    fn default() -> Self {
        Self {
            apps_enabled: true,
            files_enabled: true,
            links_enabled: true,
            commands_enabled: true,
            folders: default_folders(),
            exclusions: Vec::new(),
            hidden_apps: Vec::new(),
            empty_mode: default_empty_mode(),
            appearance: default_appearance(),
            text_size: default_text_size(),
            panel_width: default_panel_width(),
            compact: false,
            transparency: true,
            show_shortcut_hints: true,
            placement: default_placement(),
            display_id: String::new(),
            extra: BTreeMap::new(),
        }
    }
}

fn is_path(value: &str) -> bool {
    value.starts_with('/') || value.starts_with("~/")
}

impl PersonalizationSettings {
    /// Validate ranges and path syntax.
    pub fn validate(&self) -> Result<(), ConfigError> {
        let valid = ["frequent", "recent", "favorites"].contains(&self.empty_mode.as_str())
            && ["system", "light", "dark"].contains(&self.appearance.as_str())
            && ["pointer", "active", "display"].contains(&self.placement.as_str())
            && self.text_size.is_finite()
            && (11.0..=24.0).contains(&self.text_size)
            && self.panel_width.is_finite()
            && (420.0..=1000.0).contains(&self.panel_width)
            && self
                .folders
                .iter()
                .chain(&self.exclusions)
                .all(|p| is_path(p))
            && self
                .hidden_apps
                .iter()
                .all(|a| !a.is_empty() && !a.contains('\0'));
        if valid {
            Ok(())
        } else {
            Err(ConfigError::invalid(
                "Invalid search or appearance preferences.",
            ))
        }
    }

    /// Whether a file path is inside an indexed folder and outside every exclusion.
    pub fn allows_file(&self, path: &str, home: &str) -> bool {
        if !self.files_enabled {
            return false;
        }
        let normalized = normalize_path(&expand_tilde(path, home));
        self.folders.iter().any(|f| contains(&normalized, f, home))
            && !self
                .exclusions
                .iter()
                .any(|f| contains(&normalized, f, home))
    }
}

/// Path-boundary-aware containment: `/a/Docs` contains `/a/Docs/x` but not `/a/Docs-old/x`.
pub fn contains(path: &str, folder: &str, home: &str) -> bool {
    let root = normalize_path(&expand_tilde(folder, home));
    root == "/" || path == root || path.starts_with(&format!("{root}/"))
}

/// A pinned (favourite) result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinnedResult {
    /// Result identifier, e.g. `file:/path` or `link:<id>`.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Path or target.
    pub path: String,
    /// `application`, `file`, `quicklink` or `command`.
    pub kind: String,
}

/// Whether a result is pinned.
pub fn is_pinned(pins: &[PinnedResult], id: &str) -> bool {
    pins.iter().any(|p| p.id == id)
}

/// Pin an unpinned result or unpin a pinned one. Returns the new pinned state.
pub fn toggle_pin(pins: &mut Vec<PinnedResult>, result: PinnedResult) -> bool {
    if is_pinned(pins, &result.id) {
        pins.retain(|p| p.id != result.id);
        false
    } else {
        pins.push(result);
        true
    }
}

/// Remove a pin by id.
pub fn remove_pin(pins: &mut Vec<PinnedResult>, id: &str) {
    pins.retain(|p| p.id != id);
}

/// Global (not profile-scoped) usage state: launch counts and recents.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageState {
    /// Launch counts keyed by result id (the macOS `usage` dictionary).
    #[serde(default)]
    pub usage: BTreeMap<String, u64>,
    /// Most recent result ids, newest first (max [`MAX_RECENT_RESULTS`]).
    #[serde(default)]
    pub recent_results: Vec<String>,
    /// Most recent emoji, newest first (max [`MAX_EMOJI_RECENTS`]).
    #[serde(default)]
    pub emoji_recents: Vec<String>,
}

impl UsageState {
    /// Count one successful launch of `id`.
    pub fn record_launch(&mut self, id: &str) {
        let count = self.usage.entry(id.to_string()).or_insert(0);
        *count = count.saturating_add(1);
    }

    /// Launch count for `id`.
    pub fn launch_count(&self, id: &str) -> u64 {
        self.usage.get(id).copied().unwrap_or(0)
    }

    /// Ids ordered by launch count (descending), ties broken by id.
    pub fn most_used(&self, limit: usize) -> Vec<String> {
        let mut entries: Vec<_> = self.usage.iter().filter(|(_, c)| **c > 0).collect();
        entries.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
        entries
            .into_iter()
            .take(limit)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Move `id` to the front of the recents.
    pub fn record_recent(&mut self, id: &str) {
        self.recent_results.retain(|r| r != id);
        self.recent_results.insert(0, id.to_string());
        self.recent_results.truncate(MAX_RECENT_RESULTS);
    }

    /// Move an emoji to the front of the recents.
    pub fn record_emoji(&mut self, symbol: &str) {
        self.emoji_recents.retain(|s| s != symbol);
        self.emoji_recents.insert(0, symbol.to_string());
        self.emoji_recents.truncate(MAX_EMOJI_RECENTS);
    }

    /// Trim over-long lists loaded from disk.
    pub(crate) fn sanitize(&mut self) {
        self.recent_results.truncate(MAX_RECENT_RESULTS);
        self.emoji_recents.truncate(MAX_EMOJI_RECENTS);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: &str = "/home/example";

    #[test]
    fn folder_boundaries() {
        let settings = PersonalizationSettings {
            folders: vec!["/home/example/Documents".into()],
            exclusions: vec!["/home/example/Documents/Private".into()],
            ..PersonalizationSettings::default()
        };
        assert!(settings.allows_file("/home/example/Documents/report.txt", HOME));
        assert!(!settings.allows_file("/home/example/Documents-old/report.txt", HOME));
        assert!(!settings.allows_file("/home/example/Documents/Private/report.txt", HOME));
        assert!(settings.allows_file("/home/example/Documents/Privateer/report.txt", HOME));
        assert!(!settings.allows_file("/home/example/Documents/../Other/x", HOME));
    }

    #[test]
    fn disabled_files_deny_everything_and_tilde_folders_expand() {
        let mut settings = PersonalizationSettings::default();
        assert!(settings.allows_file("/home/example/a.txt", HOME));
        settings.files_enabled = false;
        assert!(!settings.allows_file("/home/example/a.txt", HOME));
    }

    #[test]
    fn pins_toggle() {
        let mut pins = Vec::new();
        let pin = PinnedResult {
            id: "file:/home/example/Documents/report.txt".into(),
            name: "Report".into(),
            path: "/home/example/Documents/report.txt".into(),
            kind: "file".into(),
        };
        assert!(toggle_pin(&mut pins, pin.clone()));
        assert_eq!(pins.len(), 1);
        assert_eq!(pins[0], pin);
        assert!(!toggle_pin(&mut pins, pin));
        assert!(pins.is_empty());
    }

    #[test]
    fn older_documents_keep_choices_and_default_new_fields() {
        let old =
            r#"{"appsEnabled":false,"appearance":"dark","folders":["/home/example/Documents"]}"#;
        let mut settings: PersonalizationSettings = serde_json::from_str(old).unwrap();
        assert!(!settings.apps_enabled);
        assert_eq!(settings.appearance, "dark");
        assert_eq!(settings.folders, ["/home/example/Documents"]);
        assert!(settings.show_shortcut_hints);
        settings.show_shortcut_hints = false;
        let restored: PersonalizationSettings =
            serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert!(!restored.show_shortcut_hints);
    }

    #[test]
    fn unknown_fields_round_trip() {
        let json = r#"{"futureFlag":{"a":1},"textSize":16}"#;
        let settings: PersonalizationSettings = serde_json::from_str(json).unwrap();
        assert_eq!(settings.text_size, 16.0);
        let out = serde_json::to_value(&settings).unwrap();
        assert_eq!(out["futureFlag"]["a"], 1);
    }

    #[test]
    fn validation_ranges() {
        let mut settings = PersonalizationSettings::default();
        assert!(settings.validate().is_ok());
        settings.text_size = 30.0;
        assert!(settings.validate().is_err());
        settings.text_size = 14.0;
        settings.folders = vec!["relative".into()];
        assert!(settings.validate().is_err());
        settings.folders = vec!["/ok".into()];
        settings.placement = "nowhere".into();
        assert!(settings.validate().is_err());
    }

    #[test]
    fn usage_counts_and_recents() {
        let mut usage = UsageState::default();
        usage.record_launch("app:a");
        usage.record_launch("app:a");
        usage.record_launch("app:b");
        assert_eq!(usage.launch_count("app:a"), 2);
        assert_eq!(usage.most_used(1), ["app:a"]);
        for i in 0..30 {
            usage.record_recent(&format!("r{i}"));
            usage.record_emoji(&format!("e{i}"));
        }
        usage.record_recent("r25");
        assert_eq!(usage.recent_results.len(), MAX_RECENT_RESULTS);
        assert_eq!(usage.recent_results[0], "r25");
        assert_eq!(usage.emoji_recents.len(), MAX_EMOJI_RECENTS);
        assert_eq!(usage.emoji_recents[0], "e29");
    }
}
