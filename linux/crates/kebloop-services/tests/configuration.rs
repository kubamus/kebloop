//! Ports of ConfigurationTests, SnippetTests (workflow), AutomationConfigurationTests,
//! CommandRegistryTests / UserCommandShortcutTests (binding model), AIHandoffTests (provider
//! persistence), LauncherPersonalizationTests (settings part) and macOS-compatibility checks.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use kebloop_services::ai::AiProvider;
use kebloop_services::aliases;
use kebloop_services::automation::{
    AutomationError, AutomationRunner, ExtensionManifest, ExtensionPreference, ScriptCommand,
    install_extension, run_script, save_script, set_extension_enabled, set_extension_preference,
    set_script_enabled,
};
use kebloop_services::config::{
    ConfigPaths, ConfigStore, ImportMode, PortableConfiguration, PortableSettings, UnavailableKind,
    UpdateError,
};
use kebloop_services::error::ConfigError;
use kebloop_services::personalization::{PinnedResult, toggle_pin};
use kebloop_services::quicklinks::{
    Quicklink, WebSearchTemplate, resolve_query, save_quicklink, save_web_search,
};
use kebloop_services::shortcuts::{
    ShortcutError, ShortcutOwner, assign_app_shortcut, assign_command_shortcut,
    assign_launcher_shortcut, choose_launcher_shortcut, remove_app_shortcut,
    remove_command_shortcut,
};
use kebloop_services::snippets::{Snippet, remove_snippet, save_snippet};
use serde_json::json;

const HOME: &str = "/home/test";

fn paths(dir: &Path) -> ConfigPaths {
    ConfigPaths::in_dir(&dir.join("config"))
}

fn open(dir: &Path) -> ConfigStore {
    ConfigStore::open(paths(dir), HOME).unwrap()
}

#[test]
fn snippet_expansion_consent_is_local_and_cannot_be_enabled_by_import() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(dir.path());
    assert!(!store.global().snippet_keyword_expansion_enabled);
    store.set_snippet_expansion_enabled(true).unwrap();
    assert!(open(dir.path()).global().snippet_keyword_expansion_enabled);
    let mut exported: serde_json::Value =
        serde_json::from_slice(&store.export_data(None).unwrap()).unwrap();
    assert!(
        !exported
            .to_string()
            .contains("snippetKeywordExpansionEnabled")
    );
    store.set_snippet_expansion_enabled(false).unwrap();
    exported["settings"]["snippetKeywordExpansionEnabled"] = json!(true);
    let preview = store
        .preview(&serde_json::to_vec(&exported).unwrap())
        .unwrap();
    store.apply(&preview, ImportMode::Merge, None).unwrap();
    assert!(!store.global().snippet_keyword_expansion_enabled);
    assert!(!open(dir.path()).global().snippet_keyword_expansion_enabled);
}

fn set_alias(store: &mut ConfigStore, key: &str, alias: &str) {
    store
        .update(|s| aliases::save_alias(&mut s.aliases, key, alias))
        .unwrap();
}

fn always(_: &ShortcutOwner) -> bool {
    true
}

// ----- ConfigurationTests -----

#[test]
fn folder_boundaries_pins_and_persistence() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(dir.path());
    store
        .update(|s| -> Result<(), ConfigError> {
            s.personalization.folders = vec!["/Users/example/Documents".into()];
            s.personalization.exclusions = vec!["/Users/example/Documents/Private".into()];
            Ok(())
        })
        .unwrap();
    let p = &store.settings().personalization;
    assert!(p.allows_file("/Users/example/Documents/report.txt", HOME));
    assert!(!p.allows_file("/Users/example/Documents-old/report.txt", HOME));
    assert!(!p.allows_file("/Users/example/Documents/Private/report.txt", HOME));
    assert!(p.allows_file("/Users/example/Documents/Privateer/report.txt", HOME));

    let pin = PinnedResult {
        id: "file:/Users/example/Documents/report.txt".into(),
        name: "Report".into(),
        path: "/Users/example/Documents/report.txt".into(),
        kind: "file".into(),
    };
    store
        .update(|s| -> Result<bool, ConfigError> { Ok(toggle_pin(&mut s.pins, pin.clone())) })
        .unwrap();
    assert_eq!(open(dir.path()).settings().pins.len(), 1);
    assert_eq!(open(dir.path()).settings().pins[0], pin);
    store
        .update(|s| -> Result<bool, ConfigError> { Ok(toggle_pin(&mut s.pins, pin.clone())) })
        .unwrap();
    assert!(open(dir.path()).settings().pins.is_empty());
    // Persisted preferences survive a relaunch.
    store
        .update(|s| -> Result<(), ConfigError> {
            s.personalization.appearance = "dark".into();
            Ok(())
        })
        .unwrap();
    assert_eq!(
        open(dir.path()).settings().personalization.appearance,
        "dark"
    );
    assert_eq!(
        open(dir.path()).settings().personalization.folders,
        ["/Users/example/Documents"]
    );
}

#[test]
fn invalid_preferences_are_rejected_without_changing_anything() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(dir.path());
    let before = fs::read(store_path(dir.path())).unwrap();
    let result = store.update(|s| -> Result<(), ConfigError> {
        s.personalization.text_size = 99.0;
        Ok(())
    });
    assert!(matches!(
        result,
        Err(UpdateError::Config(ConfigError::Invalid(_)))
    ));
    assert_eq!(store.settings().personalization.text_size, 14.0);
    assert_eq!(fs::read(store_path(dir.path())).unwrap(), before);
}

fn store_path(dir: &Path) -> PathBuf {
    paths(dir).config_file
}

