//! Configuration: profiles, portable export/import and the single-file persisted store.
//!
//! Persisted layout (`config.json`):
//!
//! ```json
//! {
//!   "version": 1,
//!   "activeProfileID": "…",
//!   "profiles": [ { "id": "…", "name": "Personal", "settings": { /* PortableSettings */ } } ],
//!   "global": { "aiProvider": "openai", "usage": {}, "recentResults": [], "emojiRecents": [],
//!               "extensionPreferences": {} }
//! }
//! ```
//!
//! `settings` of a profile uses exactly the macOS export schema (`PortableSettings`), and the
//! export file is a `PortableConfiguration` (`version`, `settings`, `profiles`,
//! `activeProfileID`). Everything under `global` is never exported.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::accelerator::Accelerator;
use crate::ai::AiProvider;
use crate::aliases;
use std::sync::Arc;

use crate::automation::{
    ExtensionManifest, ScriptCommand, retain_valid_extensions, retain_valid_scripts,
};
use crate::commands::LauncherCommand;
use crate::error::ConfigError;
use crate::persist::DebouncedWriter;
use crate::personalization::{PersonalizationSettings, PinnedResult, UsageState};
use crate::quicklinks::{
    Destination, Quicklink, WebSearchTemplate, destination, retain_resolvable,
};
use crate::shortcuts::{
    AppShortcutBinding, CommandShortcutBinding, Shortcut, ShortcutOwner, identity,
    unavailable_bindings, validate_chord,
};
use crate::snippets::Snippet;
use crate::util::{atomic_write, corrupt_destination, expand_tilde, is_uuid, new_id};

/// Import size limit in bytes.
pub const MAX_IMPORT_BYTES: usize = 5_000_000;
/// Placeholder home used when validation does not depend on the real home directory.
const VALIDATION_HOME: &str = "/home/validation";

/// An Apple Shortcut selected on macOS. Kept so configurations round-trip; inactive on Linux.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AppleShortcut {
    /// UUID of the shortcut.
    pub id: String,
    /// Shortcut name.
    pub name: String,
}

fn default_launcher_id() -> String {
    "option".into()
}

fn default_web_searches() -> Vec<WebSearchTemplate> {
    vec![WebSearchTemplate::default_search()]
}

/// Everything that is part of a profile and of the portable export.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortableSettings {
    /// Launcher shortcut choice (`option`, `control`, `commandShift`, `command`, `custom`).
    #[serde(default = "default_launcher_id", rename = "launcherShortcutID")]
    pub launcher_shortcut_id: String,
    /// Shortcut used when the choice is `custom`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launcher_custom_shortcut: Option<Shortcut>,
    /// Application aliases keyed by application identity.
    #[serde(default)]
    pub aliases: BTreeMap<String, String>,
    /// Quicklinks.
    #[serde(default)]
    pub quicklinks: Vec<Quicklink>,
    /// Web search templates.
    #[serde(default = "default_web_searches")]
    pub web_searches: Vec<WebSearchTemplate>,
    /// Application shortcuts.
    #[serde(default)]
    pub app_shortcuts: Vec<AppShortcutBinding>,
    /// Command shortcuts.
    #[serde(default)]
    pub command_shortcuts: Vec<CommandShortcutBinding>,
    /// macOS Apple Shortcuts (inactive on Linux, preserved).
    #[serde(default)]
    pub selected_apple_shortcuts: Vec<AppleShortcut>,
    /// Search and appearance preferences.
    #[serde(default)]
    pub personalization: PersonalizationSettings,
    /// Pinned results.
    #[serde(default)]
    pub pins: Vec<PinnedResult>,
    /// Snippets.
    #[serde(default)]
    pub snippets: Vec<Snippet>,
    /// Script commands.
    #[serde(default)]
    pub scripts: Vec<ScriptCommand>,
    /// Extension manifests.
    #[serde(default)]
    pub extensions: Vec<ExtensionManifest>,
    /// Unknown keys from newer or foreign exports, preserved verbatim.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl Default for PortableSettings {
    fn default() -> Self {
        Self {
            launcher_shortcut_id: default_launcher_id(),
            launcher_custom_shortcut: None,
            aliases: BTreeMap::new(),
            quicklinks: Vec::new(),
            web_searches: default_web_searches(),
            app_shortcuts: Vec::new(),
            command_shortcuts: Vec::new(),
            selected_apple_shortcuts: Vec::new(),
            personalization: PersonalizationSettings::default(),
            pins: Vec::new(),
            snippets: Vec::new(),
            scripts: Vec::new(),
            extensions: Vec::new(),
            extra: BTreeMap::new(),
        }
    }
}

fn unique<'a>(ids: impl IntoIterator<Item = &'a str>) -> Result<(), ConfigError> {
    let mut seen = BTreeSet::new();
    for id in ids {
        if id.is_empty() || !seen.insert(id) {
            return Err(ConfigError::invalid(
                "Configuration contains duplicate identifiers or conflicting shortcuts.",
            ));
        }
    }
    Ok(())
}

impl PortableSettings {
    /// The effective launcher shortcut definition, if the id is known.
    pub fn launcher_shortcut(&self) -> Option<Shortcut> {
        if self.launcher_shortcut_id == "custom" {
            self.launcher_custom_shortcut.clone()
        } else {
            Shortcut::choices()
                .into_iter()
                .find(|s| s.id == self.launcher_shortcut_id)
        }
    }

