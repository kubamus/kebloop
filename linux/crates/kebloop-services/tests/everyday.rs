//! Ports of the everyday-workflow tests: `CalculationStoreTests` (history), `HistoryBrowsingTests`,
//! `PinnedIndicatorTests`, the window-management part of `CommandRegistryTests` and the
//! configuration part of `SnippetTests`. The Swift tests drive the AppKit launcher; these check
//! the same behaviour at the model level the GTK launcher is built on.

use std::path::Path;
use std::sync::Arc;

use jiff::Timestamp;
use kebloop_core::CalculationResult;
use kebloop_services::calculation_history::{CalculationHistoryStore, FILE_NAME, MAXIMUM_ENTRIES};
use kebloop_services::commands::LauncherCommand;
use kebloop_services::config::{ConfigPaths, ConfigStore, PortableSettings, UpdateError};
use kebloop_services::error::ConfigError;
use kebloop_services::l10n::{Language, Localizer};
use kebloop_services::personalization::{PinnedResult, is_pinned, toggle_pin};
use kebloop_services::result_action::{
    CALCULATION_HISTORY_ACTIONS, ResultActionId, row_accessibility_label,
};
use kebloop_services::shortcuts::{
    ShortcutError, ShortcutOwner, assign_app_shortcut, assign_command_shortcut,
};
use kebloop_services::snippets::{Snippet, save_snippet_with_keyword};
use kebloop_services::window_management::{WindowAction, apply_suggested_shortcuts};

const HOME: &str = "/home/test";

fn always(_: &ShortcutOwner) -> bool {
    true
}

fn open_config(dir: &Path) -> ConfigStore {
    ConfigStore::open(ConfigPaths::in_dir(&dir.join("config")), HOME).unwrap()
}

fn clock(seconds: i64) -> Arc<dyn Fn() -> Timestamp + Send + Sync> {
    Arc::new(move || Timestamp::from_second(seconds).unwrap())
}

fn result(expression: &str, display: &str, clipboard: &str) -> CalculationResult {
    CalculationResult {
        expression: expression.into(),
        display: display.into(),
        clipboard: clipboard.into(),
    }
}

// ----- CalculationStoreTests (history) -----

#[test]
fn history_persists_only_accepted_calculations_and_preserves_clipboard_text() {
    let dir = tempfile::tempdir().unwrap();
    let accepted_at = 1_791_000_000;
    let mut store = CalculationHistoryStore::open(dir.path(), clock(accepted_at));
    assert!(store.entries().is_empty());

    assert!(store.record_accepted(&result("2 + 2", "4", "four")));
    assert_eq!(store.entries().len(), 1);
    assert_eq!(store.entries()[0].result().clipboard, "four");
    assert_eq!(store.entries()[0].accepted_at.as_second(), accepted_at);

    let mut reloaded = CalculationHistoryStore::open(dir.path(), clock(0));
    assert_eq!(reloaded.entries(), store.entries());
    reloaded.clear();
    assert!(reloaded.entries().is_empty());
    assert!(!dir.path().join(FILE_NAME).exists());
    assert!(
        CalculationHistoryStore::open(dir.path(), clock(0))
            .entries()
            .is_empty()
    );
}

#[test]
fn history_keeps_at_most_one_hundred_accepted_entries() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = CalculationHistoryStore::open(dir.path(), clock(1));
    for value in 0..=100 {
        let answer = (value + 1).to_string();
        store.record_accepted(&result(&format!("{value} + 1"), &answer, &answer));
    }
    assert_eq!(store.entries().len(), MAXIMUM_ENTRIES);
    assert_eq!(store.entries()[0].expression, "100 + 1");
    assert_eq!(
        CalculationHistoryStore::open(dir.path(), clock(1))
            .entries()
            .len(),
        MAXIMUM_ENTRIES
    );
}

#[test]
fn history_rejects_unstorable_results_and_skips_invalid_stored_entries() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = CalculationHistoryStore::open(dir.path(), clock(1));
    assert!(!store.record_accepted(&result("", "4", "4")));
    assert!(!store.record_accepted(&result("2 + 2", "", "4")));
    // 2,048 characters of three bytes each exceed the 2 KiB byte limit.
    assert!(!store.record_accepted(&result("2 + 2", &"€".repeat(2_048), "x")));
    assert!(store.record_accepted(&result(&"9".repeat(3_000), "x", "x")));
    assert_eq!(
        store.entries()[0].expression.len(),
        2_048,
        "long text is cut to 2 KiB"
    );

    std::fs::write(
        dir.path().join(FILE_NAME),
        r#"[{"id":"","expression":"a","display":"b","acceptedAt":1},
            {"id":"ok","expression":"a","display":"b","acceptedAt":1},
            {"id":"bad","expression":"a","display":"","acceptedAt":1}]"#,
    )
    .unwrap();
    let loaded = CalculationHistoryStore::open(dir.path(), clock(1));
    assert_eq!(loaded.entries().len(), 1);
    assert_eq!(
        loaded.entries()[0].result().clipboard,
        "b",
        "missing clipboard falls back to display"
    );

    std::fs::write(dir.path().join(FILE_NAME), "garbage").unwrap();
    assert!(
        CalculationHistoryStore::open(dir.path(), clock(1))
            .entries()
            .is_empty()
    );
}