#[test]
fn round_trip_privacy_exclusion_and_replace_backup() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(dir.path());
    set_alias(&mut store, "test.desktop", "test");
    store.record_launch("secret-launch-id").unwrap();
    store.set_ai_provider(AiProvider::Claude).unwrap();
    let exported = store.export_data(None).unwrap();
    let text = String::from_utf8(exported.clone()).unwrap();
    for private in [
        "secret-launch-id",
        "aiProvider",
        "usage",
        "recentResults",
        "global",
    ] {
        assert!(!text.contains(private), "export leaked {private}");
    }
    set_alias(&mut store, "test.desktop", "");
    set_alias(&mut store, "other.desktop", "other");
    let backup = dir.path().join("backup.json");
    let preview = store.preview(&exported).unwrap();
    let written = store
        .apply(&preview, ImportMode::Replace, Some(&backup))
        .unwrap();
    assert_eq!(written.as_deref(), Some(backup.as_path()));
    assert_eq!(store.settings().aliases.len(), 1);
    assert_eq!(store.settings().aliases["test.desktop"], "test");
    let saved: PortableConfiguration = serde_json::from_slice(&fs::read(&backup).unwrap()).unwrap();
    assert_eq!(saved.settings.aliases.len(), 1);
    assert_eq!(saved.settings.aliases["other.desktop"], "other");
    // Global data is never replaced.
    assert_eq!(store.global().usage.launch_count("secret-launch-id"), 1);
    assert_eq!(store.global().ai_provider, AiProvider::Claude);
    assert_eq!(
        fs::metadata(&backup).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn replace_without_explicit_backup_writes_into_the_backup_directory() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(dir.path());
    let exported = store.export_data(None).unwrap();
    let preview = store.preview(&exported).unwrap();
    let backup = store
        .apply(&preview, ImportMode::Replace, None)
        .unwrap()
        .unwrap();
    assert!(backup.starts_with(dir.path().join("config/backups")));
    assert!(backup.exists());
}

#[test]
fn validation_and_activation_failure_do_not_partially_write() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(dir.path());
    set_alias(&mut store, "original.desktop", "original");
    let mut configuration: PortableConfiguration =
        serde_json::from_slice(&store.export_data(None).unwrap()).unwrap();
    configuration.version = 99;
    assert!(
        store
            .preview(&serde_json::to_vec(&configuration).unwrap())
            .is_err()
    );
    configuration.version = 1;
    configuration.settings.aliases = [("new.desktop".to_string(), "new".to_string())].into();
    let preview = store
        .preview(&serde_json::to_vec(&configuration).unwrap())
        .unwrap();
    let before = fs::read(store_path(dir.path())).unwrap();
    let error = store
        .apply_with(&preview, ImportMode::Merge, None, &mut |_| {
            Err(ConfigError::invalid("Shortcut unavailable"))
        })
        .unwrap_err();
    assert!(error.to_string().contains("Shortcut unavailable"));
    assert_eq!(
        store.settings().aliases.keys().collect::<Vec<_>>(),
        ["original.desktop"]
    );
    assert_eq!(fs::read(store_path(dir.path())).unwrap(), before);
    assert!(store.preview(b"{}").is_err());
    assert!(store.preview(b"not json").is_err());
    assert!(store.preview(&vec![b' '; 5_000_001]).is_err());
}

#[test]
fn profile_snapshots_switch_and_rollback() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(dir.path());
    set_alias(&mut store, "personal.desktop", "personal");
    let personal = store.active_profile_id().to_string();
    assert_eq!(store.active_profile_name(), "Personal");
    let work = store.create_profile("  Work  ").unwrap();
    assert_eq!(
        store.profiles().iter().find(|p| p.id == work).unwrap().name,
        "Work"
    );
    store.switch_profile(&work).unwrap();
    set_alias(&mut store, "personal.desktop", "");
    set_alias(&mut store, "work.desktop", "work");
    let err = store
        .switch_profile_with(&personal, &mut |_| {
            Err(ConfigError::invalid("Unavailable shortcut"))
        })
        .unwrap_err();
    assert!(err.to_string().contains("Unavailable shortcut"));
    assert_eq!(store.active_profile_id(), work);
    assert_eq!(
        store.settings().aliases.keys().collect::<Vec<_>>(),
        ["work.desktop"]
    );
    assert_eq!(open(dir.path()).active_profile_id(), work);
    store.switch_profile(&personal).unwrap();
    assert_eq!(
        store.settings().aliases.keys().collect::<Vec<_>>(),
        ["personal.desktop"]
    );
    // Deleting an unknown profile is a silent no-op, as on macOS.
    let before = store.profiles().len();
    store.delete_profile("missing").unwrap();
    assert_eq!(store.profiles().len(), before);
    store.delete_profile(&work).unwrap();
    assert!(store.delete_profile(&personal).is_err());
    // The "keep one profile" rule is checked before the id, so even an unknown id errors.
    assert!(store.delete_profile("missing").is_err());
    assert_eq!(store.profiles().len(), 1);
    assert!(store.switch_profile("missing").is_err());
}

#[test]
fn profile_management_duplicate_rename_delete_active() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(dir.path());
    set_alias(&mut store, "a.desktop", "alpha");
    let first = store.active_profile_id().to_string();
    let copy = store.duplicate_profile(&first, "Copy").unwrap();
    store.rename_profile(&copy, " Renamed ").unwrap();
    assert_eq!(
        open(dir.path())
            .profiles()
            .iter()
            .find(|p| p.id == copy)
            .unwrap()
            .name,
        "Renamed"
    );
    assert!(store.rename_profile(&copy, "   ").is_err());
    assert!(store.create_profile(&"x".repeat(101)).is_err());
    assert!(store.duplicate_profile("missing", "Name").is_err());
    store.switch_profile(&copy).unwrap();
    assert_eq!(store.settings().aliases["a.desktop"], "alpha");
    // Deleting the active profile switches to the remaining one first.
    store.delete_profile(&copy).unwrap();
    assert_eq!(store.active_profile_id(), first);
    assert_eq!(store.profiles().len(), 1);
}

#[test]
fn portable_validation_rejects_templates_keywords_and_unknown_commands() {
    let mut settings = PortableSettings::default();
    assert!(settings.validate().is_ok());
    settings.web_searches = vec![WebSearchTemplate {
        id: "bad".into(),
        name: "Bad".into(),
        keyword: "web".into(),
        target: "https://{query}.example.com".into(),
    }];
    assert!(settings.validate().is_err());
    settings.web_searches = vec![WebSearchTemplate {
        id: "web".into(),
        name: "Web".into(),
        keyword: "gh".into(),
        target: "https://example.com/?q={query}".into(),
    }];
    settings.quicklinks = vec![Quicklink {
        id: "gh".into(),
        name: "GitHub".into(),
        target: "https://github.com".into(),
        keyword: "GH".into(),
    }];
    assert!(settings.validate().is_err());
    settings.quicklinks.clear();
    assert!(settings.validate().is_ok());
    let rejected = assign_command_shortcut(&mut settings, "unknown", "<Alt>h", "Alt+H", &always);
    assert!(rejected.is_ok());
    assert!(
        settings.validate().is_err(),
        "unknown command id must not validate"
    );
    settings.command_shortcuts.clear();
    settings.selected_apple_shortcuts = vec![kebloop_services::config::AppleShortcut {
        id: "not-a-uuid".into(),
        name: "Broken".into(),
    }];
    assert!(settings.validate().is_err());
    settings.selected_apple_shortcuts.clear();
    settings.launcher_shortcut_id = "custom".into();
    assert!(settings.validate().is_err(), "custom without definition");
    settings.launcher_shortcut_id = "nonsense".into();
    assert!(settings.validate().is_err());
}