    /// Launcher accelerator on Linux. Falls back to `<Alt>space` when an imported macOS shortcut
    /// has no Linux equivalent.
    pub fn launcher_accelerator(&self) -> Accelerator {
        self.launcher_shortcut()
            .and_then(|s| s.resolved())
            .unwrap_or_else(Accelerator::default_launcher)
    }

    /// Apply load-time cleanups (drop invalid aliases, unresolvable links, invalid automation).
    pub fn sanitize(&mut self) {
        self.aliases = aliases::clean(&self.aliases);
        retain_resolvable(&mut self.quicklinks, VALIDATION_HOME);
        retain_valid_scripts(&mut self.scripts);
        retain_valid_extensions(&mut self.extensions);
    }

    /// Set every script command and extension to disabled (imports require review).
    pub fn require_automation_review(&mut self) {
        for command in &mut self.scripts {
            command.enabled = false;
        }
        for manifest in &mut self.extensions {
            manifest.enabled = false;
        }
    }

    /// Full validation of the portable schema.
    pub fn validate(&self) -> Result<(), ConfigError> {
        self.personalization.validate()?;
        for command in &self.scripts {
            command.validate()?;
        }
        unique(self.scripts.iter().map(|c| c.id.as_str()))?;
        for manifest in &self.extensions {
            manifest.validate()?;
        }
        unique(self.extensions.iter().map(|m| m.id.as_str()))?;
        for snippet in &self.snippets {
            snippet.validate()?;
        }
        unique(self.snippets.iter().map(|s| s.id.as_str()))?;
        Snippet::validate_unique_keywords(&self.snippets)?;

        let launcher = self
            .launcher_shortcut()
            .ok_or_else(|| ConfigError::invalid("Unknown launcher shortcut."))?;
        validate_chord(
            launcher.key_code,
            launcher.modifiers,
            launcher.accelerator.as_deref(),
            "The launcher shortcut",
        )?;
        let launcher_identity = identity(
            launcher.key_code,
            launcher.modifiers,
            launcher.accelerator.as_deref(),
        );
        let app_ids: Vec<Option<String>> = self
            .app_shortcuts
            .iter()
            .map(|b| identity(b.key_code, b.modifiers, b.accelerator.as_deref()))
            .collect();
        let command_ids: Vec<Option<String>> = self
            .command_shortcuts
            .iter()
            .map(|b| identity(b.key_code, b.modifiers, b.accelerator.as_deref()))
            .collect();
        if launcher_identity.is_some()
            && app_ids
                .iter()
                .chain(&command_ids)
                .any(|i| *i == launcher_identity)
        {
            return Err(ConfigError::invalid(
                "A command or application shortcut conflicts with the launcher.",
            ));
        }

        let alias_values: BTreeSet<String> = self
            .aliases
            .values()
            .map(|v| aliases::normalized(v))
            .collect();
        let aliases_ok = self.aliases.iter().all(|(key, value)| {
            !key.is_empty()
                && !aliases::normalized(value).is_empty()
                && value.chars().count() <= 100
        }) && alias_values.len() == self.aliases.len()
            && self.quicklinks.iter().all(|l| {
                !l.id.is_empty()
                    && !l.name.is_empty()
                    && destination(&l.target, "", VALIDATION_HOME).is_some()
            })
            && self.pins.iter().all(|p| {
                !p.id.is_empty()
                    && !p.name.is_empty()
                    && ["application", "file", "quicklink", "command"].contains(&p.kind.as_str())
            });
        if !aliases_ok {
            return Err(ConfigError::invalid(
                "Invalid aliases, quicklinks, or pinned results.",
            ));
        }

        let keywords: Vec<&str> = self
            .quicklinks
            .iter()
            .map(|l| l.keyword.as_str())
            .filter(|k| !k.is_empty())
            .chain(self.web_searches.iter().map(|s| s.keyword.as_str()))
            .collect();
        let search_ok = keywords
            .iter()
            .all(|k| !k.is_empty() && !k.chars().any(char::is_whitespace))
            && self.web_searches.iter().all(|s| {
                !s.name.is_empty()
                    && s.target.contains("{query}")
                    && destination(&s.target, "test", VALIDATION_HOME).is_some_and(|d| !d.is_file())
            })
            && self
                .selected_apple_shortcuts
                .iter()
                .all(|a| !a.name.is_empty() && is_uuid(&a.id));
        if !search_ok {
            return Err(ConfigError::invalid(
                "Invalid web-search templates, keywords, or Apple Shortcuts.",
            ));
        }
        let lowered: Vec<String> = keywords.iter().map(|k| k.to_lowercase()).collect();
        unique(lowered.iter().map(String::as_str))?;
        unique(self.quicklinks.iter().map(|l| l.id.as_str()))?;
        unique(self.web_searches.iter().map(|s| s.id.as_str()))?;
        unique(self.pins.iter().map(|p| p.id.as_str()))?;
        unique(self.app_shortcuts.iter().map(|b| b.id.as_str()))?;
        unique(self.command_shortcuts.iter().map(|b| b.id.as_str()))?;
        unique(self.selected_apple_shortcuts.iter().map(|a| a.id.as_str()))?;

        let mut known: BTreeSet<String> = BTreeSet::new();
        known.extend(self.scripts.iter().map(ScriptCommand::command_id));
        known.extend(self.extensions.iter().map(ExtensionManifest::command_id));
        known.extend(self.snippets.iter().map(|s| format!("snippet:{}", s.id)));
        known.extend(LauncherCommand::ALL.iter().map(|c| c.id().to_string()));
        known.extend(
            self.selected_apple_shortcuts
                .iter()
                .map(|a| format!("apple:{}", a.id)),
        );
        known.extend(self.quicklinks.iter().map(|l| format!("link:{}", l.id)));
        let shortcuts_ok =
            self.command_shortcuts
                .iter()
                .all(|b| known.contains(&b.id) && !b.label.is_empty())
                && self.app_shortcuts.iter().all(|b| {
                    !b.app_path.is_empty() && !b.app_name.is_empty() && !b.label.is_empty()
                });
        if !shortcuts_ok {
            return Err(ConfigError::invalid(
                "Unknown command identifiers or invalid application shortcuts.",
            ));
        }
        let combinations: Vec<&str> = app_ids
            .iter()
            .chain(&command_ids)
            .flatten()
            .map(String::as_str)
            .collect();
        unique(combinations)?;
        for binding in &self.app_shortcuts {
            validate_chord(
                binding.key_code,
                binding.modifiers,
                binding.accelerator.as_deref(),
                &binding.label,
            )?;
        }
        for binding in &self.command_shortcuts {
            validate_chord(
                binding.key_code,
                binding.modifiers,
                binding.accelerator.as_deref(),
                &binding.label,
            )?;
        }
        Ok(())
    }
}