#[test]
fn history_is_local_and_excluded_from_export() {
    let dir = tempfile::tempdir().unwrap();
    let mut history = CalculationHistoryStore::open(&dir.path().join("data"), clock(1));
    history.record_accepted(&result(
        "secret-expression",
        "secret-answer",
        "secret-answer",
    ));
    let config = open_config(dir.path());
    let export = String::from_utf8(config.export_data(None).unwrap()).unwrap();
    assert!(!export.contains("secret"), "history must not be exported");
    assert!(!export.to_lowercase().contains("calculationhistory"));
    let config_files: Vec<_> = std::fs::read_dir(dir.path().join("config"))
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        config_files.iter().all(|name| !name.contains("history")),
        "{config_files:?}"
    );
}

#[test]
fn an_in_memory_history_never_touches_the_disk() {
    let mut history = CalculationHistoryStore::in_memory(clock(1));
    assert!(history.record_accepted(&result("1 + 1", "2", "2")));
    assert_eq!(history.entries().len(), 1);
    history.clear();
    assert!(history.entries().is_empty());
}

// ----- HistoryBrowsingTests -----

#[test]
fn history_shows_distinct_accepted_events_filters_and_offers_the_clear_action() {
    let dir = tempfile::tempdir().unwrap();
    let mut history = CalculationHistoryStore::open(dir.path(), clock(1));
    let accepted = result("2 + 2", "4", "4");
    history.record_accepted(&accepted);
    history.record_accepted(&accepted);

    // Opening the command lists every entry, with distinct result ids.
    let identifiers: Vec<String> = history.filtered("").iter().map(|e| e.result_id()).collect();
    let expected: Vec<String> = history
        .entries()
        .iter()
        .map(|e| format!("calculation-history:{}", e.id))
        .collect();
    assert_eq!(identifiers, expected);
    assert_eq!(
        identifiers
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len(),
        2
    );
    assert!(LauncherCommand::from_id("calculationHistory").is_some());

    // Typing filters by expression or answer.
    assert_eq!(history.filtered("2 +").len(), 2);
    assert_eq!(history.filtered("4").len(), 2);
    assert_eq!(history.filtered("NOT SAVED").len(), 0);
    // The menu offers Open and Clear only.
    assert_eq!(
        CALCULATION_HISTORY_ACTIONS,
        [
            ResultActionId::Open,
            ResultActionId::ClearCalculationHistory
        ]
    );
    // Browsing and typing do not add accepted calculations.
    assert_eq!(history.entries().len(), 2);
}

// ----- PinnedIndicatorTests -----

#[test]
fn a_pinned_result_has_an_accessible_status_and_unpinning_removes_it() {
    let english = Localizer::english();
    let polish = Localizer::new(Language::Polish);
    let mut pins = Vec::new();
    let pin = PinnedResult {
        id: "firefox.desktop".into(),
        name: "Firefox".into(),
        path: "/usr/share/applications/firefox.desktop".into(),
        kind: "application".into(),
    };
    let label = |pinned: bool, localizer: &Localizer| {
        row_accessibility_label("Firefox", "Application", "Open", pinned, localizer)
    };

    assert!(!is_pinned(&pins, &pin.id));
    assert_eq!(
        label(is_pinned(&pins, &pin.id), &english),
        "Firefox, Application, Open"
    );

    assert!(toggle_pin(&mut pins, pin.clone()));
    assert!(is_pinned(&pins, &pin.id));
    assert_eq!(
        label(true, &english),
        "Firefox, Application, Open, Pinned to Favorites"
    );
    assert!(
        label(true, &polish).ends_with(", Przypięte do ulubionych"),
        "{}",
        label(true, &polish)
    );

    assert!(!toggle_pin(&mut pins, pin.clone()));
    assert!(!is_pinned(&pins, &pin.id));
    assert!(!label(is_pinned(&pins, &pin.id), &english).contains("Pinned to Favorites"));
}

// ----- CommandRegistryTests (window management) -----

#[test]
fn window_shortcuts_persist_and_reject_a_conflicting_app_binding() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open_config(dir.path());
    let accelerator = "<Control><Alt><Super><Shift>F19";
    store
        .update(|s| {
            assign_app_shortcut(s, "test-app.desktop", "Test", accelerator, "Test", &always)
        })
        .unwrap();
    let top_left = LauncherCommand::TopLeftQuarter.id();

    let rejected =
        store.update(|s| assign_command_shortcut(s, top_left, accelerator, "Conflict", &always));
    assert!(rejected.is_err());
    assert!(store.settings().command_shortcuts.is_empty());

    store
        .update(|s| -> Result<(), ConfigError> {
            kebloop_services::shortcuts::remove_app_shortcut(s, "test-app.desktop");
            Ok(())
        })
        .unwrap();
    store
        .update(|s| assign_command_shortcut(s, top_left, accelerator, "Test shortcut", &always))
        .unwrap();
    let reloaded = open_config(dir.path());
    assert_eq!(reloaded.settings().command_shortcuts[0].id, top_left);
    assert_eq!(
        reloaded.settings().command_shortcuts[0].label,
        "Test shortcut"
    );
}