#[test]
fn global_and_per_profile_settings_are_separate() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(dir.path());
    store.record_launch("app:firefox").unwrap();
    store.record_launch("app:firefox").unwrap();
    store.record_recent("app:firefox").unwrap();
    store.record_emoji("😀").unwrap();
    let second = store.create_profile("Second").unwrap();
    store.switch_profile(&second).unwrap();
    assert_eq!(store.global().usage.launch_count("app:firefox"), 2);
    let reopened = open(dir.path());
    assert_eq!(reopened.global().usage.recent_results, ["app:firefox"]);
    assert_eq!(reopened.global().usage.emoji_recents, ["😀"]);
    assert_eq!(reopened.global().usage.most_used(5), ["app:firefox"]);
}

#[test]
fn corrupt_config_is_set_aside_and_defaults_restored() {
    let dir = tempfile::tempdir().unwrap();
    let config = paths(dir.path());
    fs::create_dir_all(config.config_file.parent().unwrap()).unwrap();
    fs::write(&config.config_file, "{ this is not json").unwrap();
    let store = ConfigStore::open(config.clone(), HOME).unwrap();
    let aside = store.recovered_corrupt_file().unwrap();
    assert_eq!(fs::read_to_string(aside).unwrap(), "{ this is not json");
    assert_eq!(store.profiles().len(), 1);
    assert_eq!(store.active_profile_name(), "Personal");
    assert_eq!(
        fs::metadata(&config.config_file)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    // A newer schema is refused rather than overwritten.
    fs::write(
        &config.config_file,
        r#"{"version":2,"activeProfileID":"x","profiles":[]}"#,
    )
    .unwrap();
    assert!(ConfigStore::open(config.clone(), HOME).is_err());
    assert!(
        fs::read_to_string(&config.config_file)
            .unwrap()
            .contains("\"version\":2")
    );
}

#[test]
fn missing_active_profile_and_invalid_stored_data_are_repaired_on_open() {
    let dir = tempfile::tempdir().unwrap();
    let config = paths(dir.path());
    fs::create_dir_all(config.config_file.parent().unwrap()).unwrap();
    let doc = json!({
        "version": 1, "activeProfileID": "gone",
        "profiles": [{"id": "P", "name": "Only", "settings": {
            "aliases": {"a": "Term", "b": "term", "c": ""},
            "quicklinks": [{"id":"ok","name":"Ok","target":"https://example.com"}, {"id":"bad","name":"Bad","target":"javascript:alert(1)"}],
            "scripts": [{"id":"s","title":"S","executable":"relative","arguments":[],"timeout":10,"enabled":true}]
        }}]
    });
    fs::write(&config.config_file, doc.to_string()).unwrap();
    let store = ConfigStore::open(config, HOME).unwrap();
    assert_eq!(store.active_profile_id(), "P");
    let settings = store.settings();
    assert_eq!(settings.aliases.len(), 1);
    assert_eq!(settings.quicklinks.len(), 1);
    assert!(settings.scripts.is_empty());
    assert_eq!(settings.web_searches.len(), 1, "defaults fill missing keys");
}

// ----- macOS compatibility -----

fn macos_export() -> serde_json::Value {
    let settings = json!({
        "launcherShortcutID": "custom",
        "launcherCustomShortcut": {"id": "custom", "label": "⌥⌘S", "modifiers": 2304, "keyCode": 1},
        "aliases": {"/Applications/Safari.app": "web browser"},
        "quicklinks": [{"id": "q1", "name": "GitHub", "target": "https://github.com", "keyword": "gh"},
                       {"id": "q2", "name": "Missing", "target": "/definitely/not/here", "keyword": ""}],
        "webSearches": [{"id": "ddg2", "name": "Duck", "keyword": "web", "target": "https://duckduckgo.com/?q={query}"}],
        "appShortcuts": [{"id": "/Applications/Safari.app", "appPath": "/Applications/Safari.app", "appName": "Safari",
                          "keyCode": 0, "modifiers": 2304, "label": "⌥⌘A"}],
        "commandShortcuts": [{"id": "emoji", "keyCode": 10, "modifiers": 2304, "label": "⌥⌘§"},
                             {"id": "apple:6A5C1A3E-0F5A-4F6A-9C2B-1D2E3F4A5B6C", "keyCode": 11, "modifiers": 2304, "label": "⌥⌘B"}],
        "selectedAppleShortcuts": [{"id": "6A5C1A3E-0F5A-4F6A-9C2B-1D2E3F4A5B6C", "name": "Daily summary"}],
        "personalization": {"appsEnabled": true, "filesEnabled": true, "linksEnabled": true, "commandsEnabled": true,
            "folders": ["/Users/me/Documents"], "exclusions": [], "hiddenApps": [], "emptyMode": "frequent",
            "appearance": "system", "textSize": 14, "panelWidth": 560, "compact": false, "transparency": true,
            "showShortcutHints": true, "placement": "pointer", "displayID": "", "futureMacOnlyFlag": {"nested": [1, 2]}},
        "pins": [{"id": "app:/Applications/Safari.app", "name": "Safari", "path": "/Applications/Safari.app", "kind": "application"}],
        "snippets": [{"id": "sn", "name": "Greeting", "template": "Hello {name}"}],
        "scripts": [{"id": "sc", "title": "Script", "description": "", "executable": "/usr/bin/true", "arguments": [], "timeout": 10, "enabled": true}],
        "extensions": [{"id": "ex", "name": "Ext", "description": "", "apiVersion": 1, "executable": "/usr/bin/true", "arguments": [], "preferences": [], "enabled": true}],
        "someFutureSection": {"keep": true}
    });
    json!({
        "version": 1,
        "settings": settings,
        "profiles": [{"id": "P1", "name": "Personal", "settings": settings}],
        "activeProfileID": "P1"
    })
}

#[test]
fn macos_export_previews_with_unavailable_items_and_never_crashes() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(dir.path());
    let preview = store
        .preview(macos_export().to_string().as_bytes())
        .unwrap();
    assert!(preview.warnings.iter().any(|w| w.contains("disabled")));
    assert!(
        preview
            .warnings
            .iter()
            .any(|w| w.contains("Keyword conflict: web"))
    );
    let has = |kind: UnavailableKind, needle: &str| {
        preview
            .unavailable
            .iter()
            .any(|u| u.kind == kind && u.subject.contains(needle))
    };
    assert!(has(UnavailableKind::AppleShortcut, "Daily summary"));
    assert!(has(
        UnavailableKind::MissingPath,
        "/Applications/Safari.app"
    ));
    assert!(has(UnavailableKind::MissingPath, "/Users/me/Documents"));
    assert!(has(UnavailableKind::MissingPath, "/definitely/not/here"));
    assert!(
        has(UnavailableKind::Shortcut, "⌥⌘§"),
        "unmapped key code is reported"
    );
    assert!(
        !has(UnavailableKind::Shortcut, "⌥⌘A"),
        "key code 0 maps to <Alt><Super>a"
    );
    assert_eq!(preview.summary[0], "1 snippets");
    assert!(
        preview
            .configuration
            .settings
            .scripts
            .iter()
            .all(|s| !s.enabled)
    );
    assert!(
        preview.configuration.profiles[0]
            .settings
            .extensions
            .iter()
            .all(|e| !e.enabled)
    );
}