/// A named configuration profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfigurationProfile {
    /// Stable identifier.
    pub id: String,
    /// Display name.
    pub name: String,
    /// The profile's settings.
    pub settings: PortableSettings,
}

/// The portable export document (compatible with the macOS app).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortableConfiguration {
    /// Schema version (1).
    pub version: u32,
    /// Settings of the exporting app's active profile.
    pub settings: PortableSettings,
    /// Exported profiles.
    pub profiles: Vec<ConfigurationProfile>,
    /// Active profile id.
    #[serde(rename = "activeProfileID")]
    pub active_profile_id: String,
}

/// Why an imported item cannot be used on this machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnavailableKind {
    /// A file or folder does not exist.
    MissingPath,
    /// A file or folder exists but is not readable.
    RestrictedPath,
    /// A macOS Apple Shortcut.
    AppleShortcut,
    /// A shortcut whose key has no Linux equivalent.
    Shortcut,
}

/// Imported item that is kept in the configuration but inactive here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnavailableItem {
    /// Category.
    pub kind: UnavailableKind,
    /// Path, shortcut name or label.
    pub subject: String,
}

/// Result of inspecting an import file; nothing is applied yet.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigurationPreview {
    /// The validated configuration (scripts/extensions forced disabled).
    pub configuration: PortableConfiguration,
    /// Counts for display.
    pub summary: Vec<String>,
    /// Conflict and availability warnings.
    pub warnings: Vec<String>,
    /// Structured list of unavailable items.
    pub unavailable: Vec<UnavailableItem>,
}

/// Import strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportMode {
    /// Keep current preferences and conflicting identifiers; add what is new.
    Merge,
    /// Use the imported values (a backup of the current configuration is written first).
    Replace,
}

