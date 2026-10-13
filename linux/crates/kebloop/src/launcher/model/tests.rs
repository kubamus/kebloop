use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use kebloop_core::{CalculationOutcome, CalculationResult};
use kebloop_platform::apps::{AppOrigin, DesktopApp};
use kebloop_services::automation::ScriptCommand;
use kebloop_services::clipboard::ClipboardEntry;
use kebloop_services::config::{ConfigurationProfile, PortableSettings};
use kebloop_services::file_search::FileSearchResult;
use kebloop_services::l10n::Localizer;
use kebloop_services::personalization::PinnedResult;
use kebloop_services::quicklinks::Quicklink;
use kebloop_services::snippets::Snippet;

use super::*;
use crate::platform::CommandAvailability;

const HOME: &str = "/home/test";

fn app(id: &str, name: &str) -> DesktopApp {
    DesktopApp {
        id: id.into(),
        path: PathBuf::from(format!("/usr/share/applications/{id}")),
        name: name.into(),
        generic_name: None,
        keywords: Vec::new(),
        comment: None,
        icon: Some("icon".into()),
        terminal: false,
        exec: "x".into(),
        categories: Vec::new(),
        startup_wm_class: None,
        actions: Vec::new(),
        origin: AppOrigin::System,
        subtitle: None,
    }
}

struct Fixture {
    settings: PortableSettings,
    apps: Vec<DesktopApp>,
    usage: HashMap<String, i64>,
    recents: Vec<String>,
    files: Vec<FileSearchResult>,
    directories: HashSet<String>,
    calculation: CalculationOutcome,
    l10n: Localizer,
}

impl Fixture {
    fn new() -> Self {
        Self {
            settings: PortableSettings::default(),
            apps: vec![
                app("firefox.desktop", "Firefox"),
                app("ghostty.desktop", "Ghostty"),
                app("terminal.desktop", "Terminal"),
                app("zed.desktop", "Zed"),
                app("alpha.desktop", "Alpha"),
            ],
            usage: HashMap::new(),
            recents: Vec::new(),
            files: Vec::new(),
            directories: HashSet::new(),
            calculation: CalculationOutcome::None,
            l10n: Localizer::english(),
        }
    }

    fn root(&self, query: &str) -> Assembled {
        assemble_root(&RootInputs {
            query,
            settings: &self.settings,
            l10n: &self.l10n,
            apps: &self.apps,
            usage: &self.usage,
            recents: &self.recents,
            files: &self.files,
            directories: &self.directories,
            calculation: &self.calculation,
            availability: &CommandAvailability::new(true, [], None),
            running: &HashSet::new(),
            home: HOME,
            file_exists: &|_| true,
        })
    }
}

fn ids(assembled: &Assembled) -> Vec<&str> {
    assembled.items.iter().map(|i| i.id.as_str()).collect()
}

#[test]
fn alias_outranks_exact_application_name() {
    let mut f = Fixture::new();
    f.settings
        .aliases
        .insert("ghostty.desktop".into(), "terminal".into());
    let result = f.root("terminal");
    assert_eq!(result.items[0].id, "ghostty.desktop");
    assert_eq!(result.items[1].id, "terminal.desktop");
}

#[test]
fn desktop_entry_keywords_and_generic_names_find_applications() {
    let mut fixture = Fixture::new();
    fixture.apps[0].generic_name = Some("Web Browser".into());
    fixture.apps[1].keywords = vec!["terminal".into(), "console".into()];
    let browser = fixture.root("browser");
    assert_eq!(
        browser.items.first().map(|i| i.id.as_str()),
        Some("firefox.desktop")
    );
    let terminal = fixture.root("console");
    assert_eq!(
        terminal.items.first().map(|i| i.id.as_str()),
        Some("ghostty.desktop")
    );
    // The real name still wins over a keyword on another app.
    let by_name = fixture.root("terminal");
    assert_eq!(
        by_name.items.first().map(|i| i.id.as_str()),
        Some("terminal.desktop")
    );
}