#[test]
fn macos_import_round_trips_unknown_fields_and_carbon_codes() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(dir.path());
    let preview = store
        .preview(macos_export().to_string().as_bytes())
        .unwrap();
    store
        .apply(
            &preview,
            ImportMode::Replace,
            Some(&dir.path().join("b.json")),
        )
        .unwrap();
    assert_eq!(store.active_profile_id(), "P1");
    let exported: serde_json::Value =
        serde_json::from_slice(&store.export_data(None).unwrap()).unwrap();
    let settings = &exported["settings"];
    assert_eq!(settings["someFutureSection"]["keep"], true);
    assert_eq!(
        settings["personalization"]["futureMacOnlyFlag"]["nested"][1],
        2
    );
    assert_eq!(settings["launcherCustomShortcut"]["keyCode"], 1);
    assert_eq!(settings["launcherCustomShortcut"]["modifiers"], 2304);
    assert_eq!(settings["commandShortcuts"][0]["keyCode"], 10);
    assert_eq!(
        settings["selectedAppleShortcuts"][0]["name"],
        "Daily summary"
    );
    assert_eq!(exported["activeProfileID"], "P1");
    assert_eq!(exported["version"], 1);
    // Required macOS keys are always present.
    for key in [
        "launcherShortcutID",
        "aliases",
        "quicklinks",
        "webSearches",
        "appShortcuts",
        "commandShortcuts",
        "selectedAppleShortcuts",
        "personalization",
        "pins",
        "snippets",
        "scripts",
        "extensions",
    ] {
        assert!(settings.get(key).is_some(), "missing {key}");
    }
    // The imported shortcuts resolve to Linux accelerators where possible.
    let app = &store.settings().app_shortcuts[0];
    assert_eq!(app.resolved().unwrap().to_string(), "<Alt><Super>a");
    assert!(store.settings().command_shortcuts[0].resolved().is_none());
    assert_eq!(
        store.settings().launcher_accelerator().to_string(),
        "<Alt><Super>s"
    );
    // Reopening keeps everything, including the extras.
    let reopened = open(dir.path());
    assert_eq!(reopened.settings().extra["someFutureSection"]["keep"], true);
}

#[test]
fn linux_created_shortcuts_carry_carbon_codes_for_macos() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(dir.path());
    store
        .update(|s| assign_command_shortcut(s, "emoji", "<Alt>e", "Alt+E", &always))
        .unwrap();
    let exported: serde_json::Value =
        serde_json::from_slice(&store.export_data(None).unwrap()).unwrap();
    let binding = &exported["settings"]["commandShortcuts"][0];
    assert_eq!(binding["keyCode"], 14);
    assert_eq!(binding["modifiers"], 2048);
    assert_eq!(binding["accelerator"], "<Alt>e");
}

#[test]
fn merge_reports_conflicts_and_keeps_current_values() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(dir.path());
    set_alias(&mut store, "firefox.desktop", "browser");
    store
        .update(|s| {
            save_quicklink(
                &mut s.quicklinks,
                &s.web_searches.clone(),
                Some("gh"),
                "GitHub",
                "https://github.com",
                "gh",
                HOME,
            )
        })
        .unwrap();
    let mut incoming: PortableConfiguration =
        serde_json::from_slice(&store.export_data(None).unwrap()).unwrap();
    incoming
        .settings
        .aliases
        .insert("firefox.desktop".into(), "web".into());
    incoming.settings.quicklinks[0].name = "Changed".into();
    incoming.settings.personalization.appearance = "dark".into();
    incoming.settings.snippets.push(Snippet {
        id: "new".into(),
        name: "New".into(),
        template: "text".into(),
        keyword: None,
    });
    incoming.profiles[0].settings = incoming.settings.clone();
    let preview = store
        .preview(&serde_json::to_vec(&incoming).unwrap())
        .unwrap();
    assert!(
        preview
            .warnings
            .iter()
            .any(|w| w.contains("Alias conflict for firefox.desktop"))
    );
    assert!(
        preview
            .warnings
            .iter()
            .any(|w| w.contains("Quicklink conflict: Changed"))
    );
    assert!(
        preview
            .warnings
            .iter()
            .any(|w| w.contains("Keyword conflict: gh"))
    );
    store.apply(&preview, ImportMode::Merge, None).unwrap();
    let settings = store.settings();
    assert_eq!(settings.aliases["firefox.desktop"], "browser");
    assert_eq!(settings.quicklinks[0].name, "GitHub");
    assert_eq!(settings.personalization.appearance, "system");
    assert_eq!(settings.snippets.len(), 1);
    assert_eq!(
        store.profiles().len(),
        1,
        "same profile id is not duplicated"
    );
}

// ----- SnippetTests (workflow) -----

