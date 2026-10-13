//! The offline emoji catalog for the launcher's emoji picker.
//!
//! The catalog is every fully-qualified RGI emoji of Unicode's bundled `emoji-test.txt`
//! (Emoji 18.0), with the hand-tuned names, search aliases and ordering of familiar emoji
//! kept from the curated table. It is parsed once, lazily, on first use (a few milliseconds);
//! the strings are leaked so entries are `'static` for the lifetime of the process.
//!
//! The Unicode data is covered by the Unicode License V3, which is embedded as
//! [`EmojiCatalog::UNICODE_LICENSE`] so the application can show it.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use crate::emoji_data::CURATED;
use crate::search::fold;

const EMOJI_TEST: &str = include_str!("../../../../Sources/KebloopCore/Resources/emoji-test.txt");
const LICENSE: &str = include_str!("../../../../Sources/KebloopCore/Resources/unicode-license.txt");

/// One emoji with its search metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EmojiEntry {
    /// The emoji itself (also its identifier).
    pub symbol: &'static str,
    /// Lowercase English name.
    pub name: &'static str,
    /// Category label such as `Smileys`, `Animals` or `Flags`.
    pub category: &'static str,
    /// Extra search words.
    pub keywords: &'static [&'static str],
}

impl EmojiEntry {
    /// The stable identifier (the symbol).
    pub fn id(&self) -> &'static str {
        self.symbol
    }
}

struct Catalog {
    entries: Vec<EmojiEntry>,
    by_symbol: HashMap<&'static str, usize>,
    categories: Vec<&'static str>,
    curated: HashSet<&'static str>,
}

fn catalog() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        let official = load_official_entries();
        let official_categories: HashMap<&str, &str> = official
            .iter()
            .map(|entry| (entry.symbol, entry.category))
            .collect();
        let curated: HashSet<&'static str> = CURATED.iter().map(|entry| entry.symbol).collect();

        // Keep the hand-tuned display names, aliases and initial ordering for familiar emoji.
        // Unicode's groups remain the source of truth for category placement.
        let mut entries: Vec<EmojiEntry> = CURATED
            .iter()
            .map(|entry| EmojiEntry {
                category: official_categories
                    .get(entry.symbol)
                    .copied()
                    .unwrap_or(entry.category),
                ..*entry
            })
            .collect();
        entries.extend(
            official
                .iter()
                .filter(|entry| !curated.contains(entry.symbol))
                .copied(),
        );

        let by_symbol = entries
            .iter()
            .enumerate()
            .map(|(index, entry)| (entry.symbol, index))
            .collect();
        let mut categories: Vec<&'static str> = Vec::new();
        for entry in &entries {
            if !categories.contains(&entry.category) {
                categories.push(entry.category);
            }
        }
        Catalog {
            entries,
            by_symbol,
            categories,
            curated,
        }
    })
}

/// The fully-qualified records of `emoji-test.txt`, in file (CLDR) order.
fn load_official_entries() -> Vec<EmojiEntry> {
    let mut group = "";
    let mut subgroup = "";
    let mut parsed = Vec::with_capacity(4_000);
    for line in EMOJI_TEST.lines() {
        if let Some(name) = line.strip_prefix("# group: ") {
            group = name;
            continue;
        }
        if let Some(name) = line.strip_prefix("# subgroup: ") {
            subgroup = name;
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        let Some((data, comment)) = line.split_once('#') else {
            continue;
        };
        let mut fields = data.split(';');
        let (Some(code_points), Some(status), None) = (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        if status.trim() != "fully-qualified" {
            continue;
        }
        let scalars: Option<String> = code_points
            .split_whitespace()
            .map(|code| u32::from_str_radix(code, 16).ok().and_then(char::from_u32))
            .collect();
        let Some(symbol) = scalars.filter(|symbol| !symbol.is_empty()) else {
            continue;
        };
        // "<emoji> E1.0 grinning face": skip the symbol and the version.
        let words: Vec<&str> = comment.split_whitespace().collect();
        if words.len() < 3 {
            continue;
        }
        let Some(category) = category_for(group, subgroup) else {
            continue;
        };
        parsed.push(EmojiEntry {
            symbol: leak(symbol),
            name: leak(words[2..].join(" ")),
            category,
            keywords: &[],
        });
    }
    parsed
}

fn leak(text: String) -> &'static str {
    Box::leak(text.into_boxed_str())
}

/// Maps Unicode's group/subgroup to the picker's section.
fn category_for(group: &str, subgroup: &str) -> Option<&'static str> {
    Some(match group {
        "Smileys & Emotion" => "Smileys",
        "People & Body" => "People",
        "Animals & Nature" => {
            if subgroup.starts_with("animal-") {
                "Animals"
            } else {
                "Nature"
            }
        }
        "Food & Drink" => "Food",
        "Travel & Places" => {
            if subgroup.starts_with("place-") || subgroup == "hotel" {
                "Places"
            } else if subgroup == "sky & weather" {
                "Nature"
            } else {
                "Travel"
            }
        }
        "Activities" => "Activities",
        "Objects" => "Objects",
        "Symbols" => "Symbols",
        "Flags" => "Flags",
        _ => return None,
    })
}