#[test]
fn hidden_applications_are_filtered_by_id_or_path() {
    let mut f = Fixture::new();
    f.apps.push(app("monitor.desktop", "Monitor"));
    f.settings.personalization.hidden_apps = vec![
        "firefox.desktop".into(),
        "/usr/share/applications/ghostty.desktop".into(),
    ];
    let result = f.root("o");
    assert!(!ids(&result).contains(&"firefox.desktop"));
    assert!(!ids(&result).contains(&"ghostty.desktop"));
    // The broad query "o" is capped at the ranking limit; check the visible app separately.
    assert!(ids(&f.root("monitor")).contains(&"monitor.desktop"));
}

#[test]
fn launch_counts_boost_otherwise_equal_matches() {
    let mut f = Fixture::new();
    f.apps = vec![
        app("a.desktop", "Editor One"),
        app("b.desktop", "Editor Two"),
    ];
    assert_eq!(f.root("editor").items[0].id, "a.desktop");
    f.usage.insert("b.desktop".into(), 25);
    assert_eq!(f.root("editor").items[0].id, "b.desktop");
}

#[test]
fn calculation_comes_first_and_copies() {
    let mut f = Fixture::new();
    f.calculation = CalculationOutcome::Result(CalculationResult {
        expression: "2*(3+4)".into(),
        display: "14".into(),
        clipboard: "14".into(),
    });
    let result = f.root("2*(3+4)");
    assert_eq!(result.items[0].id, "calculator");
    assert_eq!(result.items[0].title, "14");
    assert!(matches!(
        result.items[0].target,
        Target::Calculation { ref result } if result.clipboard == "14" && result.display == "14"
    ));
}

#[test]
fn calculator_failure_becomes_a_message() {
    let mut f = Fixture::new();
    f.calculation = CalculationOutcome::Failure("Division by zero.".into());
    assert_eq!(f.root("1/0").message.as_deref(), Some("Division by zero."));
}

#[test]
fn files_sit_below_ranked_results_and_respect_folders() {
    let mut f = Fixture::new();
    f.settings.personalization.folders = vec!["/home/test/Documents".into()];
    f.files = vec![
        FileSearchResult::new("fire.txt", "/home/test/Documents/fire.txt"),
        FileSearchResult::new("fire.log", "/home/test/Other/fire.log"),
    ];
    let result = f.root("fire");
    let order = ids(&result);
    assert_eq!(order[0], "firefox.desktop");
    assert!(order.contains(&"file:/home/test/Documents/fire.txt"));
    assert!(!order.contains(&"file:/home/test/Other/fire.log"));
    let last = result.items.last().map(|i| i.id.as_str());
    assert_eq!(last, Some("file:/home/test/Documents/fire.txt"));
}

#[test]
fn at_most_twelve_files_are_shown() {
    let mut f = Fixture::new();
    f.files = (0..30)
        .map(|n| FileSearchResult::new(format!("note{n}.txt"), format!("/home/test/note{n}.txt")))
        .collect();
    let files = f
        .root("note")
        .items
        .into_iter()
        .filter(|i| matches!(i.target, Target::File { .. }))
        .count();
    assert_eq!(files, 12);
}

#[test]
fn disabling_categories_removes_their_results() {
    let mut f = Fixture::new();
    f.settings.personalization.apps_enabled = false;
    f.settings.personalization.commands_enabled = false;
    f.settings.personalization.files_enabled = false;
    f.settings.personalization.links_enabled = false;
    f.files = vec![FileSearchResult::new("fire.txt", "/home/test/fire.txt")];
    assert!(f.root("fire").items.is_empty());
    assert!(f.root("").items.is_empty());
}

