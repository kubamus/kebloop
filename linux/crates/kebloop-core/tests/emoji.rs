//! Port of `Tests/KebloopCoreTests/EmojiTests.swift`.

use std::collections::HashSet;

use kebloop_core::EmojiCatalog;

#[test]
fn catalog_has_unique_symbols_and_useful_breadth() {
    let entries = EmojiCatalog::entries();
    assert_eq!(entries.len(), 3_963);
    let ids: HashSet<_> = entries.iter().map(|entry| entry.id()).collect();
    assert_eq!(ids.len(), entries.len());
    let categories: HashSet<_> = entries.iter().map(|entry| entry.category).collect();
    for required in ["Smileys", "Animals", "Food", "Symbols", "Flags"] {
        assert!(categories.contains(required), "missing category {required}");
    }
    assert_eq!(EmojiCatalog::categories().len(), categories.len());
    assert!(EmojiCatalog::entries_in("Flags").all(|entry| entry.category == "Flags"));
}

#[test]
fn unicode_18_catalog_includes_fully_qualified_sequences_and_variants() {
    assert_eq!(EmojiCatalog::UNICODE_VERSION, "18.0");

    let symbols: HashSet<_> = EmojiCatalog::entries()
        .iter()
        .map(|entry| entry.symbol)
        .collect();
    assert!(symbols.contains("🫫"), "Emoji 18.0 cracking face");
    assert!(symbols.contains("👩🏽‍⚕️"), "profession with skin tone");
    assert!(symbols.contains("👨‍👩‍👧‍👦"), "family ZWJ sequence");
    assert!(
        symbols.contains("🏴\u{E0067}\u{E0062}\u{E0073}\u{E0063}\u{E0074}\u{E007F}"),
        "Scotland flag"
    );
    assert!(symbols.contains("1️⃣"), "keycap");
    assert!(!symbols.contains("🏻"), "standalone modifiers are excluded");
}

#[test]
fn fully_qualified_symbols_have_no_variation_selector_only_duplicates() {
    let symbols: Vec<_> = EmojiCatalog::entries()
        .iter()
        .map(|entry| entry.symbol)
        .collect();
    let without_selectors: HashSet<String> = symbols
        .iter()
        .map(|symbol| symbol.replace('\u{FE0F}', ""))
        .collect();
    assert_eq!(without_selectors.len(), symbols.len());
    assert_eq!(
        EmojiCatalog::entry_for_symbol("☕").map(|entry| entry.symbol),
        Some("☕")
    );
    assert_eq!(
        EmojiCatalog::entry_for_symbol("❤️").map(|entry| entry.symbol),
        Some("❤️")
    );
}

#[test]
fn unicode_categories_map_into_picker_sections() {
    let category =
        |symbol: &str| EmojiCatalog::entry_for_symbol(symbol).map(|entry| entry.category);
    assert_eq!(category("🐕"), Some("Animals"));
    assert_eq!(category("🌸"), Some("Nature"));
    assert_eq!(category("🚆"), Some("Travel"));
    assert_eq!(category("🗼"), Some("Places"));
    assert_eq!(category("🏳️‍🌈"), Some("Flags"));
    assert_eq!(category("🏴‍☠️"), Some("Flags"));
}

#[test]
fn curated_names_and_aliases_survive_and_the_license_is_embedded() {
    let rofl = EmojiCatalog::entry_for_symbol("🤣").expect("curated entry");
    assert_eq!(rofl.name, "rolling on the floor laughing");
    assert!(rofl.keywords.contains(&"rofl"));
    // Generated entries have the Unicode name and no aliases.
    let cracking = EmojiCatalog::entry_for_symbol("🫫").expect("generated entry");
    assert!(cracking.keywords.is_empty());
    assert!(!cracking.name.is_empty());
    assert!(EmojiCatalog::UNICODE_LICENSE.contains("UNICODE LICENSE"));
    // The picker sections are exactly the documented ones, in first-appearance order.
    let categories = EmojiCatalog::categories();
    for expected in [
        "Smileys",
        "People",
        "Animals",
        "Nature",
        "Food",
        "Travel",
        "Places",
        "Activities",
        "Objects",
        "Symbols",
        "Flags",
    ] {
        assert!(categories.contains(&expected), "missing {expected}");
    }
}

#[test]
fn search_finds_names_and_ranks_specific_matches_first() {
    let results = EmojiCatalog::search("heart");
    assert_eq!(results.first().map(|entry| entry.symbol), Some("🫶"));
    assert!(results.iter().any(|entry| entry.symbol == "💔"));
}

#[test]
fn search_finds_aliases_without_case_sensitivity() {
    let results = EmojiCatalog::search("ROFL");
    assert_eq!(results.first().map(|entry| entry.symbol), Some("🤣"));
}

#[test]
fn search_finds_category_and_emoji_symbol() {
    assert!(
        EmojiCatalog::search("japan")
            .iter()
            .any(|entry| entry.symbol == "🇯🇵")
    );
    assert_eq!(
        EmojiCatalog::search("🧋").first().map(|entry| entry.symbol),
        Some("🧋")
    );
}

#[test]
fn empty_search_returns_whole_catalog_and_unknown_search_returns_nothing() {
    assert_eq!(
        EmojiCatalog::search("  ").len(),
        EmojiCatalog::entries().len()
    );
    assert!(EmojiCatalog::search("unfindable-emoji-query").is_empty());
}