#[test]
fn suggested_window_shortcuts_fill_gaps_keep_existing_and_report_conflicts() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open_config(dir.path());
    // An application already owns Ctrl+Alt+Left, and one window command already has a binding.
    store
        .update(|s| {
            assign_app_shortcut(
                s,
                "files.desktop",
                "Files",
                "<Control><Alt>Left",
                "Files",
                &always,
            )
        })
        .unwrap();
    store
        .update(|s| assign_command_shortcut(s, "centerWindow", "<Super>c", "Super+C", &always))
        .unwrap();

    let outcome = store
        .update(|s| -> Result<_, ConfigError> { Ok(apply_suggested_shortcuts(s, &always)) })
        .unwrap();
    assert_eq!(outcome.conflicts, ["leftHalf"]);
    assert_eq!(outcome.added, WindowAction::ALL.len() - 2);
    let settings: &PortableSettings = store.settings();
    let binding = |id: &str| settings.command_shortcuts.iter().find(|b| b.id == id);
    assert_eq!(
        binding("centerWindow").map(|b| b.label.as_str()),
        Some("Super+C")
    );
    assert!(binding("leftHalf").is_none());
    assert_eq!(
        binding("maximizeWindow").and_then(|b| b.accelerator.as_deref()),
        Some("<Control><Alt>Return")
    );
    // Persisted with the macOS-compatible codes so the configuration stays portable.
    let reloaded = open_config(dir.path());
    let maximize = reloaded
        .settings()
        .command_shortcuts
        .iter()
        .find(|b| b.id == "maximizeWindow")
        .unwrap();
    assert_eq!(
        (maximize.key_code, maximize.modifiers),
        (Some(36), Some(0x1800))
    );

    // Applying again adds nothing: the conflicting suggestion is reported again.
    let again = store
        .update(|s| -> Result<_, ConfigError> { Ok(apply_suggested_shortcuts(s, &always)) })
        .unwrap();
    assert_eq!(again.added, 0);
    assert_eq!(again.conflicts, ["leftHalf"]);
    let rejected = store
        .update(|s| assign_command_shortcut(s, "leftHalf", "<Control><Alt>Left", "x", &always));
    assert!(matches!(
        rejected,
        Err(UpdateError::Rejected(ShortcutError::AssignedElsewhere))
    ));
}

#[test]
fn imported_window_bindings_from_macos_use_the_new_command_ids() {
    // A macOS export with a Control-Option window binding validates and resolves on Linux.
    let mut settings = PortableSettings::default();
    settings
        .command_shortcuts
        .push(kebloop_services::shortcuts::CommandShortcutBinding {
            id: "topLeftQuarter".into(),
            key_code: Some(32),
            modifiers: Some(0x1800),
            label: "⌃⌥ U".into(),
            accelerator: None,
        });
    assert!(settings.validate().is_ok());
    let resolved = settings.command_shortcuts[0].resolved().unwrap();
    assert_eq!(resolved.to_string(), "<Control><Alt>u");
}

// ----- SnippetTests (configuration) -----

#[test]
fn snippet_keywords_and_defaults_persist_through_configuration_and_conflicts_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open_config(dir.path());
    store
        .update(|s| {
            save_snippet_with_keyword(
                &mut s.snippets,
                None,
                "Greeting",
                "Hello {name:friend}",
                Some(";hello"),
            )
        })
        .unwrap();
    assert_eq!(
        open_config(dir.path()).settings().snippets,
        store.settings().snippets
    );
    assert_eq!(
        store.settings().snippets[0].keyword.as_deref(),
        Some(";hello")
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

    // A keyword clash is rejected by the editor and by import validation.
    assert!(
        store
            .update(|s| save_snippet_with_keyword(
                &mut s.snippets,
                None,
                "Other",
                "Other",
                Some(";HELLO")
            ))
            .is_err()
    );
    assert_eq!(store.settings().snippets.len(), 1);
    let mut settings = store.settings().clone();
    settings
        .snippets
        .push(Snippet::new("Clash", "x").with_keyword(";Hello"));
    assert!(settings.validate().is_err());

    // A configuration written before keywords existed still imports.
    let mut json: serde_json::Value = serde_json::from_slice(&data).unwrap();
    for key in ["snippets", "scripts", "extensions"] {
        json["settings"].as_object_mut().unwrap().remove(key);
    }
    store.preview(json.to_string().as_bytes()).unwrap();
    let legacy = serde_json::json!({
        "id": "legacy", "name": "Legacy", "template": "Hello {name}"
    });
    json["settings"]["snippets"] = serde_json::json!([legacy]);
    let preview = store.preview(json.to_string().as_bytes()).unwrap();
    assert_eq!(preview.configuration.settings.snippets[0].keyword, None);
}