#[test]
fn quicklinks_match_by_name_and_by_keyword_with_argument() {
    let mut f = Fixture::new();
    f.settings.quicklinks.push(Quicklink {
        id: "gh".into(),
        name: "GitHub".into(),
        target: "https://github.com/search?q={query}".into(),
        keyword: "gh".into(),
    });
    let by_name = f.root("github");
    assert_eq!(by_name.items[0].id, "link:gh");
    let by_keyword = f.root("gh kebloop");
    assert_eq!(by_keyword.items[0].id, "link:gh");
    assert!(matches!(
        by_keyword.items[0].target,
        Target::Quicklink { ref argument, .. } if argument == "kebloop"
    ));
    assert!(
        by_keyword.items[0]
            .subtitle
            .contains("https://github.com/search?q=kebloop")
    );
}

#[test]
fn web_search_keyword_resolves_to_the_template() {
    let f = Fixture::new();
    let result = f.root("web cats");
    assert_eq!(result.items[0].id, "link:web:duckduckgo");
}

#[test]
fn web_search_fallback_only_when_nothing_matches() {
    let f = Fixture::new();
    let nothing = f.root("zzzzqqq");
    assert_eq!(nothing.items.len(), 1);
    assert!(matches!(nothing.items[0].target, Target::WebSearch { .. }));
    let mut without_template = Fixture::new();
    without_template.settings.web_searches.clear();
    assert!(without_template.root("zzzzqqq").items.is_empty());
    let something = f.root("firefox");
    assert!(
        !something
            .items
            .iter()
            .any(|i| matches!(i.target, Target::WebSearch { .. }))
    );
}

#[test]
fn commands_match_english_and_polish_aliases() {
    let f = Fixture::new();
    assert_eq!(f.root("schowek").items[0].id, "command:clipboard");
    assert_eq!(f.root("emoji").items[0].id, "command:emoji");
    assert_eq!(f.root("ustawienia").items[0].id, "command:settings");
}

#[test]
fn shutdown_is_hidden_when_unavailable() {
    let f = Fixture::new();
    let hidden = assemble_root(&RootInputs {
        query: "shutdown",
        settings: &f.settings,
        l10n: &f.l10n,
        apps: &f.apps,
        usage: &f.usage,
        recents: &[],
        files: &[],
        directories: &f.directories,
        calculation: &f.calculation,
        availability: &CommandAvailability::new(false, [], None),
        running: &HashSet::new(),
        home: HOME,
        file_exists: &|_| true,
    });
    assert!(!ids(&hidden).contains(&"command:shutdown"));
    assert!(ids(&f.root("shutdown")).contains(&"command:shutdown"));
    assert!(!ids(&f.root("")).is_empty());
}

#[test]
fn unavailable_commands_never_appear() {
    let f = Fixture::new();
    assert!(!ids(&f.root("apple shortcuts")).contains(&"command:appleShortcuts"));
}

#[test]
fn enabled_scripts_and_snippets_are_searchable_but_disabled_scripts_are_not() {
    let mut f = Fixture::new();
    let mut enabled = ScriptCommand::new("s1", "Backup Notes", "/bin/true", vec![]);
    enabled.enabled = true;
    let disabled = ScriptCommand::new("s2", "Backup Photos", "/bin/true", vec![]);
    f.settings.scripts = vec![enabled, disabled];
    f.settings.snippets = vec![Snippet {
        id: "n1".into(),
        name: "Backup Greeting".into(),
        template: "Hi".into(),
        keyword: None,
    }];
    let result = f.root("backup");
    let order = ids(&result);
    assert!(order.contains(&"script:s1"));
    assert!(order.contains(&"snippet:n1"));
    assert!(!order.contains(&"script:s2"));
}

#[test]
fn empty_state_frequent_mode_shows_favorites_then_top_apps_then_commands() {
    let mut f = Fixture::new();
    f.usage.insert("zed.desktop".into(), 9);
    f.usage.insert("ghostty.desktop".into(), 4);
    f.settings.pins = vec![PinnedResult {
        id: "firefox.desktop".into(),
        name: "Firefox".into(),
        path: "/usr/share/applications/firefox.desktop".into(),
        kind: "application".into(),
    }];
    let result = f.root("");
    let order = ids(&result);
    assert_eq!(
        &order[..4],
        [
            "firefox.desktop",
            "zed.desktop",
            "ghostty.desktop",
            "alpha.desktop"
        ]
    );
    assert!(order[4..].iter().all(|id| id.starts_with("command:")));
    assert!(order.contains(&"command:emoji"));
    // No duplicates.
    let unique: HashSet<&&str> = order.iter().collect();
    assert_eq!(unique.len(), order.len());
}