#[test]
fn snippet_editing_merge_replace_and_profiles() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(dir.path());
    let work_id = store
        .update(|s| save_snippet(&mut s.snippets, None, "Work", "Work {name}"))
        .unwrap();
    store
        .update(|s| save_snippet(&mut s.snippets, Some(&work_id), "Edited", "Edited {name}"))
        .unwrap();
    assert_eq!(store.settings().snippets.len(), 1);
    assert_eq!(store.settings().snippets[0].name, "Edited");
    for (name, text) in [
        (" ", "valid".to_string()),
        ("Valid", String::new()),
        ("Valid", "x".repeat(100_001)),
    ] {
        assert!(
            store
                .update(|s| save_snippet(&mut s.snippets, None, name, &text))
                .is_err()
        );
    }
    assert_eq!(store.settings().snippets.len(), 1);

    let work_profile = store.active_profile_id().to_string();
    let personal_profile = store.create_profile("Personal snippets").unwrap();
    store.switch_profile(&personal_profile).unwrap();
    store
        .update(|s| -> Result<(), ConfigError> {
            remove_snippet(&mut s.snippets, &work_id);
            save_snippet(&mut s.snippets, None, "Personal", "Personal text").map(|_| ())
        })
        .unwrap();
    let personal = store.settings().snippets.clone();
    store.switch_profile(&work_profile).unwrap();
    assert_eq!(
        store
            .settings()
            .snippets
            .iter()
            .map(|s| s.id.as_str())
            .collect::<Vec<_>>(),
        [work_id.as_str()]
    );
    store.switch_profile(&personal_profile).unwrap();
    assert_eq!(store.settings().snippets, personal);

    let personal_export = store.export_data(None).unwrap();
    store
        .update(|s| -> Result<(), ConfigError> {
            remove_snippet(&mut s.snippets, &personal[0].id);
            save_snippet(&mut s.snippets, None, "Other", "Other text").map(|_| ())
        })
        .unwrap();
    let preview = store.preview(&personal_export).unwrap();
    store.apply(&preview, ImportMode::Merge, None).unwrap();
    let mut names: Vec<_> = store
        .settings()
        .snippets
        .iter()
        .map(|s| s.name.clone())
        .collect();
    names.sort();
    assert_eq!(names, ["Other", "Personal"]);
    let backup = dir.path().join("SnippetBackup.json");
    store
        .apply(&preview, ImportMode::Replace, Some(&backup))
        .unwrap();
    assert_eq!(store.settings().snippets, personal);
    assert!(backup.exists());
}

#[test]
fn snippet_persistence_configuration_and_legacy_migration() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(dir.path());
    store
        .update(|s| save_snippet(&mut s.snippets, None, "Greeting", "Hello {name}"))
        .unwrap();
    assert_eq!(
        open(dir.path()).settings().snippets,
        store.settings().snippets
    );
    let data = store.export_data(None).unwrap();
    assert_eq!(
        store
            .preview(&data)
            .unwrap()
            .configuration
            .settings
            .snippets,
        store.settings().snippets
    );
    let mut json: serde_json::Value = serde_json::from_slice(&data).unwrap();
    for key in ["snippets", "scripts", "extensions"] {
        json["settings"].as_object_mut().unwrap().remove(key);
    }
    store.preview(json.to_string().as_bytes()).unwrap();
    let id = store.settings().snippets[0].id.clone();
    store
        .update(|s| -> Result<(), ConfigError> {
            remove_snippet(&mut s.snippets, &id);
            Ok(())
        })
        .unwrap();
    assert!(open(dir.path()).settings().snippets.is_empty());
}

// ----- QuicklinkTests (persistence) -----

#[test]
fn quicklinks_persist_edit_and_remove_through_the_store() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(dir.path());
    assert!(
        store
            .update(|s| save_quicklink(
                &mut s.quicklinks,
                &s.web_searches.clone(),
                None,
                "  ",
                "https://example.com",
                "",
                HOME
            ))
            .is_err()
    );
    assert!(
        store
            .update(|s| save_quicklink(
                &mut s.quicklinks,
                &s.web_searches.clone(),
                None,
                "Bad",
                "javascript:alert(1)",
                "",
                HOME
            ))
            .is_err()
    );
    assert!(store.settings().quicklinks.is_empty());
    let id = store
        .update(|s| {
            save_quicklink(
                &mut s.quicklinks,
                &s.web_searches.clone(),
                None,
                " Docs ",
                " https://example.com/docs ",
                "",
                HOME,
            )
        })
        .unwrap();
    assert_eq!(open(dir.path()).settings().quicklinks[0].id, id);
    store
        .update(|s| {
            save_quicklink(
                &mut s.quicklinks,
                &s.web_searches.clone(),
                Some(&id),
                "Documents",
                "~/Documents",
                "",
                HOME,
            )
        })
        .unwrap();
    assert_eq!(
        open(dir.path()).settings().quicklinks[0].target,
        "~/Documents"
    );
    store
        .update(|s| -> Result<(), ConfigError> {
            s.quicklinks.clear();
            Ok(())
        })
        .unwrap();
    assert!(open(dir.path()).settings().quicklinks.is_empty());
}

#[test]
fn keyword_resolution_through_the_store() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(dir.path());
    store
        .update(|s| {
            save_quicklink(
                &mut s.quicklinks,
                &s.web_searches.clone(),
                None,
                "GitHub",
                "https://github.com/search?q={query}",
                "gh",
                HOME,
            )
        })
        .unwrap();
    assert!(
        store
            .update(|s| save_web_search(
                &s.quicklinks.clone(),
                &mut s.web_searches,
                None,
                "Conflict",
                "gh",
                "https://example.com/?q={query}",
                HOME
            ))
            .is_err()
    );
    let settings = store.settings();
    let resolved = resolve_query(
        &settings.quicklinks,
        &settings.web_searches,
        "gh hello world",
    )
    .unwrap();
    assert_eq!(resolved.argument, "hello world");
    assert_eq!(open(dir.path()).settings().quicklinks[0].keyword, "gh");
}

// ----- AutomationConfigurationTests -----

