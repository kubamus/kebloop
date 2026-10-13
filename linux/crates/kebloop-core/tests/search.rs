//! Port of `Tests/KebloopCoreTests/SearchTests.swift` and the 1,000-app ranking benchmark.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use kebloop_core::{AppSearch, AppSearchEntry, Searchable};

fn entry(id: &str, name: &str) -> AppSearchEntry {
    AppSearchEntry::new(id, name, format!("/Applications/{name}.app"))
}

fn usage(pairs: &[(&str, i64)]) -> HashMap<String, i64> {
    pairs
        .iter()
        .map(|&(id, count)| (id.to_owned(), count))
        .collect()
}

fn ids<'a>(results: &[&'a AppSearchEntry]) -> Vec<&'a str> {
    results.iter().map(|entry| entry.id.as_str()).collect()
}

fn ranked<'a>(query: &str, entries: &'a [AppSearchEntry]) -> Vec<&'a AppSearchEntry> {
    AppSearch::new().ranked(query, entries, &HashMap::new())
}

#[test]
fn exact_and_prefix_matches_stay_ahead_of_fuzzy_matches() {
    let entries = [
        entry("safari", "Safari"),
        entry("preview", "Safari Technology Preview"),
        entry("fuzzy", "System Activity File Archive"),
    ];
    let results = AppSearch::new().ranked("safari", &entries, &usage(&[("fuzzy", 50_000)]));
    assert_eq!(ids(&results), ["safari", "preview", "fuzzy"]);
}

#[test]
fn exact_alias_outranks_exact_application_name_even_with_high_usage() {
    let entries = [
        AppSearchEntry::new(
            "terminal",
            "Terminal",
            "/System/Applications/Utilities/Terminal.app",
        ),
        AppSearchEntry::new("ghostty", "Ghostty", "/Applications/Ghostty.app")
            .with_aliases(["terminal"]),
    ];
    let results = AppSearch::new().ranked("terminal", &entries, &usage(&[("terminal", 50_000)]));
    assert_eq!(ids(&results), ["ghostty", "terminal"]);
}

#[test]
fn alias_prefix_ranks_below_exact_name_and_above_name_prefix() {
    let entries = [
        AppSearchEntry::new("alias", "Ghostty", "/Applications/Ghostty.app")
            .with_aliases(["terminal"]),
        entry("exact", "Term"),
        entry("prefix", "Terminal Utility"),
    ];
    assert_eq!(ids(&ranked("term", &entries)), ["exact", "alias", "prefix"]);
}

#[test]
fn aliases_use_case_and_diacritic_insensitive_matching() {
    let entries = [
        AppSearchEntry::new("notes", "Notes", "/Applications/Notes.app").with_aliases(["Résumé"]),
    ];
    assert_eq!(ids(&ranked("resume", &entries)), ["notes"]);
}

#[test]
fn search_ignores_case_and_diacritics() {
    let entries = [entry("cafe", "Café"), entry("time", "CafeTime")];
    assert_eq!(ids(&ranked("CAFE", &entries)), ["cafe", "time"]);
}

#[test]
fn only_decomposable_diacritics_fold() {
    // Like Foundation's diacritic-insensitive comparison: ó and ź fold, ł (no decomposition) does not.
    let entries = [entry("lodz", "Łódź Maps"), entry("other", "Other")];
    assert_eq!(ids(&ranked("łodz", &entries)), ["lodz"]);
    assert!(ranked("lodz", &entries).is_empty());
}

#[test]
fn word_prefixes_can_match_words_after_the_first() {
    let entries = [
        entry("visual-studio", "Visual Studio Code"),
        entry("studio-code", "Studio Code"),
    ];
    assert_eq!(
        ids(&ranked("sto co", &entries)),
        ["studio-code", "visual-studio"]
    );
}

#[test]
fn fuzzy_subsequence_matches_initials() {
    let entries = [
        entry("vscode", "Visual Studio Code"),
        entry("other", "TextEdit"),
    ];
    assert_eq!(ids(&ranked("vsc", &entries)), ["vscode"]);
}

#[test]
fn usage_orders_the_unfiltered_list() {
    let entries = [entry("mail", "Mail"), entry("safari", "Safari")];
    let results = AppSearch::new().ranked("   ", &entries, &usage(&[("safari", 4), ("mail", 1)]));
    assert_eq!(ids(&results), ["safari", "mail"]);
}

#[test]
fn duplicate_names_use_stable_ids() {
    let entries = [entry("b", "Safari"), entry("a", "Safari")];
    assert_eq!(ids(&ranked("saf", &entries)), ["a", "b"]);
}

#[test]
fn custom_types_can_be_ranked_through_the_trait() {
    struct Command(&'static str, &'static str, Vec<String>);
    impl Searchable for Command {
        fn id(&self) -> &str {
            self.0
        }
        fn name(&self) -> &str {
            self.1
        }
        fn aliases(&self) -> &[String] {
            &self.2
        }
    }
    let commands = [
        Command("clipboard", "Clipboard History", vec!["schowek".into()]),
        Command("settings", "Settings", vec!["ustawienia".into()]),
    ];
    let results = AppSearch::new().ranked("ustaw", &commands, &HashMap::new());
    assert_eq!(
        results
            .iter()
            .map(|command| command.id())
            .collect::<Vec<_>>(),
        ["settings"]
    );
}

#[test]
fn thousand_app_search_performance() {
    let entries: Vec<AppSearchEntry> = (0..1000)
        .map(|i| entry(&format!("app-{i}"), &format!("Application Tool {i}")))
        .collect();
    let search = AppSearch::new();
    let usage = HashMap::new();

    let started = Instant::now();
    for _ in 0..10 {
        assert_eq!(search.ranked("at", &entries, &usage).len(), 1000);
    }
    let per_run = started.elapsed() / 10;
    // The Swift benchmark averaged ~9 ms; this bound only catches pathological regressions
    // and stays generous for unoptimized debug builds on slow CI machines.
    assert!(
        per_run < Duration::from_millis(500),
        "ranking 1,000 entries took {per_run:?}"
    );
}

#[test]
fn keywords_find_apps_but_never_outrank_names_or_aliases() {
    let entries = [
        entry("firefox", "Firefox").with_keywords(["Web Browser", "Internet", "WWW"]),
        entry("terminal", "Alacritty").with_keywords(["terminal", "console", "command prompt"]),
        entry("browser-lover", "Browser Notes"),
        entry("alias-app", "Notes Plus").with_aliases(["browser"]),
    ];
    // Word-prefix on a name beats a keyword; an exact alias beats everything.
    let results = ranked("browser", &entries);
    assert_eq!(ids(&results), ["alias-app", "browser-lover", "firefox"]);
    // A keyword alone is enough to find the app.
    assert_eq!(ids(&ranked("terminal", &entries)), ["terminal"]);
    assert_eq!(ids(&ranked("consol", &entries)), ["terminal"]);
    assert_eq!(ids(&ranked("internet", &entries)), ["firefox"]);
    // Keywords are not fuzzy: a scattered subsequence does not match.
    assert!(ranked("tnrt", &entries).is_empty());
}

#[test]
fn exact_name_beats_a_keyword_exact_match() {
    let entries = [
        entry("a", "Code").with_keywords(["editor"]),
        entry("b", "Editor").with_keywords(["code"]),
    ];
    assert_eq!(ids(&ranked("code", &entries)), ["a", "b"]);
}