#[test]
fn empty_state_favorites_mode_lists_only_pins_and_commands() {
    let mut f = Fixture::new();
    f.settings.personalization.empty_mode = "favorites".into();
    f.settings.pins = vec![PinnedResult {
        id: "command:emoji".into(),
        name: "Emoji".into(),
        path: String::new(),
        kind: "command".into(),
    }];
    let result = f.root("");
    let order = ids(&result);
    assert_eq!(order[0], "command:emoji");
    assert!(order.iter().all(|id| id.starts_with("command:")));
    assert_eq!(order.iter().filter(|id| **id == "command:emoji").count(), 1);
}

#[test]
fn empty_state_recent_mode_follows_recents() {
    let mut f = Fixture::new();
    f.settings.personalization.empty_mode = "recent".into();
    f.recents = vec![
        "zed.desktop".into(),
        "gone.desktop".into(),
        "firefox.desktop".into(),
    ];
    let result = f.root("");
    let order = ids(&result);
    assert_eq!(&order[..2], ["zed.desktop", "firefox.desktop"]);
}

#[test]
fn pinned_files_appear_only_while_they_exist() {
    let mut f = Fixture::new();
    f.settings.personalization.folders = vec!["/home/test".into()];
    f.settings.pins = vec![PinnedResult {
        id: "file:/home/test/notes.txt".into(),
        name: "notes.txt".into(),
        path: "/home/test/notes.txt".into(),
        kind: "file".into(),
    }];
    assert_eq!(ids(&f.root(""))[0], "file:/home/test/notes.txt");
    let missing = assemble_root(&RootInputs {
        query: "",
        settings: &f.settings,
        l10n: &f.l10n,
        apps: &f.apps,
        usage: &f.usage,
        recents: &[],
        files: &[],
        directories: &f.directories,
        calculation: &f.calculation,
        availability: &CommandAvailability::new(true, [], None),
        running: &HashSet::new(),
        home: HOME,
        file_exists: &|_| false,
    });
    assert!(!ids(&missing).contains(&"file:/home/test/notes.txt"));
}

#[test]
fn duplicate_application_names_get_a_location_subtitle() {
    let mut f = Fixture::new();
    let mut flatpak = app("org.x.Edit.desktop", "Edit");
    flatpak.subtitle = Some("Flatpak".into());
    let mut system = app("edit.desktop", "Edit");
    system.subtitle = Some("System".into());
    f.apps = vec![flatpak, system];
    let result = f.root("edit");
    assert!(
        result.items[0].subtitle.starts_with("Flatpak · ")
            || result.items[0].subtitle.starts_with("System · ")
    );
    f.apps = vec![app("solo.desktop", "Solo")];
    assert_eq!(f.root("solo").items[0].subtitle, "Application");
}

#[test]
fn ranking_a_thousand_applications_is_fast() {
    let mut f = Fixture::new();
    f.apps = (0..1000)
        .map(|n| {
            app(
                &format!("app{n}.desktop"),
                &format!("Application Number {n}"),
            )
        })
        .collect();
    let started = std::time::Instant::now();
    for query in ["a", "app", "number 99", "xyz"] {
        let _ = f.root(query);
    }
    let per_query = started.elapsed() / 4;
    assert!(
        per_query < std::time::Duration::from_millis(30),
        "ranking took {per_query:?} per query"
    );
}