/// Settings that are not part of profiles or exports.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GlobalSettings {
    /// Provider for AI handoff.
    #[serde(default)]
    pub ai_provider: AiProvider,
    /// Launch counts and recents.
    #[serde(flatten)]
    pub usage: UsageState,
    /// Saved extension preference values, `extension id -> key -> value`.
    #[serde(default)]
    pub extension_preferences: BTreeMap<String, BTreeMap<String, String>>,
    /// The first-run walkthrough was completed or skipped.
    #[serde(default)]
    pub onboarding_dismissed: bool,
    /// Explicit local consent for accessibility-based snippet expansion; never exported.
    #[serde(default)]
    pub snippet_keyword_expansion_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConfigFile {
    version: u32,
    #[serde(rename = "activeProfileID")]
    active_profile_id: String,
    profiles: Vec<ConfigurationProfile>,
    #[serde(default)]
    global: GlobalSettings,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

impl ConfigFile {
    fn fresh() -> Self {
        let profile = ConfigurationProfile {
            id: new_id(),
            name: "Personal".into(),
            settings: PortableSettings::default(),
        };
        Self {
            version: 1,
            active_profile_id: profile.id.clone(),
            profiles: vec![profile],
            global: GlobalSettings::default(),
            extra: BTreeMap::new(),
        }
    }

    fn active_index(&self) -> usize {
        self.profiles
            .iter()
            .position(|p| p.id == self.active_profile_id)
            .unwrap_or(0)
    }
}

/// Where the configuration lives. Injected so tests never touch real directories.
#[derive(Debug, Clone)]
pub struct ConfigPaths {
    /// The JSON file (`$XDG_CONFIG_HOME/kebloop/config.json`).
    pub config_file: PathBuf,
    /// Directory for backups written before a replace import.
    pub backup_dir: PathBuf,
}

impl ConfigPaths {
    /// Standard layout inside a config directory: `config.json` and `backups/`.
    pub fn in_dir(dir: &Path) -> Self {
        Self {
            config_file: dir.join("config.json"),
            backup_dir: dir.join("backups"),
        }
    }
}

/// Error from [`ConfigStore::update`]: either the caller's own rejection or a store failure.
#[derive(Debug, thiserror::Error)]
pub enum UpdateError<E: std::error::Error + 'static> {
    /// The update closure refused the change.
    #[error(transparent)]
    Rejected(E),
    /// Validation or persistence failed.
    #[error(transparent)]
    Config(#[from] ConfigError),
}

/// Activation hook run after profile switches and imports. Returning an error rolls the change
/// back (e.g. when a shortcut cannot be registered).
pub type ActivationHook<'a> = &'a mut dyn FnMut(&PortableSettings) -> Result<(), ConfigError>;

/// The persisted configuration store.
#[derive(Debug)]
pub struct ConfigStore {
    paths: ConfigPaths,
    home: String,
    file: ConfigFile,
    recovered_corrupt: Option<PathBuf>,
    created: bool,
    /// Why the store runs from memory only (nothing is read or written), if it does.
    ephemeral: Option<String>,
    /// Deferred writer for frequent, low-value updates (usage counters).
    writer: Option<Arc<DebouncedWriter>>,
}

impl ConfigStore {
    /// Open (or create) the store. A corrupt file is moved aside (`*.corrupt`) and replaced by
    /// defaults; see [`ConfigStore::recovered_corrupt_file`].
    pub fn open(paths: ConfigPaths, home: &str) -> Result<Self, ConfigError> {
        let mut recovered = None;
        let original = match fs::read(&paths.config_file) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == ErrorKind::NotFound => None,
            Err(e) => return Err(ConfigError::io(&paths.config_file, e)),
        };
        let mut file = match &original {
            None => ConfigFile::fresh(),
            Some(bytes) => match serde_json::from_slice::<ConfigFile>(bytes) {
                Ok(file) if file.version > 1 => {
                    return Err(ConfigError::invalid(format!(
                        "The configuration was written by a newer Kebloop (version {}).",
                        file.version
                    )));
                }
                Ok(file) => file,
                Err(_) => {
                    let aside = corrupt_destination(&paths.config_file);
                    fs::rename(&paths.config_file, &aside)
                        .map_err(|e| ConfigError::io(&paths.config_file, e))?;
                    recovered = Some(aside);
                    ConfigFile::fresh()
                }
            },
        };
        if file.profiles.is_empty() {
            file.profiles = ConfigFile::fresh().profiles;
            file.active_profile_id.clear();
        }
        if !file.profiles.iter().any(|p| p.id == file.active_profile_id) {
            file.active_profile_id = file.profiles[0].id.clone();
        }
        for profile in &mut file.profiles {
            profile.settings.sanitize();
        }
        file.global.usage.sanitize();
        let store = Self {
            paths,
            home: home.to_string(),
            file,
            recovered_corrupt: recovered,
            created: original.is_none(),
            ephemeral: None,
            writer: None,
        };
        let encoded = store.encode()?;
        if original.as_deref() != Some(encoded.as_slice()) {
            atomic_write(&store.paths.config_file, &encoded, 0o600)?;
        }
        Ok(store)
    }

    /// A store that lives in memory only: defaults, never reading or writing the configuration
    /// file. Used when [`ConfigStore::open`] failed (unreadable directory, configuration written
    /// by a newer Kebloop) so the launcher still starts without clobbering the file; `reason`
    /// is shown to the user.
    pub fn in_memory(paths: ConfigPaths, home: &str, reason: impl Into<String>) -> Self {
        let mut file = ConfigFile::fresh();
        file.active_profile_id = file.profiles[0].id.clone();
        for profile in &mut file.profiles {
            profile.settings.sanitize();
        }
        file.global.usage.sanitize();
        Self {
            paths,
            home: home.to_string(),
            file,
            recovered_corrupt: None,
            created: false,
            ephemeral: Some(reason.into()),
            writer: None,
        }
    }

    /// Why this store is memory-only, if it is (see [`ConfigStore::in_memory`]).
    pub fn ephemeral_reason(&self) -> Option<&str> {
        self.ephemeral.as_deref()
    }

    /// Route usage-counter saves (launch counts, recents, emoji) through a debounced writer; all
    /// other saves stay synchronous but are serialised with it.
    pub fn set_deferred_writer(&mut self, writer: Arc<DebouncedWriter>) {
        self.writer = Some(writer);
    }

    /// Write any deferred usage update now (call on quit).
    pub fn flush(&self) -> Result<(), ConfigError> {
        self.writer.as_ref().map_or(Ok(()), |w| w.flush())
    }

    /// Path of the corrupt file set aside by [`ConfigStore::open`], if recovery happened.
    pub fn recovered_corrupt_file(&self) -> Option<&Path> {
        self.recovered_corrupt.as_deref()
    }

    /// Whether [`ConfigStore::open`] found no configuration file and created a fresh one (the
    /// first run on this machine). Recovering from a corrupt file is not a first run; see
    /// [`ConfigStore::recovered_corrupt_file`].
    pub fn was_created(&self) -> bool {
        self.created
    }

    /// Home directory used for `~/` expansion.
    pub fn home(&self) -> &str {
        &self.home
    }

    /// Settings of the active profile.
    pub fn settings(&self) -> &PortableSettings {
        &self.file.profiles[self.file.active_index()].settings
    }

    /// Global (non-portable) settings.
    pub fn global(&self) -> &GlobalSettings {
        &self.file.global
    }

    /// All profiles.
    pub fn profiles(&self) -> &[ConfigurationProfile] {
        &self.file.profiles
    }

    /// Id of the active profile.
    pub fn active_profile_id(&self) -> &str {
        &self.file.profiles[self.file.active_index()].id
    }

    /// Name of the active profile.
    pub fn active_profile_name(&self) -> &str {
        &self.file.profiles[self.file.active_index()].name
    }

    fn encode(&self) -> Result<Vec<u8>, ConfigError> {
        Ok(serde_json::to_vec_pretty(&self.file)?)
    }

    fn persist(&self) -> Result<(), ConfigError> {
        if self.ephemeral.is_some() {
            return Ok(());
        }
        let bytes = self.encode()?;
        match &self.writer {
            Some(writer) => writer.write_now(&self.paths.config_file, &bytes, 0o600),
            None => atomic_write(&self.paths.config_file, &bytes, 0o600),
        }
    }

    /// Save without waiting for the disk when a deferred writer is installed.
    fn persist_deferred(&self) -> Result<(), ConfigError> {
        let Some(writer) = self.writer.as_ref().filter(|_| self.ephemeral.is_none()) else {
            return self.persist();
        };
        let bytes = self.encode()?;
        writer.schedule(&self.paths.config_file, 0o600, Box::new(move || Ok(bytes)));
        Ok(())
    }

    /// Run `change` against a copy of the file, persist on success and restore on failure.
    fn transact<R>(
        &mut self,
        change: impl FnOnce(&mut ConfigFile) -> Result<R, ConfigError>,
    ) -> Result<R, ConfigError> {
        self.transact_with(false, change)
    }

    fn transact_with<R>(
        &mut self,
        deferred: bool,
        change: impl FnOnce(&mut ConfigFile) -> Result<R, ConfigError>,
    ) -> Result<R, ConfigError> {
        let snapshot = self.file.clone();
        let outcome = change(&mut self.file).and_then(|value| {
            if deferred {
                self.persist_deferred()
            } else {
                self.persist()
            }
            .map(|()| value)
        });
        if outcome.is_err() {
            self.file = snapshot;
        }
        outcome
    }

    /// Edit the active profile's settings. The result must validate (unless it already did not
    /// before the edit) and is written atomically; on any error nothing changes.
    pub fn update<R, E: std::error::Error + 'static>(
        &mut self,
        change: impl FnOnce(&mut PortableSettings) -> Result<R, E>,
    ) -> Result<R, UpdateError<E>> {
        let mut draft = self.settings().clone();
        let was_valid = draft.validate().is_ok();
        let value = change(&mut draft).map_err(UpdateError::Rejected)?;
        if was_valid {
            draft.validate()?;
        }
        self.transact(|file| {
            let index = file.active_index();
            file.profiles[index].settings = draft;
            Ok(())
        })?;
        Ok(value)
    }

    /// Edit the global settings.
    pub fn update_global(
        &mut self,
        change: impl FnOnce(&mut GlobalSettings),
    ) -> Result<(), ConfigError> {
        self.transact(|file| {
            change(&mut file.global);
            file.global.usage.sanitize();
            Ok(())
        })
    }

    /// Choose the AI handoff provider.
    pub fn set_ai_provider(&mut self, provider: AiProvider) -> Result<(), ConfigError> {
        self.update_global(|g| g.ai_provider = provider)
    }

    /// Enable accessibility snippet expansion on this machine, independently of profiles.
    pub fn set_snippet_expansion_enabled(&mut self, enabled: bool) -> Result<(), ConfigError> {
        self.update_global(|g| g.snippet_keyword_expansion_enabled = enabled)
    }

    fn update_usage(
        &mut self,
        change: impl FnOnce(&mut GlobalSettings),
    ) -> Result<(), ConfigError> {
        self.transact_with(true, |file| {
            change(&mut file.global);
            file.global.usage.sanitize();
            Ok(())
        })
    }

    /// Count a successful launch of a result.
    pub fn record_launch(&mut self, id: &str) -> Result<(), ConfigError> {
        self.update_usage(|g| g.usage.record_launch(id))
    }

    /// Remember a recently used result.
    pub fn record_recent(&mut self, id: &str) -> Result<(), ConfigError> {
        self.update_usage(|g| g.usage.record_recent(id))
    }

    /// Remember a result as recent and, with `count_launch`, count the launch: one save instead
    /// of two.
    pub fn record_use(&mut self, id: &str, count_launch: bool) -> Result<(), ConfigError> {
        self.update_usage(|g| {
            if count_launch {
                g.usage.record_launch(id);
            }
            g.usage.record_recent(id);
        })
    }

    /// Remember a recently used emoji.
    pub fn record_emoji(&mut self, symbol: &str) -> Result<(), ConfigError> {
        self.update_usage(|g| g.usage.record_emoji(symbol))
    }

    // ----- profiles -----

    fn clean_name(name: &str) -> Result<String, ConfigError> {
        if name.trim().is_empty() || name.chars().count() > 100 {
            return Err(ConfigError::invalid(
                "Enter a profile name up to 100 characters.",
            ));
        }
        Ok(name.trim().to_string())
    }

    /// Create a profile from the active profile's current settings. Returns its id.
    pub fn create_profile(&mut self, name: &str) -> Result<String, ConfigError> {
        let name = Self::clean_name(name)?;
        self.transact(|file| {
            let profile = ConfigurationProfile {
                id: new_id(),
                name,
                settings: file.profiles[file.active_index()].settings.clone(),
            };
            let id = profile.id.clone();
            file.profiles.push(profile);
            Ok(id)
        })
    }

    /// Duplicate a profile. Returns the new id.
    pub fn duplicate_profile(&mut self, id: &str, name: &str) -> Result<String, ConfigError> {
        let name = Self::clean_name(name)?;
        self.transact(|file| {
            let source = file
                .profiles
                .iter()
                .find(|p| p.id == id)
                .ok_or_else(|| ConfigError::invalid("Profile not found."))?;
            let profile = ConfigurationProfile {
                id: new_id(),
                name,
                settings: source.settings.clone(),
            };
            let new = profile.id.clone();
            file.profiles.push(profile);
            Ok(new)
        })
    }

    /// Rename a profile.
    pub fn rename_profile(&mut self, id: &str, name: &str) -> Result<(), ConfigError> {
        let name = Self::clean_name(name)?;
        self.transact(|file| {
            let profile = file
                .profiles
                .iter_mut()
                .find(|p| p.id == id)
                .ok_or_else(|| ConfigError::invalid("Profile not found."))?;
            profile.name = name;
            Ok(())
        })
    }

    /// Delete a profile; at least one always remains. Deleting the active profile switches to
    /// another one first. An unknown id is a silent no-op, as on macOS.
    pub fn delete_profile(&mut self, id: &str) -> Result<(), ConfigError> {
        self.delete_profile_with(id, &mut |_| Ok(()))
    }

    /// [`ConfigStore::delete_profile`] with an activation hook for the implied switch.
    pub fn delete_profile_with(
        &mut self,
        id: &str,
        activate: ActivationHook<'_>,
    ) -> Result<(), ConfigError> {
        if self.file.profiles.len() <= 1 {
            return Err(ConfigError::invalid("Keep at least one profile."));
        }
        if id == self.file.active_profile_id {
            let next = self
                .file
                .profiles
                .iter()
                .find(|p| p.id != id)
                .map(|p| p.id.clone())
                .ok_or_else(|| ConfigError::invalid("Keep at least one profile."))?;
            self.switch_profile_with(&next, activate)?;
        }
        self.transact(|file| {
            file.profiles.retain(|p| p.id != id);
            Ok(())
        })
    }

    /// Make another profile active.
    pub fn switch_profile(&mut self, id: &str) -> Result<(), ConfigError> {
        self.switch_profile_with(id, &mut |_| Ok(()))
    }

    /// Switch profiles, running `activate` with the new settings; an error restores the previous
    /// profile.
    pub fn switch_profile_with(
        &mut self,
        id: &str,
        activate: ActivationHook<'_>,
    ) -> Result<(), ConfigError> {
        if id == self.file.active_profile_id {
            return Ok(());
        }
        let target = self
            .file
            .profiles
            .iter()
            .find(|p| p.id == id)
            .ok_or_else(|| ConfigError::invalid("Profile not found."))?
            .settings
            .clone();
        target.validate()?;
        let snapshot = self.file.clone();
        self.transact(|file| {
            file.active_profile_id = id.to_string();
            Ok(())
        })?;
        if let Err(error) = activate(&target) {
            self.file = snapshot;
            let _ = self.persist();
            return Err(error);
        }
        Ok(())
    }

    // ----- export / import -----

    /// Serialise the portable configuration (all profiles, or just one).
    pub fn export_data(&self, profile_id: Option<&str>) -> Result<Vec<u8>, ConfigError> {
        let configuration = match profile_id {
            Some(id) => {
                let profile = self
                    .file
                    .profiles
                    .iter()
                    .find(|p| p.id == id)
                    .ok_or_else(|| ConfigError::invalid("Profile not found."))?;
                PortableConfiguration {
                    version: 1,
                    settings: profile.settings.clone(),
                    profiles: vec![profile.clone()],
                    active_profile_id: profile.id.clone(),
                }
            }
            None => PortableConfiguration {
                version: 1,
                settings: self.settings().clone(),
                profiles: self.file.profiles.clone(),
                active_profile_id: self.file.active_profile_id.clone(),
            },
        };
        Ok(serde_json::to_vec_pretty(&configuration)?)
    }

    /// Parse and validate an import file and report conflicts and unavailable items.
    pub fn preview(&self, data: &[u8]) -> Result<ConfigurationPreview, ConfigError> {
        if data.len() > MAX_IMPORT_BYTES {
            return Err(ConfigError::invalid("Configuration file is too large."));
        }
        let mut configuration: PortableConfiguration =
            serde_json::from_slice(data).map_err(|e| {
                ConfigError::invalid(format!("The configuration file is malformed: {e}"))
            })?;
        if configuration.version != 1 {
            return Err(ConfigError::invalid(format!(
                "Unsupported configuration version {}.",
                configuration.version
            )));
        }
        configuration.settings.require_automation_review();
        for profile in &mut configuration.profiles {
            profile.settings.require_automation_review();
        }
        configuration.settings.validate()?;
        let ids: BTreeSet<&str> = configuration
            .profiles
            .iter()
            .map(|p| p.id.as_str())
            .collect();
        let profiles_ok = !configuration.profiles.is_empty()
            && ids.len() == configuration.profiles.len()
            && ids.contains(configuration.active_profile_id.as_str())
            && configuration
                .profiles
                .iter()
                .all(|p| !p.id.is_empty() && !p.name.trim().is_empty());
        if !profiles_ok {
            return Err(ConfigError::invalid("Invalid profiles or active profile."));
        }
        for profile in &configuration.profiles {
            profile.settings.validate()?;
        }

        let settings = &configuration.settings;
        let current = self.settings();
        let mut warnings = Vec::new();
        let mut unavailable = Vec::new();
        if !settings.scripts.is_empty() || !settings.extensions.is_empty() {
            warnings.push(
                "Imported scripts and extensions are disabled. Review executable paths and arguments in Settings before enabling. Local code runs with your privileges."
                    .to_string(),
            );
        }
        for (path, alias) in &settings.aliases {
            if current
                .aliases
                .get(path)
                .is_some_and(|existing| existing != alias)
            {
                warnings.push(format!(
                    "Alias conflict for {path}; merge keeps the existing alias."
                ));
            }
        }
        let current_keywords: BTreeSet<String> = current
            .quicklinks
            .iter()
            .map(|l| l.keyword.as_str())
            .chain(current.web_searches.iter().map(|s| s.keyword.as_str()))
            .filter(|k| !k.is_empty())
            .map(str::to_lowercase)
            .collect();
        let incoming_keywords: BTreeSet<&str> = settings
            .quicklinks
            .iter()
            .map(|l| l.keyword.as_str())
            .chain(settings.web_searches.iter().map(|s| s.keyword.as_str()))
            .filter(|k| !k.is_empty())
            .collect();
        for keyword in incoming_keywords {
            if current_keywords.contains(&keyword.to_lowercase()) {
                warnings.push(format!(
                    "Keyword conflict: {keyword}. Merge validates the combined configuration before applying."
                ));
            }
        }
        for link in &settings.quicklinks {
            if current
                .quicklinks
                .iter()
                .any(|c| c.id == link.id && c != link)
            {
                warnings.push(format!(
                    "Quicklink conflict: {}; merge keeps the existing quicklink.",
                    link.name
                ));
            }
        }

        let mut paths: BTreeSet<String> = BTreeSet::new();
        let payloads =
            std::iter::once(settings).chain(configuration.profiles.iter().map(|p| &p.settings));
        for payload in payloads {
            let path_like = |p: &String| p.starts_with('/') || p.starts_with("~/");
            paths.extend(payload.personalization.folders.iter().cloned());
            paths.extend(
                payload
                    .personalization
                    .hidden_apps
                    .iter()
                    .filter(|p| path_like(p))
                    .cloned(),
            );
            paths.extend(
                payload
                    .app_shortcuts
                    .iter()
                    .map(|b| b.app_path.clone())
                    .filter(path_like),
            );
            paths.extend(payload.aliases.keys().filter(|p| path_like(p)).cloned());
            for pin in &payload.pins {
                if (pin.kind == "file" || pin.kind == "application") && path_like(&pin.path) {
                    paths.insert(pin.path.clone());
                }
            }
            for link in &payload.quicklinks {
                if let Some(Destination::File(path)) = link.resolved("", &self.home) {
                    paths.insert(path.to_string_lossy().into_owned());
                }
            }
        }
        for path in paths {
            let expanded = expand_tilde(&path, &self.home);
            match fs::metadata(&expanded) {
                Ok(meta) => {
                    let readable = if meta.is_dir() {
                        fs::read_dir(&expanded).is_ok()
                    } else {
                        fs::File::open(&expanded).is_ok()
                    };
                    if !readable {
                        warnings.push(format!("Access is restricted on this computer: {path}"));
                        unavailable.push(UnavailableItem {
                            kind: UnavailableKind::RestrictedPath,
                            subject: path,
                        });
                    }
                }
                Err(e) if e.kind() == ErrorKind::PermissionDenied => {
                    warnings.push(format!("Access is restricted on this computer: {path}"));
                    unavailable.push(UnavailableItem {
                        kind: UnavailableKind::RestrictedPath,
                        subject: path,
                    });
                }
                Err(_) => {
                    warnings.push(format!("Unavailable on this computer: {path}"));
                    unavailable.push(UnavailableItem {
                        kind: UnavailableKind::MissingPath,
                        subject: path,
                    });
                }
            }
        }
        if !settings.selected_apple_shortcuts.is_empty() {
            warnings.push(
                "Apple Shortcuts are not available on Linux; the imported entries are kept but inactive."
                    .to_string(),
            );
            for shortcut in &settings.selected_apple_shortcuts {
                unavailable.push(UnavailableItem {
                    kind: UnavailableKind::AppleShortcut,
                    subject: shortcut.name.clone(),
                });
            }
        }
        for (owner, label) in unavailable_bindings(settings) {
            let subject = match owner {
                ShortcutOwner::Launcher => format!("launcher shortcut {label}"),
                ShortcutOwner::App(path) => format!("{label} ({path})"),
                ShortcutOwner::Command(id) => format!("{label} ({id})"),
            };
            warnings.push(format!(
                "Shortcut key has no equivalent on Linux and stays inactive: {subject}"
            ));
            unavailable.push(UnavailableItem {
                kind: UnavailableKind::Shortcut,
                subject,
            });
        }
        if !settings.app_shortcuts.is_empty() || !settings.command_shortcuts.is_empty() {
            warnings.push(
                "Shortcut availability is checked when applying; conflicts restore the previous configuration."
                    .to_string(),
            );
        }
        warnings.push(
            "Merge keeps current search and appearance preferences and conflicting identifiers; replace uses the imported values."
                .to_string(),
        );
        let summary = vec![
            format!("{} snippets", settings.snippets.len()),
            format!("{} scripts", settings.scripts.len()),
            format!("{} extensions", settings.extensions.len()),
            format!("{} aliases", settings.aliases.len()),
            format!("{} quicklinks", settings.quicklinks.len()),
            format!(
                "{} shortcuts",
                settings.app_shortcuts.len() + settings.command_shortcuts.len()
            ),
            format!("{} pins", settings.pins.len()),
            format!("{} profiles", configuration.profiles.len()),
        ];
        Ok(ConfigurationPreview {
            configuration,
            summary,
            warnings,
            unavailable,
        })
    }

    /// Apply a previewed import. `Replace` writes a backup of the current configuration first
    /// (to `backup` or `<backup_dir>/<uuid>.json`) and returns its path.
    pub fn apply(
        &mut self,
        preview: &ConfigurationPreview,
        mode: ImportMode,
        backup: Option<&Path>,
    ) -> Result<Option<PathBuf>, ConfigError> {
        self.apply_with(preview, mode, backup, &mut |_| Ok(()))
    }

    /// [`ConfigStore::apply`] with an activation hook; if it fails the previous configuration is
    /// restored and the error returned.
    pub fn apply_with(
        &mut self,
        preview: &ConfigurationPreview,
        mode: ImportMode,
        backup: Option<&Path>,
        activate: ActivationHook<'_>,
    ) -> Result<Option<PathBuf>, ConfigError> {
        // Revalidate even if the preview was constructed programmatically.
        let validated = self.preview(&serde_json::to_vec(&preview.configuration)?)?;
        let incoming = validated.configuration;
        let current = self.settings().clone();
        let next = match mode {
            ImportMode::Replace => incoming.settings.clone(),
            ImportMode::Merge => merge(&current, &incoming.settings),
        };
        next.validate()?;
        let mut backup_path = None;
        if mode == ImportMode::Replace {
            let destination = backup
                .map(Path::to_path_buf)
                .unwrap_or_else(|| self.paths.backup_dir.join(format!("{}.json", new_id())));
            atomic_write(&destination, &self.export_data(None)?, 0o600)?;
            backup_path = Some(destination);
        }
        let snapshot = self.file.clone();
        self.transact(|file| {
            match mode {
                ImportMode::Replace => {
                    file.profiles = incoming.profiles.clone();
                    file.active_profile_id = incoming.active_profile_id.clone();
                }
                ImportMode::Merge => {
                    let existing: BTreeSet<String> =
                        file.profiles.iter().map(|p| p.id.clone()).collect();
                    file.profiles.extend(
                        incoming
                            .profiles
                            .iter()
                            .filter(|p| !existing.contains(&p.id))
                            .cloned(),
                    );
                }
            }
            let index = file.active_index();
            file.profiles[index].settings = next.clone();
            Ok(())
        })?;
        if let Err(error) = activate(&next) {
            self.file = snapshot;
            let _ = self.persist();
            return Err(error);
        }
        Ok(backup_path)
    }
}

