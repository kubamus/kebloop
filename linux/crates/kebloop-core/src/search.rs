//! Ranking for launcher results: exact matches, prefixes, word prefixes and fuzzy
//! subsequences, with diacritic folding, aliases and a small usage boost.
//!
//! The ranker works on anything implementing [`Searchable`], so applications, commands,
//! quicklinks and files share one implementation. Ties are broken deterministically.

use std::cmp::Ordering;
use std::collections::HashMap;

use unicode_normalization::UnicodeNormalization;

/// Something the launcher can rank by name and aliases.
pub trait Searchable {
    /// A stable identifier, used for usage lookup and as the final tie-break.
    fn id(&self) -> &str;
    /// The display name that is matched.
    fn name(&self) -> &str;
    /// Alternative names or keywords. An exact alias outranks an exact name.
    fn aliases(&self) -> &[String] {
        &[]
    }
    /// Secondary keywords (desktop-entry `Keywords=` and `GenericName=`). They only help when
    /// neither the name nor an alias matches well: every keyword match scores below name
    /// prefixes and word-prefix matches.
    fn keywords(&self) -> &[String] {
        &[]
    }
}

/// A plain searchable entry (an application, command, quicklink, file ...).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct AppSearchEntry {
    /// Stable identifier.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Path or target; not searched.
    pub path: String,
    /// Alternative names.
    pub aliases: Vec<String>,
    /// Secondary keywords, ranked below names and aliases.
    pub keywords: Vec<String>,
}

impl AppSearchEntry {
    /// Creates an entry without aliases.
    pub fn new(id: impl Into<String>, name: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            path: path.into(),
            aliases: Vec::new(),
            keywords: Vec::new(),
        }
    }

    /// Adds secondary keywords.
    #[must_use]
    pub fn with_keywords<I, S>(mut self, keywords: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.keywords = keywords.into_iter().map(Into::into).collect();
        self
    }

    /// Adds aliases.
    #[must_use]
    pub fn with_aliases<I, S>(mut self, aliases: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.aliases = aliases.into_iter().map(Into::into).collect();
        self
    }
}

impl Searchable for AppSearchEntry {
    fn id(&self) -> &str {
        &self.id
    }
    fn name(&self) -> &str {
        &self.name
    }
    fn aliases(&self) -> &[String] {
        &self.aliases
    }
    fn keywords(&self) -> &[String] {
        &self.keywords
    }
}

/// Ranks entries by exact matches, prefixes, word prefixes, and fuzzy subsequences.
#[derive(Debug, Clone, Copy, Default)]
pub struct AppSearch;

struct Scored<'a, T> {
    item: &'a T,
    normalized_name: String,
    score: i64,
    usage: i64,
}

impl AppSearch {
    /// Creates a ranker.
    pub fn new() -> Self {
        Self
    }

    /// Returns the matching `items`, best first. An empty (or punctuation-only) query returns
    /// all items ordered by usage and then by name. `usage` maps ids to launch counts.
    pub fn ranked<'a, T: Searchable>(
        &self,
        query: &str,
        items: &'a [T],
        usage: &HashMap<String, i64>,
    ) -> Vec<&'a T> {
        let normalized_query = normalize(query);
        let usage_of = |item: &T| usage.get(item.id()).copied().unwrap_or(0);

        if normalized_query.is_empty() {
            let mut prepared: Vec<Scored<'a, T>> = items
                .iter()
                .map(|item| Scored {
                    item,
                    normalized_name: normalize(item.name()),
                    score: 0,
                    usage: usage_of(item),
                })
                .collect();
            prepared.sort_by(|left, right| {
                right
                    .usage
                    .cmp(&left.usage)
                    .then_with(|| stable_name_order(left, right))
            });
            return prepared.into_iter().map(|scored| scored.item).collect();
        }

        let query_words = words(&normalized_query);
        let query_characters: Vec<char> = normalized_query.chars().collect();
        let mut scored: Vec<Scored<'a, T>> = items
            .iter()
            .filter_map(|item| {
                let normalized_name = normalize(item.name());
                let item_usage = usage_of(item);
                let name_score = match_score(
                    &normalized_query,
                    &query_characters,
                    &query_words,
                    &normalized_name,
                );
                let alias_score = item
                    .aliases()
                    .iter()
                    .filter_map(|alias| {
                        alias_match_score(&normalized_query, &query_characters, &query_words, alias)
                    })
                    .max();
                let keyword_score = item
                    .keywords()
                    .iter()
                    .filter_map(|keyword| {
                        keyword_match_score(
                            &normalized_query,
                            &query_characters,
                            &query_words,
                            keyword,
                        )
                    })
                    .max();
                let best_match = name_score.max(alias_score).max(keyword_score)?;
                Some(Scored {
                    item,
                    normalized_name,
                    score: best_match + usage_boost(item_usage),
                    usage: item_usage.max(0),
                })
            })
            .collect();
        scored.sort_by(|left, right| {
            right
                .score
                .cmp(&left.score)
                .then_with(|| right.usage.cmp(&left.usage))
                .then_with(|| stable_name_order(left, right))
        });
        scored.into_iter().map(|scored| scored.item).collect()
    }
}

fn stable_name_order<T: Searchable>(left: &Scored<'_, T>, right: &Scored<'_, T>) -> Ordering {
    left.normalized_name
        .cmp(&right.normalized_name)
        .then_with(|| left.item.name().cmp(right.item.name()))
        .then_with(|| left.item.id().cmp(right.item.id()))
}