#[test]
fn snippet_and_quicklink_lists_rank_by_name() {
    let l10n = Localizer::english();
    let snippets = vec![
        Snippet {
            id: "1".into(),
            name: "Signature".into(),
            template: "Best,\nMe".into(),
            keyword: None,
        },
        Snippet {
            id: "2".into(),
            name: "Address".into(),
            template: "1 Road".into(),
            keyword: None,
        },
    ];
    let found = assemble_snippets("sig", &snippets, &HashMap::new(), &l10n);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].subtitle, "Best, Me");
    assert_eq!(
        assemble_snippets("", &snippets, &HashMap::new(), &l10n).len(),
        2
    );
}

#[test]
fn web_search_view_needs_a_query_and_a_template() {
    let l10n = Localizer::english();
    let settings = PortableSettings::default();
    assert!(assemble_web_search("  ", &settings, &l10n, HOME).is_empty());
    assert_eq!(assemble_web_search("cats", &settings, &l10n, HOME).len(), 1);
    let mut none = PortableSettings::default();
    none.web_searches.clear();
    assert!(assemble_web_search("cats", &none, &l10n, HOME).is_empty());
}

#[test]
fn profile_list_marks_the_active_profile() {
    let l10n = Localizer::english();
    let profiles = vec![
        ConfigurationProfile {
            id: "1".into(),
            name: "Personal".into(),
            settings: PortableSettings::default(),
        },
        ConfigurationProfile {
            id: "2".into(),
            name: "Work".into(),
            settings: PortableSettings::default(),
        },
    ];
    let items = assemble_profiles("", &profiles, "2", &l10n);
    assert_eq!(items.len(), 2);
    assert!(matches!(
        items[1].target,
        Target::Profile { active: true, .. }
    ));
    assert_eq!(items[1].subtitle, "Active");
    assert_eq!(assemble_profiles("wor", &profiles, "2", &l10n).len(), 1);
}

#[test]
fn clipboard_list_puts_pins_first_and_filters_diacritics() {
    let l10n = Localizer::english();
    let tz = jiff::tz::TimeZone::UTC;
    let at = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
    let mut pinned = ClipboardEntry::new("p", "pinned note", at);
    pinned.is_pinned = true;
    let entries = vec![
        ClipboardEntry::new("a", "Zażółć gęślą jaźń", at),
        ClipboardEntry::new("b", "second\nline text", at),
        pinned,
    ];
    let all = assemble_clipboard("", &entries, &l10n, &tz);
    assert_eq!(all[0].id, "clipboard:p");
    assert!(all[0].title.starts_with("📌 "));
    let filtered = assemble_clipboard("zazołc", &entries, &l10n, &tz);
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].id, "clipboard:a");
    assert_eq!(all[2].title, "second line text");
    assert_eq!(all[0].subtitle, "2023-11-14 22:13");
}

#[test]
fn browse_lists_commands_and_quicklinks_with_shortcut_labels() {
    let mut f = Fixture::new();
    f.settings.quicklinks.push(Quicklink {
        id: "gh".into(),
        name: "GitHub".into(),
        target: "https://github.com".into(),
        keyword: String::new(),
    });
    let mut labels = HashMap::new();
    labels.insert("emoji".to_owned(), "<Alt>e".to_owned());
    let inputs = RootInputs {
        query: "",
        settings: &f.settings,
        l10n: &f.l10n,
        apps: &f.apps,
        usage: &f.usage,
        recents: &[],
        files: &[],
        directories: &f.directories,
        calculation: &f.calculation,
        availability: &CommandAvailability::new(true, [], None),
        running: &HashSet::new(),
        home: HOME,
        file_exists: &|_| true,
    };
    let items = assemble_browse(&inputs, &labels);
    assert!(
        items
            .iter()
            .all(|i| !matches!(i.target, Target::Application { .. }))
    );
    assert!(items.iter().any(|i| i.id == "link:gh"));
    let emoji = items
        .iter()
        .find(|i| i.id == "command:emoji")
        .map(|i| i.subtitle.clone());
    assert_eq!(emoji.as_deref(), Some("Command · <Alt>e"));
}