/// The emoji catalog and its search.
#[derive(Debug, Clone, Copy)]
pub struct EmojiCatalog;

impl EmojiCatalog {
    /// Unicode Emoji version of the bundled data.
    pub const UNICODE_VERSION: &'static str = "18.0";

    /// The Unicode License V3 that covers the bundled emoji data (to be shown in About/Help).
    pub const UNICODE_LICENSE: &'static str = LICENSE;

    /// Every entry, in catalog order: curated emoji first, then the rest in Unicode order.
    pub fn entries() -> &'static [EmojiEntry] {
        &catalog().entries
    }

    /// The entry for exactly `symbol` (fully-qualified, with its variation selectors).
    pub fn entry_for_symbol(symbol: &str) -> Option<&'static EmojiEntry> {
        let catalog = catalog();
        catalog
            .by_symbol
            .get(symbol)
            .and_then(|&index| catalog.entries.get(index))
    }

    /// Categories in first-appearance order.
    pub fn categories() -> Vec<&'static str> {
        catalog().categories.clone()
    }

    /// Entries of one category, in catalog order.
    pub fn entries_in(category: &str) -> impl Iterator<Item = &'static EmojiEntry> {
        let category = category.to_owned();
        catalog()
            .entries
            .iter()
            .filter(move |entry| entry.category == category)
    }

    /// Searches by emoji name, category, keywords or the symbol itself. Results are stable
    /// and most relevant first; an empty query returns the whole catalog.
    pub fn search(query: &str) -> Vec<&'static EmojiEntry> {
        let catalog = catalog();
        let entries = &catalog.entries;
        let raw_query = query.trim();
        if raw_query.is_empty() {
            return entries.iter().collect();
        }
        let normalized_query = normalize(query);
        if normalized_query.is_empty() {
            return entries
                .iter()
                .filter(|entry| entry.symbol.contains(raw_query))
                .collect();
        }
        let query_words: Vec<&str> = normalized_query.split(' ').collect();
        let query_length = normalized_query.chars().count();

        let mut scored: Vec<(&'static EmojiEntry, i64)> = prepared()
            .iter()
            .zip(entries)
            .filter_map(|(prepared, entry)| {
                let Prepared {
                    name,
                    keywords,
                    haystack,
                } = prepared;
                let score = if *name == normalized_query {
                    1_000
                } else if name.starts_with(normalized_query.as_str()) {
                    let extra = name.chars().count() - query_length;
                    900 - i64::try_from(extra).unwrap_or(80).min(80)
                } else if name.contains(normalized_query.as_str()) {
                    // Like the original, the penalty is the matched length, not its position.
                    800 - i64::try_from(query_length).unwrap_or(80).min(80)
                } else if query_words
                    .iter()
                    .all(|word| name.split(' ').any(|name_word| name_word.starts_with(word)))
                {
                    700
                } else if keywords.contains(normalized_query.as_str()) {
                    600
                } else if query_words.iter().all(|word| haystack.contains(word)) {
                    500
                } else {
                    return None;
                };
                // Curated entries win ties against the generated Unicode names.
                let curated_bonus = if catalog.curated.contains(entry.symbol) {
                    2
                } else {
                    0
                };
                Some((entry, score + curated_bonus))
            })
            .collect();
        scored.sort_by(|(left, left_score), (right, right_score)| {
            right_score
                .cmp(left_score)
                .then_with(|| left.category.cmp(right.category))
                .then_with(|| left.name.cmp(right.name))
        });
        scored.into_iter().map(|(entry, _)| entry).collect()
    }
}

/// Search-ready, pre-normalized text of one entry (computed once).
struct Prepared {
    name: String,
    keywords: String,
    haystack: String,
}

fn prepared() -> &'static [Prepared] {
    static PREPARED: OnceLock<Vec<Prepared>> = OnceLock::new();
    PREPARED.get_or_init(|| {
        catalog()
            .entries
            .iter()
            .map(|entry| {
                let name = normalize(entry.name);
                let keywords = normalize(&entry.keywords.join(" "));
                let category = normalize(entry.category);
                let haystack = format!("{name} {keywords} {category}");
                Prepared {
                    name,
                    keywords,
                    haystack,
                }
            })
            .collect()
    })
}

fn normalize(value: &str) -> String {
    fold(value)
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}