fn union<T: Clone>(current: &[T], incoming: &[T], id: impl Fn(&T) -> &str) -> Vec<T> {
    let mut merged = current.to_vec();
    merged.extend(
        incoming
            .iter()
            .filter(|item| !current.iter().any(|c| id(c) == id(item)))
            .cloned(),
    );
    merged
}

/// Merge incoming settings into the current ones: identifiers that already exist keep their
/// current definition and search/appearance preferences are untouched.
pub fn merge(current: &PortableSettings, incoming: &PortableSettings) -> PortableSettings {
    let mut merged = current.clone();
    merged.scripts = union(&current.scripts, &incoming.scripts, |s| &s.id);
    merged.extensions = union(&current.extensions, &incoming.extensions, |e| &e.id);
    merged.snippets = union(&current.snippets, &incoming.snippets, |s| &s.id);
    for (key, value) in &incoming.aliases {
        merged
            .aliases
            .entry(key.clone())
            .or_insert_with(|| value.clone());
    }
    merged.quicklinks = union(&current.quicklinks, &incoming.quicklinks, |l| &l.id);
    merged.web_searches = union(&current.web_searches, &incoming.web_searches, |s| &s.id);
    merged.app_shortcuts = union(&current.app_shortcuts, &incoming.app_shortcuts, |b| &b.id);
    merged.command_shortcuts = union(
        &current.command_shortcuts,
        &incoming.command_shortcuts,
        |b| &b.id,
    );
    merged.selected_apple_shortcuts = union(
        &current.selected_apple_shortcuts,
        &incoming.selected_apple_shortcuts,
        |a| &a.id,
    );
    merged.pins = union(&current.pins, &incoming.pins, |p| &p.id);
    merged
}