#[test]
fn clipboard_filter_is_fast_on_large_entries_and_matches_the_whole_text() {
    let l10n = Localizer::from_env(&|_| None);
    let tz = jiff::tz::TimeZone::UTC;
    let at = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    let filler = "Zażółć gęślą jaźń lorem ipsum dolor sit amet\n".repeat(2600);
    assert!(filler.len() > 114_000);
    let entries: Vec<ClipboardEntry> = (0..200)
        .map(|i| {
            let tail = if i == 7 { "needle-in-the-tail" } else { "" };
            ClipboardEntry::new(format!("e{i}"), format!("{i} head {filler}{tail}"), at)
        })
        .collect();
    let mut cache = ClipboardFoldCache::default();
    // Cold: every entry is folded once.
    let cold = Instant::now();
    assert_eq!(
        assemble_clipboard_cached(&mut cache, "head", &entries, &l10n, &tz).len(),
        200
    );
    assert!(
        cold.elapsed() < Duration::from_secs(10),
        "cold {:?}",
        cold.elapsed()
    );
    // Warm: typing more characters only scans the cached folded texts.
    let warm = Instant::now();
    for query in ["z", "za", "zaz", "zazo", "zazoł", "zazołc"] {
        let hits = assemble_clipboard_cached(&mut cache, query, &entries, &l10n, &tz);
        assert_eq!(hits.len(), 200, "{query}");
    }
    let per_filter = warm.elapsed() / 6;
    assert!(
        per_filter < Duration::from_secs(2),
        "warm filter took {per_filter:?}"
    );
    // The whole text is searched, as in the macOS app.
    let tail_hits =
        assemble_clipboard_cached(&mut cache, "needle-in-the-tail", &entries, &l10n, &tz);
    assert_eq!(tail_hits.len(), 1);
    assert_eq!(tail_hits[0].id, "clipboard:e7");
    assert_eq!(
        assemble_clipboard_cached(&mut cache, "7 head", &entries, &l10n, &tz)
            .iter()
            .filter(|item| item.id == "clipboard:e7")
            .count(),
        1
    );
}

#[test]
fn clipboard_fold_cache_follows_changed_and_removed_entries() {
    let l10n = Localizer::from_env(&|_| None);
    let tz = jiff::tz::TimeZone::UTC;
    let at = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    let mut cache = ClipboardFoldCache::default();
    let mut entries = vec![ClipboardEntry::new("a", "Łódź", at)];
    assert_eq!(
        assemble_clipboard_cached(&mut cache, "łodz", &entries, &l10n, &tz).len(),
        1
    );
    entries[0].text = "changed text".into();
    assert!(assemble_clipboard_cached(&mut cache, "łodz", &entries, &l10n, &tz).is_empty());
    entries.clear();
    assert!(assemble_clipboard_cached(&mut cache, "", &entries, &l10n, &tz).is_empty());
    assert!(cache.entries.is_empty());
}

#[test]
fn thumbnail_key_changes_when_the_entry_id_gets_a_new_image() {
    use kebloop_services::clipboard::{AttachmentKind, ClipboardAttachment};
    let at = SystemTime::UNIX_EPOCH;
    let attachment = |file: &str| ClipboardAttachment {
        file_name: file.to_owned(),
        pasteboard_type: "image/png".into(),
        byte_count: 10,
        kind: AttachmentKind::Image,
    };
    let mut entry = ClipboardEntry::new("same-id", "caption", at);
    assert_eq!(thumbnail_key(&entry), None);
    entry.image_attachment = Some(attachment("clipboard-AAA.bin"));
    let first = thumbnail_key(&entry).unwrap();
    entry.image_attachment = Some(attachment("clipboard-BBB.bin"));
    let second = thumbnail_key(&entry).unwrap();
    assert_ne!(first, second, "a reused id must not hit the old thumbnail");
    assert_eq!(
        split_thumbnail_key(&second),
        Some(("same-id", "clipboard-BBB.bin"))
    );
    let l10n = Localizer::from_env(&|_| None);
    let item = clipboard_item(&entry, &l10n, &jiff::tz::TimeZone::UTC);
    assert_eq!(item.icon, IconSpec::ClipboardImage(second));
}