fn match_score(
    query: &str,
    query_characters: &[char],
    query_words: &[String],
    name: &str,
) -> Option<i64> {
    if name == query {
        return Some(1_200);
    }
    if name.starts_with(query) {
        return Some(1_100 - clamp_len(name.chars().count() - query.chars().count(), 50));
    }
    let name_words = words(name);
    if let Some(score) = word_prefix_score(query_words, &name_words) {
        return Some(score);
    }
    subsequence_score(query_characters, name)
}

fn alias_match_score(
    query: &str,
    query_characters: &[char],
    query_words: &[String],
    alias: &str,
) -> Option<i64> {
    let normalized_alias = normalize(alias);
    if normalized_alias.is_empty() {
        return None;
    }
    // An exact alias should win over an exact application name. Prefix aliases
    // remain below exact names and above ordinary name prefixes.
    if normalized_alias == query {
        return Some(1_400);
    }
    if normalized_alias.starts_with(query) {
        return Some(
            1_170 - clamp_len(normalized_alias.chars().count() - query.chars().count(), 20),
        );
    }
    let alias_words = words(&normalized_alias);
    if let Some(score) = word_prefix_score(query_words, &alias_words) {
        return Some(score + 10);
    }
    subsequence_score(query_characters, &normalized_alias).map(|score| score + 15)
}

/// Keyword matches: exact, prefix and word-prefix only (no fuzzy matching, which would be too
/// noisy for free-form keywords). All scores stay below name prefix (1000+) and word-prefix
/// (850+) matches but above the weakest fuzzy name matches.
fn keyword_match_score(
    query: &str,
    _query_characters: &[char],
    query_words: &[String],
    keyword: &str,
) -> Option<i64> {
    let normalized = normalize(keyword);
    if normalized.is_empty() {
        return None;
    }
    if normalized == query {
        return Some(690);
    }
    if normalized.starts_with(query) {
        return Some(670 - clamp_len(normalized.chars().count() - query.chars().count(), 20));
    }
    word_prefix_score(query_words, &words(&normalized)).map(|_| 650)
}

fn clamp_len(value: usize, maximum: i64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX).min(maximum)
}

fn word_prefix_score(query_words: &[String], name_words: &[String]) -> Option<i64> {
    if query_words.is_empty() || name_words.is_empty() {
        return None;
    }
    let mut next_name_word = 0;
    let mut position_penalty = 0_i64;
    for query_word in query_words {
        let position = name_words
            .iter()
            .enumerate()
            .skip(next_name_word)
            .find(|(_, word)| word.starts_with(query_word.as_str()))
            .map(|(index, _)| index)?;
        position_penalty += i64::try_from(position).unwrap_or(0) * 12;
        next_name_word = position + 1;
    }
    let base = if query_words.len() > 1 { 950 } else { 900 };
    Some(base - position_penalty.min(100))
}

fn subsequence_score(query_characters: &[char], name: &str) -> Option<i64> {
    let name_characters: Vec<char> = name.chars().collect();
    if query_characters.is_empty() || query_characters.len() > name_characters.len() {
        return None;
    }

    let mut first_position: Option<usize> = None;
    let mut previous_position: Option<usize> = None;
    let (mut gaps, mut word_starts, mut adjacent_pairs) = (0_i64, 0_i64, 0_i64);
    let mut search_start = 0;
    for &character in query_characters {
        let position = search_start
            + name_characters
                .get(search_start..)?
                .iter()
                .position(|&c| c == character)?;
        match (first_position, previous_position) {
            (None, _) => first_position = Some(position),
            (Some(_), Some(previous)) => {
                gaps += i64::try_from(position - previous - 1).unwrap_or(0);
                if position == previous + 1 {
                    adjacent_pairs += 1;
                }
            }
            (Some(_), None) => {}
        }
        let starts_word = position == 0
            || name_characters
                .get(position - 1)
                .is_some_and(|c| !c.is_alphabetic() && !c.is_numeric());
        if starts_word {
            word_starts += 1;
        }
        previous_position = Some(position);
        search_start = position + 1;
    }

    let first = i64::try_from(first_position?).unwrap_or(0);
    Some(
        700 - (first * 2).min(50) - (gaps * 4).min(80)
            + (word_starts * 10).min(30)
            + (adjacent_pairs * 5).min(20),
    )
}

fn usage_boost(count: i64) -> i64 {
    if count <= 0 {
        return 0;
    }
    // Usage helps choose between similar matches without outranking a stronger match class.
    let boost = ((count as f64) + 1.0).log2() * 3.0;
    (boost as i64).min(24)
}

/// Folds case and diacritics and reduces the text to space-separated alphanumeric words,
/// so `"Café  Time!"` becomes `"cafe time"`.
pub fn normalize(text: &str) -> String {
    words(&fold(text)).join(" ")
}

/// Case- and diacritic-insensitive folding, like Foundation's `caseInsensitive` +
/// `diacriticInsensitive` comparison: combining marks of the canonical decomposition are
/// removed and case is folded. Letters without a decomposition (`ł`, `ø`, `đ`, `ß`, `æ`, `œ`)
/// are not mapped to plain letters.
pub fn fold(text: &str) -> String {
    text.nfd()
        .filter(|character| !('\u{0300}'..='\u{036F}').contains(character))
        .flat_map(char::to_lowercase)
        .collect()
}

fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folding_removes_case_and_diacritics() {
        assert_eq!(normalize("  Café  Time! "), "cafe time");
        assert_eq!(normalize("Łódź Straße"), "łodz straße");
        assert_eq!(normalize("Résumé"), "resume");
        assert_eq!(normalize("!!!"), "");
    }

    #[test]
    fn usage_boost_is_bounded() {
        assert_eq!(usage_boost(0), 0);
        assert_eq!(usage_boost(-5), 0);
        assert_eq!(usage_boost(1), 3);
        assert_eq!(usage_boost(i64::MAX), 24);
    }
}