#[test]
fn portable_import_disables_scripts_and_extensions_across_profiles_and_excludes_preferences() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(dir.path());
    let mut manifest = ExtensionManifest {
        id: "two".into(),
        name: "Two".into(),
        description: String::new(),
        api_version: 1,
        executable: "/usr/bin/true".into(),
        arguments: vec![],
        preferences: vec![ExtensionPreference {
            id: "text".into(),
            title: "Text".into(),
            default_value: "Default".into(),
        }],
        enabled: false,
    };
    store
        .update(|s| -> Result<(), AutomationError> {
            let mut one = ScriptCommand::new("one", "One", "/usr/bin/true", vec![]);
            one.enabled = true;
            save_script(&mut s.scripts, one)?;
            install_extension(&mut s.extensions, manifest.clone())?;
            set_extension_enabled(&mut s.extensions, "two", true);
            Ok(())
        })
        .unwrap();
    manifest = store.settings().extensions[0].clone();
    store
        .update_global(|g| {
            set_extension_preference(
                &[manifest.clone()],
                &mut g.extension_preferences,
                "two",
                "text",
                "PRIVATE-PREFERENCE-VALUE",
            );
        })
        .unwrap();
    let data = store.export_data(None).unwrap();
    assert!(!String::from_utf8_lossy(&data).contains("PRIVATE-PREFERENCE-VALUE"));
    let preview = store.preview(&data).unwrap();
    assert!(preview.warnings.iter().any(|w| w.contains("disabled")));
    store
        .update(|s| -> Result<(), ConfigError> {
            s.scripts.clear();
            s.extensions.clear();
            Ok(())
        })
        .unwrap();
    store.apply(&preview, ImportMode::Merge, None).unwrap();
    assert!(!open(dir.path()).settings().scripts[0].enabled);
    assert!(!open(dir.path()).settings().extensions[0].enabled);
    let exported: PortableConfiguration =
        serde_json::from_slice(&store.export_data(None).unwrap()).unwrap();
    assert!(
        exported
            .profiles
            .iter()
            .all(|p| p.settings.scripts.iter().all(|s| !s.enabled)
                && p.settings.extensions.iter().all(|e| !e.enabled))
    );
    // The preference value survived in the global section.
    assert_eq!(
        store.global().extension_preferences["two"]["text"],
        "PRIVATE-PREFERENCE-VALUE"
    );
}

#[test]
fn replace_import_and_profile_switch_require_review_without_executing_code() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("must-not-execute");
    let mut store = open(dir.path());
    store
        .update(|s| -> Result<(), AutomationError> {
            let mut touch = ScriptCommand::new(
                "touch",
                "Never run on import",
                "/usr/bin/touch",
                vec![marker.to_string_lossy().into_owned()],
            );
            touch.enabled = true;
            save_script(&mut s.scripts, touch)
        })
        .unwrap();
    store.create_profile("Second").unwrap();
    let data = store.export_data(None).unwrap();
    let preview = store.preview(&data).unwrap();
    assert_eq!(preview.configuration.profiles.len(), 2);
    assert!(
        preview.configuration.profiles.iter().all(|p| p
            .settings
            .scripts
            .iter()
            .all(|s| !s.enabled))
    );
    store
        .apply(
            &preview,
            ImportMode::Replace,
            Some(&dir.path().join("backup.json")),
        )
        .unwrap();
    let ids: Vec<_> = store.profiles().iter().map(|p| p.id.clone()).collect();
    for id in ids {
        store.switch_profile(&id).unwrap();
        assert!(!store.settings().scripts[0].enabled);
        assert_eq!(
            run_script(
                &store.settings().scripts,
                "touch",
                &[],
                &AutomationRunner::new()
            ),
            Err(AutomationError::NotEnabled("script"))
        );
    }
    assert!(!marker.exists());
    assert!(dir.path().join("backup.json").exists());
}

#[test]
fn malformed_automation_import_does_not_partially_change_settings() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(dir.path());
    store
        .update(|s| -> Result<(), AutomationError> {
            let mut existing = ScriptCommand::new("existing", "Existing", "/usr/bin/true", vec![]);
            existing.enabled = true;
            save_script(&mut s.scripts, existing)
        })
        .unwrap();
    let before = fs::read(store_path(dir.path())).unwrap();
    let mut configuration: PortableConfiguration =
        serde_json::from_slice(&store.export_data(None).unwrap()).unwrap();
    configuration.settings.scripts =
        vec![ScriptCommand::new("bad", "Bad", "relative/runtime", vec![])];
    assert!(
        store
            .preview(&serde_json::to_vec(&configuration).unwrap())
            .is_err()
    );
    assert_eq!(fs::read(store_path(dir.path())).unwrap(), before);
    assert!(open(dir.path()).settings().scripts[0].enabled);
    // Enabling a stored script after review works and persists.
    store
        .update(|s| -> Result<(), ConfigError> {
            set_script_enabled(&mut s.scripts, "existing", false);
            Ok(())
        })
        .unwrap();
    assert!(!open(dir.path()).settings().scripts[0].enabled);
}

// ----- Command / user-command shortcut model -----

#[test]
fn command_persistence_removal_and_duplicate_conflict() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(dir.path());
    store
        .update(|s| {
            assign_command_shortcut(s, "emoji", "<Control><Super><Shift>F19", "Test", &always)
        })
        .unwrap();
    let second = store.update(|s| {
        assign_command_shortcut(s, "help", "<Control><Super><Shift>F19", "Test", &always)
    });
    assert!(matches!(
        second,
        Err(UpdateError::Rejected(ShortcutError::AssignedElsewhere))
    ));
    assert_eq!(open(dir.path()).settings().command_shortcuts[0].id, "emoji");
    store
        .update(|s| -> Result<(), ConfigError> {
            remove_command_shortcut(s, "emoji");
            Ok(())
        })
        .unwrap();
    assert!(open(dir.path()).settings().command_shortcuts.is_empty());
    store
        .update(|s| {
            assign_command_shortcut(s, "help", "<Control><Super><Shift>F19", "Test", &always)
        })
        .unwrap();
}

#[test]
fn shortcut_rules_launcher_modifier_and_app_conflicts() {
    let mut settings = PortableSettings::default();
    assert_eq!(
        assign_command_shortcut(&mut settings, "help", "<Alt>space", "x", &always),
        Err(ShortcutError::LauncherConflict)
    );
    assert_eq!(
        assign_command_shortcut(&mut settings, "help", "<Shift>h", "x", &always),
        Err(ShortcutError::NeedsModifier)
    );
    assert!(matches!(
        assign_command_shortcut(&mut settings, "help", "<Hyper>h", "x", &always),
        Err(ShortcutError::Invalid(_))
    ));
    assign_app_shortcut(
        &mut settings,
        "firefox.desktop",
        "Firefox",
        "<Alt>f",
        "Alt+F",
        &always,
    )
    .unwrap();
    assert_eq!(
        assign_app_shortcut(
            &mut settings,
            "kate.desktop",
            "Kate",
            "<alt>F",
            "Alt+F",
            &always
        ),
        Err(ShortcutError::AssignedToApp("Firefox".into()))
    );
    assert_eq!(
        assign_command_shortcut(&mut settings, "help", "<Alt>f", "x", &always),
        Err(ShortcutError::AssignedElsewhere)
    );
    // Re-assigning the same app replaces its binding.
    assign_app_shortcut(
        &mut settings,
        "firefox.desktop",
        "Firefox",
        "<Alt>g",
        "Alt+G",
        &always,
    )
    .unwrap();
    assert_eq!(settings.app_shortcuts.len(), 1);
    assert_eq!(
        settings.app_shortcuts[0].accelerator.as_deref(),
        Some("<Alt>g")
    );
    assert!(remove_app_shortcut(&mut settings, "firefox.desktop"));
    assert!(!remove_app_shortcut(&mut settings, "firefox.desktop"));
    assert!(settings.validate().is_ok());
}

#[test]
fn disabled_command_releases_its_shortcut_and_retains_the_preference() {
    let mut settings = PortableSettings::default();
    assign_command_shortcut(&mut settings, "emoji", "<Control><Alt>F19", "Test", &always).unwrap();
    let blocked =
        assign_command_shortcut(&mut settings, "help", "<Control><Alt>F19", "Other", &always);
    assert_eq!(blocked, Err(ShortcutError::AssignedElsewhere));
    let emoji_disabled = |owner: &ShortcutOwner| *owner != ShortcutOwner::Command("emoji".into());
    assign_command_shortcut(
        &mut settings,
        "help",
        "<Control><Alt>F19",
        "Other",
        &emoji_disabled,
    )
    .unwrap();
    assert_eq!(
        settings.command_shortcuts.len(),
        2,
        "the disabled command keeps its preference"
    );
}

#[test]
fn unavailable_reassignment_retains_previous_binding() {
    let mut settings = PortableSettings::default();
    assign_command_shortcut(
        &mut settings,
        "emoji",
        "<Control><Alt>F19",
        "Original",
        &always,
    )
    .unwrap();
    assert!(
        assign_command_shortcut(&mut settings, "emoji", "<Alt>space", "Unavailable", &always)
            .is_err()
    );
    assert_eq!(settings.command_shortcuts[0].label, "Original");
    assert_eq!(
        settings.command_shortcuts[0].accelerator.as_deref(),
        Some("<Control><Alt>F19")
    );
}

#[test]
fn snippet_script_and_extension_shortcut_persistence_and_removal() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(dir.path());
    let snippet = store
        .update(|s| save_snippet(&mut s.snippets, Some("test"), "T", "text"))
        .unwrap();
    assert_eq!(snippet, "test");
    store
        .update(|s| -> Result<(), AutomationError> {
            save_script(
                &mut s.scripts,
                ScriptCommand::new("test", "T", "/usr/bin/true", vec![]),
            )?;
            install_extension(
                &mut s.extensions,
                ExtensionManifest {
                    id: "test".into(),
                    name: "T".into(),
                    description: String::new(),
                    api_version: 1,
                    executable: "/usr/bin/true".into(),
                    arguments: vec![],
                    preferences: vec![],
                    enabled: false,
                },
            )
        })
        .unwrap();
    for id in ["snippet:test", "script:test", "extension:test"] {
        store
            .update(|s| {
                assign_command_shortcut(
                    s,
                    id,
                    "<Control><Alt><Super><Shift>F19",
                    "Synthetic test shortcut",
                    &always,
                )
            })
            .unwrap();
        assert_eq!(
            open(dir.path())
                .settings()
                .command_shortcuts
                .iter()
                .map(|b| b.id.as_str())
                .collect::<Vec<_>>(),
            [id]
        );
        store
            .update(|s| -> Result<(), ConfigError> {
                remove_command_shortcut(s, id);
                Ok(())
            })
            .unwrap();
        assert!(open(dir.path()).settings().command_shortcuts.is_empty());
    }
    // A binding for a command that does not exist is refused by validation.
    let result = store
        .update(|s| assign_command_shortcut(s, "script:missing", "<Control>F19", "x", &always));
    assert!(matches!(result, Err(UpdateError::Config(_))));
    assert!(store.settings().command_shortcuts.is_empty());
}

// ----- AIHandoffTests (provider persistence) -----

#[test]
fn provider_preference_persists_and_invalid_values_fall_back_to_openai() {
    let dir = tempfile::tempdir().unwrap();
    let config = paths(dir.path());
    fs::create_dir_all(config.config_file.parent().unwrap()).unwrap();
    for stored in ["unknown-provider", "siri", "local"] {
        let doc = json!({"version":1,"activeProfileID":"P","profiles":[{"id":"P","name":"Personal","settings":{}}],
            "global":{"aiProvider": stored}});
        fs::write(&config.config_file, doc.to_string()).unwrap();
        let mut store = ConfigStore::open(config.clone(), HOME).unwrap();
        assert_eq!(store.global().ai_provider, AiProvider::OpenAi, "{stored}");
        assert!(
            fs::read_to_string(&config.config_file)
                .unwrap()
                .contains("\"aiProvider\": \"openai\"")
        );
        store.set_ai_provider(AiProvider::Claude).unwrap();
        assert_eq!(
            ConfigStore::open(config.clone(), HOME)
                .unwrap()
                .global()
                .ai_provider,
            AiProvider::Claude
        );
    }
}

// ----- LauncherPersonalizationTests (settings part) -----

#[test]
fn disabled_categories_and_favorites_persist() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(dir.path());
    store
        .update(|s| -> Result<(), ConfigError> {
            s.personalization.apps_enabled = false;
            s.personalization.files_enabled = false;
            s.personalization.commands_enabled = false;
            s.personalization.links_enabled = false;
            Ok(())
        })
        .unwrap();
    let link_id = store
        .update(|s| {
            save_quicklink(
                &mut s.quicklinks,
                &s.web_searches.clone(),
                None,
                "Example",
                "https://example.com",
                "",
                HOME,
            )
        })
        .unwrap();
    let reopened = open(dir.path());
    let p = &reopened.settings().personalization;
    assert!(!p.apps_enabled && !p.files_enabled && !p.commands_enabled && !p.links_enabled);
    assert!(!p.allows_file("/home/test/anything", HOME));
    store
        .update(|s| -> Result<(), ConfigError> {
            s.personalization.links_enabled = true;
            s.personalization.empty_mode = "favorites".into();
            toggle_pin(
                &mut s.pins,
                PinnedResult {
                    id: format!("link:{link_id}"),
                    name: "Example".into(),
                    path: "https://example.com".into(),
                    kind: "quicklink".into(),
                },
            );
            Ok(())
        })
        .unwrap();
    let reopened = open(dir.path());
    assert_eq!(reopened.settings().personalization.empty_mode, "favorites");
    assert_eq!(reopened.settings().pins[0].id, format!("link:{link_id}"));
}

#[test]
fn atomic_write_leaves_no_temporary_files() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(dir.path());
    for i in 0..5 {
        set_alias(&mut store, "a.desktop", &format!("alias {i}"));
    }
    let entries: Vec<_> = fs::read_dir(dir.path().join("config"))
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(entries, ["config.json"]);
}

#[test]
fn first_run_is_reported_once_and_the_onboarding_flag_persists() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(dir.path());
    assert!(store.was_created());
    assert!(!store.global().onboarding_dismissed);
    store
        .update_global(|g| g.onboarding_dismissed = true)
        .unwrap();
    drop(store);
    let reopened = open(dir.path());
    assert!(!reopened.was_created());
    assert!(reopened.global().onboarding_dismissed);
}

#[test]
fn launcher_shortcut_choices_and_custom_recording_check_conflicts() {
    let mut settings = PortableSettings::default();
    assign_command_shortcut(
        &mut settings,
        "emoji",
        "<Control>space",
        "Ctrl+Space",
        &always,
    )
    .unwrap();
    // A built-in choice used by a command shortcut is refused and nothing changes.
    assert_eq!(
        choose_launcher_shortcut(&mut settings, "control"),
        Err(ShortcutError::AssignedElsewhere)
    );
    assert_eq!(settings.launcher_shortcut_id, "option");
    choose_launcher_shortcut(&mut settings, "command").unwrap();
    assert_eq!(settings.launcher_accelerator().to_string(), "<Super>space");
    assert_eq!(
        choose_launcher_shortcut(&mut settings, "custom"),
        Err(ShortcutError::Invalid("custom".into()))
    );
    assert_eq!(
        assign_launcher_shortcut(&mut settings, "<Control>space", "Ctrl+Space"),
        Err(ShortcutError::AssignedElsewhere)
    );
    assert_eq!(
        assign_launcher_shortcut(&mut settings, "space", "Space"),
        Err(ShortcutError::NeedsModifier)
    );
    assign_launcher_shortcut(&mut settings, "<Control><Alt>t", "Ctrl+Alt+T").unwrap();
    assert_eq!(settings.launcher_shortcut_id, "custom");
    assert_eq!(
        settings.launcher_accelerator().to_string(),
        "<Control><Alt>t"
    );
    settings.validate().unwrap();
}

#[test]
fn recovered_corrupt_configuration_is_not_a_first_run() {
    let dir = tempfile::tempdir().unwrap();
    let config = paths(dir.path());
    fs::create_dir_all(config.config_file.parent().unwrap()).unwrap();
    fs::write(&config.config_file, "{ not json").unwrap();
    let store = ConfigStore::open(config, HOME).unwrap();
    assert!(store.recovered_corrupt_file().is_some());
    assert!(
        !store.was_created(),
        "recovery must not look like a first run"
    );
}

#[test]
fn in_memory_store_never_touches_the_configuration_file() {
    let dir = tempfile::tempdir().unwrap();
    let config = paths(dir.path());
    fs::create_dir_all(config.config_file.parent().unwrap()).unwrap();
    let newer = r#"{"version":2,"activeProfileID":"x","profiles":[]}"#;
    fs::write(&config.config_file, newer).unwrap();
    let error = ConfigStore::open(config.clone(), HOME).unwrap_err();
    let mut store = ConfigStore::in_memory(config.clone(), HOME, error.to_string());
    assert!(store.ephemeral_reason().unwrap().contains("newer Kebloop"));
    assert!(!store.was_created());
    assert_eq!(store.profiles().len(), 1);
    set_alias(&mut store, "app:x", "xx");
    store.record_use("app:x", true).unwrap();
    store
        .update_global(|g| g.onboarding_dismissed = true)
        .unwrap();
    store.flush().unwrap();
    assert_eq!(fs::read_to_string(&config.config_file).unwrap(), newer);
    assert_eq!(store.global().usage.usage["app:x"], 1);
}

#[test]
fn record_use_saves_once_and_deferred_usage_flushes() {
    use kebloop_services::persist::DebouncedWriter;
    use std::sync::Arc;
    use std::time::Duration;
    let dir = tempfile::tempdir().unwrap();
    let config = paths(dir.path());
    let mut store = ConfigStore::open(config.clone(), HOME).unwrap();
    let writer = Arc::new(DebouncedWriter::with_timing(
        Duration::from_secs(60),
        Duration::from_secs(60),
    ));
    store.set_deferred_writer(writer.clone());
    let before = fs::read(&config.config_file).unwrap();
    for _ in 0..25 {
        store.record_use("app:firefox", true).unwrap();
        store.record_emoji("🙂").unwrap();
    }
    assert_eq!(
        fs::read(&config.config_file).unwrap(),
        before,
        "usage is not written eagerly"
    );
    assert!(writer.has_pending());
    store.flush().unwrap();
    assert_eq!(
        writer.completed_writes(),
        1,
        "all updates coalesced into one write"
    );
    let reopened = ConfigStore::open(config.clone(), HOME).unwrap();
    assert_eq!(reopened.global().usage.usage["app:firefox"], 25);
    assert_eq!(reopened.global().usage.recent_results[0], "app:firefox");

    // A settings change is synchronous and carries the not yet flushed usage along.
    store.record_use("app:other", false).unwrap();
    set_alias(&mut store, "app:firefox", "ff");
    assert!(!writer.has_pending());
    let reopened = ConfigStore::open(config, HOME).unwrap();
    assert_eq!(reopened.global().usage.recent_results[0], "app:other");
    assert_eq!(reopened.settings().aliases.len(), 1);
}
