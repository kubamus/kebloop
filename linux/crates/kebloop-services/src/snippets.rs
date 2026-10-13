//! Text snippets with `{name}` placeholders, `{name:default}` defaults and the dynamic values
//! `{date}`, `{time}` and `{clipboard}`.
//!
//! Port of `Sources/Kebloop/SnippetStore.swift` (the model, not the stored list) plus the
//! platform-neutral part of `SnippetExpansionService.swift`: the keyword matcher.
//!
//! Keywords expand through the optional AT-SPI accessibility service in supported text
//! fields. This module provides the matcher independently of desktop availability.

use std::collections::{BTreeMap, HashSet};
use std::sync::OnceLock;

use jiff::Timestamp;
use jiff::tz::TimeZone;
use kebloop_core::Locale;
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::error::ConfigError;
use crate::util::new_id;

/// Maximum snippet name length in characters.
pub const MAX_NAME_LENGTH: usize = 200;
/// Maximum template size in UTF-8 bytes.
pub const MAX_TEMPLATE_BYTES: usize = 100_000;
/// Maximum keyword length in UTF-8 bytes.
pub const MAX_KEYWORD_BYTES: usize = 64;

/// Placeholder names that are filled in locally instead of being prompted for.
const DYNAMIC_NAMES: [&str; 3] = ["date", "time", "clipboard"];

/// A reusable text template.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Snippet {
    /// Stable identifier.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Template text containing `{placeholder}` / `{placeholder:default}` tokens.
    pub template: String,
    /// A space-terminated trigger for optional automatic expansion. Optional so snippets in
    /// existing configuration files continue to decode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyword: Option<String>,
}

/// One `{name}` or `{name:default}` token of a template.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Token<'a> {
    start: usize,
    end: usize,
    name: &'a str,
    default: Option<&'a str>,
}

fn token_pattern() -> Option<&'static Regex> {
    static PATTERN: OnceLock<Option<Regex>> = OnceLock::new();
    PATTERN
        .get_or_init(|| Regex::new(r"\{([A-Za-z][A-Za-z0-9_]*)(?::([^{}]*))?\}").ok())
        .as_ref()
}

fn tokens(template: &str) -> Vec<Token<'_>> {
    let Some(pattern) = token_pattern() else {
        return Vec::new();
    };
    pattern
        .captures_iter(template)
        .filter_map(|captures| {
            let whole = captures.get(0)?;
            Some(Token {
                start: whole.start(),
                end: whole.end(),
                name: captures.get(1)?.as_str(),
                default: captures.get(2).map(|m| m.as_str()),
            })
        })
        .collect()
}

fn is_dynamic(name: &str) -> bool {
    DYNAMIC_NAMES.contains(&name)
}

/// Everything `{date}`, `{time}` and `{clipboard}` depend on, injected so rendering is
/// deterministic. The clipboard is read lazily, only when the template uses `{clipboard}`.
pub struct RenderContext<'a> {
    /// The instant used for `{date}` and `{time}`.
    pub now: Timestamp,
    /// Formats `{date}` (medium style) and `{time}` (short style).
    pub locale: Locale,
    /// The zone in which `now` is shown.
    pub time_zone: TimeZone,
    /// Returns the current clipboard text (empty when there is none).
    pub clipboard: &'a dyn Fn() -> String,
}

fn no_clipboard() -> String {
    String::new()
}

impl RenderContext<'static> {
    /// The system clock and time zone, `en_US` formats and an empty clipboard. Callers that
    /// can read the clipboard or know the user's locale should build the context themselves.
    pub fn system() -> Self {
        Self {
            now: Timestamp::now(),
            locale: Locale::default(),
            time_zone: TimeZone::system(),
            clipboard: &no_clipboard,
        }
    }
}

impl Snippet {
    /// New snippet with a generated id and no keyword.
    pub fn new(name: impl Into<String>, template: impl Into<String>) -> Self {
        Self {
            id: new_id(),
            name: name.into(),
            template: template.into(),
            keyword: None,
        }
    }

    /// The same snippet with an expansion keyword.
    #[must_use]
    pub fn with_keyword(mut self, keyword: impl Into<String>) -> Self {
        self.keyword = Some(keyword.into());
        self
    }

    /// Distinct prompted placeholder names in order of first appearance (`{date}`, `{time}`
    /// and `{clipboard}` are filled in locally and are not listed).
    pub fn placeholders(&self) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        for token in tokens(&self.template) {
            if !is_dynamic(token.name) && !names.iter().any(|n| n == token.name) {
                names.push(token.name.to_owned());
            }
        }
        names
    }

    /// Default values from `{name:default}` tokens; the first default of a name wins.
    pub fn placeholder_defaults(&self) -> BTreeMap<String, String> {
        let mut defaults = BTreeMap::new();
        for token in tokens(&self.template) {
            if is_dynamic(token.name) {
                continue;
            }
            if let Some(default) = token.default {
                defaults
                    .entry(token.name.to_owned())
                    .or_insert_with(|| default.to_owned());
            }
        }
        defaults
    }

    /// Whether every prompted placeholder has a default, so keyword expansion needs no input.
    pub fn can_expand_automatically(&self) -> bool {
        let defaults = self.placeholder_defaults();
        self.placeholders().iter().all(|n| defaults.contains_key(n))
    }

    /// Substitute values, using the system clock, `en_US` formats and an empty clipboard for
    /// the dynamic values. See [`Snippet::render_with`].
    pub fn render(&self, values: &BTreeMap<String, String>) -> Result<String, ConfigError> {
        self.render_with(values, &RenderContext::system())
    }

    /// Substitute values. Repeated placeholders share one value; braces inside values stay
    /// literal. A missing value falls back to the placeholder's default.
    pub fn render_with(
        &self,
        values: &BTreeMap<String, String>,
        context: &RenderContext<'_>,
    ) -> Result<String, ConfigError> {
        let defaults = self.placeholder_defaults();
        if !self
            .placeholders()
            .iter()
            .all(|name| values.contains_key(name) || defaults.contains_key(name))
        {
            return Err(ConfigError::invalid("Fill every snippet placeholder."));
        }
        let tokens = tokens(&self.template);
        if tokens
            .iter()
            .any(|token| is_dynamic(token.name) && token.default.is_some())
        {
            return Err(ConfigError::invalid(
                "Dynamic snippet values cannot have defaults.",
            ));
        }

        let local = context.now.to_zoned(context.time_zone.clone());
        let clamp = |value: i8| u8::try_from(value).unwrap_or(0);
        let rendered_date = || {
            context.locale.format_date(
                i32::from(local.year()),
                clamp(local.month()),
                clamp(local.day()),
                None,
            )
        };
        let rendered_time = || {
            context
                .locale
                .format_time_short(clamp(local.hour()), clamp(local.minute()))
        };

        let mut output = String::with_capacity(self.template.len());
        let mut cursor = 0;
        for token in &tokens {
            output.push_str(&self.template[cursor..token.start]);
            match token.name {
                "date" => output.push_str(&rendered_date()),
                "time" => output.push_str(&rendered_time()),
                "clipboard" => output.push_str(&(context.clipboard)()),
                name => output.push_str(
                    values
                        .get(name)
                        .or_else(|| defaults.get(name))
                        .map(String::as_str)
                        .or(token.default)
                        .unwrap_or(""),
                ),
            }
            cursor = token.end;
        }
        output.push_str(&self.template[cursor..]);
        Ok(output)
    }

    /// Check naming, size and placeholder rules.
    pub fn validate(&self) -> Result<(), ConfigError> {
        let ok = !self.id.is_empty()
            && !self.name.trim().is_empty()
            && self.name.chars().count() <= MAX_NAME_LENGTH
            && !self.template.is_empty()
            && self.template.len() <= MAX_TEMPLATE_BYTES;
        if !ok {
            return Err(ConfigError::invalid(
                "A snippet needs a name and text (up to 100 KB).",
            ));
        }
        if let Some(keyword) = &self.keyword {
            let printable = keyword.bytes().all(|b| (33..=126).contains(&b));
            if keyword.is_empty() || keyword.len() > MAX_KEYWORD_BYTES || !printable {
                return Err(ConfigError::invalid(
                    "A snippet keyword must use 1–64 printable ASCII characters with no spaces.",
                ));
            }
        }
        let tokens = tokens(&self.template);
        let mut seen: Vec<(&str, &str)> = Vec::new();
        for token in &tokens {
            if is_dynamic(token.name) {
                continue;
            }
            let Some(default) = token.default else {
                continue;
            };
            if let Some((_, previous)) = seen.iter().find(|(name, _)| *name == token.name)
                && *previous != default
            {
                return Err(ConfigError::invalid(format!(
                    "Repeated placeholder {{{}}} must use the same default value.",
                    token.name
                )));
            }
            seen.push((token.name, default));
        }
        if tokens
            .iter()
            .any(|token| is_dynamic(token.name) && token.default.is_some())
        {
            return Err(ConfigError::invalid(
                "Dynamic snippet values cannot have defaults.",
            ));
        }
        Ok(())
    }

    /// Keywords must be unique ignoring ASCII letter case.
    pub fn validate_unique_keywords(snippets: &[Snippet]) -> Result<(), ConfigError> {
        let mut seen: HashSet<String> = HashSet::new();
        for keyword in snippets.iter().filter_map(|s| s.keyword.as_deref()) {
            if !seen.insert(keyword.to_lowercase()) {
                return Err(ConfigError::invalid(
                    "Snippet keywords must be unique, ignoring letter case.",
                ));
            }
        }
        Ok(())
    }
}

/// Create or update a snippet. When updating, the existing keyword is kept; use
/// [`save_snippet_with_keyword`] to set or clear it. Returns its id; nothing changes on
/// validation failure.
pub fn save_snippet(
    snippets: &mut Vec<Snippet>,
    id: Option<&str>,
    name: &str,
    template: &str,
) -> Result<String, ConfigError> {
    let existing = id
        .and_then(|id| snippets.iter().find(|s| s.id == id))
        .and_then(|s| s.keyword.clone());
    save_snippet_with_keyword(snippets, id, name, template, existing.as_deref())
}

/// Create or update a snippet including its expansion keyword (blank means none). Keywords
/// must be unique ignoring case; nothing changes on validation failure.
pub fn save_snippet_with_keyword(
    snippets: &mut Vec<Snippet>,
    id: Option<&str>,
    name: &str,
    template: &str,
    keyword: Option<&str>,
) -> Result<String, ConfigError> {
    let keyword = keyword.map(str::trim).filter(|k| !k.is_empty());
    let snippet = Snippet {
        id: id.map(str::to_string).unwrap_or_else(new_id),
        name: name.trim().to_string(),
        template: template.to_string(),
        keyword: keyword.map(str::to_owned),
    };
    snippet.validate()?;
    let id = snippet.id.clone();
    let mut updated = snippets.clone();
    match updated.iter_mut().find(|s| s.id == snippet.id) {
        Some(existing) => *existing = snippet,
        None => updated.push(snippet),
    }
    Snippet::validate_unique_keywords(&updated)?;
    *snippets = updated;
    Ok(id)
}

/// Remove a snippet by id.
pub fn remove_snippet(snippets: &mut Vec<Snippet>, id: &str) {
    snippets.retain(|s| s.id != id);
}

/// Keeps only a short in-memory tail of typed characters and resolves an exact,
/// space-terminated snippet keyword. A plain value type, testable without any input hook.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnippetKeywordMatcher {
    buffer: Vec<char>,
    maximum_buffer_length: usize,
}

impl Default for SnippetKeywordMatcher {
    fn default() -> Self {
        Self::new(128)
    }
}

impl SnippetKeywordMatcher {
    /// A matcher that remembers at most `maximum_buffer_length` (at least one) characters.
    pub fn new(maximum_buffer_length: usize) -> Self {
        Self {
            buffer: Vec::new(),
            maximum_buffer_length: maximum_buffer_length.max(1),
        }
    }

    /// The remembered typing tail.
    pub fn buffer(&self) -> String {
        self.buffer.iter().collect()
    }

    /// Forget everything (focus, application or navigation changed).
    pub fn reset(&mut self) {
        self.buffer.clear();
    }

    /// Records one typed character. Returns the snippet to expand when `character` is the
    /// space that completes an exact (case-insensitive) keyword that does not continue a
    /// word and that needs no prompted values; the buffer is then cleared.
    pub fn record<'a>(&mut self, character: char, snippets: &'a [Snippet]) -> Option<&'a Snippet> {
        if character == ' ' {
            let mut candidates: Vec<(&Snippet, Vec<char>)> = snippets
                .iter()
                .filter_map(|snippet| {
                    let keyword = snippet.keyword.as_deref().filter(|k| !k.is_empty())?;
                    Some((snippet, keyword.chars().collect()))
                })
                .collect();
            candidates.sort_by_key(|(_, keyword)| std::cmp::Reverse(keyword.len()));

            for (snippet, keyword) in candidates {
                if self.buffer.len() < keyword.len() {
                    continue;
                }
                let split = self.buffer.len() - keyword.len();
                let trigger: String = self.buffer[split..].iter().collect();
                let wanted: String = keyword.iter().collect();
                if trigger.to_lowercase() != wanted.to_lowercase() {
                    continue;
                }
                if let Some(previous) = self.buffer[..split].last()
                    && (previous.is_alphanumeric() || *previous == '_')
                {
                    continue;
                }
                self.reset();
                return snippet.can_expand_automatically().then_some(snippet);
            }
        }

        self.buffer.push(character);
        if self.buffer.len() > self.maximum_buffer_length {
            let excess = self.buffer.len() - self.maximum_buffer_length;
            self.buffer.drain(..excess);
        }
        None
    }
}

/// How an expansion replaces the typed keyword.
pub struct SnippetExpansionPlan;

impl SnippetExpansionPlan {
    /// The terminal space has already been suppressed, so replacement must erase only the
    /// keyword that reached the target application.
    pub fn erase_count(keyword: &str) -> usize {
        keyword.chars().count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn values(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn placeholder_order_and_literal_values() {
        let snippet = Snippet::new("Hello", "Hi {name}, {name}! {message}");
        assert_eq!(snippet.placeholders(), ["name", "message"]);
        let rendered = snippet
            .render(&values(&[("name", "{message}"), ("message", "Zażółć 😀")]))
            .unwrap();
        assert_eq!(rendered, "Hi {message}, {message}! Zażółć 😀");
        assert!(snippet.render(&values(&[("name", "A")])).is_err());
    }

    #[test]
    fn non_placeholders_are_left_alone() {
        let snippet = Snippet::new("x", "{1bad} {ok_1} {} {unterminated {a b} }");
        assert_eq!(snippet.placeholders(), ["ok_1"]);
        assert_eq!(
            snippet.render(&values(&[("ok_1", "V")])).unwrap(),
            "{1bad} V {} {unterminated {a b} }"
        );
    }

    #[test]
    fn editing_and_validation() {
        let mut snippets = Vec::new();
        let id = save_snippet(&mut snippets, None, "Work", "Work {name}").unwrap();
        save_snippet(&mut snippets, Some(&id), "Edited", "Edited {name}").unwrap();
        assert_eq!(snippets.len(), 1);
        assert_eq!(snippets[0].name, "Edited");
        for (name, text) in [
            (" ", "valid".to_string()),
            ("Valid", String::new()),
            ("Valid", "x".repeat(100_001)),
        ] {
            assert!(save_snippet(&mut snippets, None, name, &text).is_err());
        }
        assert!(save_snippet(&mut snippets, None, &"n".repeat(201), "text").is_err());
        assert_eq!(snippets.len(), 1);
        remove_snippet(&mut snippets, &id);
        assert!(snippets.is_empty());
    }

    fn context<'a>(clipboard: &'a dyn Fn() -> String) -> RenderContext<'a> {
        RenderContext {
            now: "2024-01-02T03:04:00Z".parse().expect("timestamp"),
            locale: Locale::en_us(),
            time_zone: TimeZone::UTC,
            clipboard,
        }
    }

    #[test]
    fn placeholder_defaults_and_dynamic_values_render_without_prompting() {
        let snippet = Snippet::new(
            "Reply",
            "Hi {name:friend}. {name} {date} {time} {clipboard}",
        );
        let copied = || "copied".to_owned();
        let actual = snippet
            .render_with(&BTreeMap::new(), &context(&copied))
            .unwrap();
        assert_eq!(actual, "Hi friend. friend Jan 2, 2024 3:04 AM copied");
        assert_eq!(snippet.placeholders(), ["name"]);
        assert_eq!(
            snippet.placeholder_defaults(),
            values(&[("name", "friend")])
        );
        assert!(snippet.can_expand_automatically());
        let prompted = snippet
            .render_with(&values(&[("name", "pal")]), &context(&copied))
            .unwrap();
        assert!(prompted.starts_with("Hi pal. pal"), "{prompted}");

        let unresolved = Snippet::new("Prompt", "Hello {name} {other:default}");
        assert!(!unresolved.can_expand_automatically());
        assert!(
            unresolved
                .render_with(&BTreeMap::new(), &context(&copied))
                .is_err()
        );
        assert!(Snippet::new("Bad", "{date:tomorrow}").validate().is_err());
        assert!(Snippet::new("Bad", "{name:a} {name:b}").validate().is_err());
        assert!(
            Snippet::new("Ok", "{name:a} {name:a} {name}")
                .validate()
                .is_ok()
        );
    }

    #[test]
    fn the_clipboard_is_only_read_when_the_template_uses_it() {
        let reads = std::cell::Cell::new(0);
        let provider = || {
            reads.set(reads.get() + 1);
            "clip".to_owned()
        };
        let plain = Snippet::new("Plain", "no tokens {date}");
        plain
            .render_with(&BTreeMap::new(), &context(&provider))
            .unwrap();
        assert_eq!(reads.get(), 0);
        let pasted = Snippet::new("Paste", "[{clipboard}][{clipboard}]");
        assert_eq!(
            pasted
                .render_with(&BTreeMap::new(), &context(&provider))
                .unwrap(),
            "[clip][clip]"
        );
        assert_eq!(reads.get(), 2);
    }

    #[test]
    fn defaults_may_be_empty_and_values_override_them() {
        let snippet = Snippet::new("x", "A{suffix:}B {n:1}{n}");
        assert!(snippet.can_expand_automatically());
        assert_eq!(snippet.render(&BTreeMap::new()).unwrap(), "AB 11");
        assert_eq!(
            snippet
                .render(&values(&[("suffix", "-"), ("n", "7")]))
                .unwrap(),
            "A-B 77"
        );
    }

    #[test]
    fn keyword_validation_and_bounded_space_trigger() {
        let address = Snippet::new("Address", "42 Main St").with_keyword(";addr");
        let prompted = Snippet::new("Prompted", "Hello {name}").with_keyword(";hello");
        let snippets = [address.clone(), prompted.clone()];
        assert!(Snippet::validate_unique_keywords(&snippets).is_ok());
        let clash = [
            address.clone(),
            Snippet::new("Other", "Other").with_keyword(";ADDR"),
        ];
        assert!(Snippet::validate_unique_keywords(&clash).is_err());
        assert!(
            Snippet::new("Bad", "Text")
                .with_keyword("not a keyword")
                .validate()
                .is_err()
        );
        assert!(
            Snippet::new("Bad", "Text")
                .with_keyword("")
                .validate()
                .is_err()
        );
        assert!(
            Snippet::new("Bad", "Text")
                .with_keyword("é")
                .validate()
                .is_err()
        );
        assert!(
            Snippet::new("Bad", "Text")
                .with_keyword("k".repeat(65))
                .validate()
                .is_err()
        );
        assert!(
            Snippet::new("Ok", "Text")
                .with_keyword("k".repeat(64))
                .validate()
                .is_ok()
        );
        assert_eq!(SnippetExpansionPlan::erase_count(";addr"), 5); // the suppressed space is not backspaced

        let only_address = [address.clone()];
        let mut matcher = SnippetKeywordMatcher::new(12);
        for character in "x;addr".chars() {
            assert!(matcher.record(character, &only_address).is_none());
        }
        assert!(matcher.record(' ', &only_address).is_none()); // a trigger inside a word is ignored
        assert!(matcher.buffer().ends_with(' '));
        matcher.reset();
        for character in ";addr".chars() {
            assert!(matcher.record(character, &only_address).is_none());
        }
        assert_eq!(
            matcher.record(' ', &only_address).map(|s| s.id.as_str()),
            Some(address.id.as_str())
        );
        assert!(matcher.buffer().is_empty());

        let only_prompted = [prompted];
        for character in ";hello".chars() {
            assert!(matcher.record(character, &only_prompted).is_none());
        }
        // Missing defaults never trigger replacement.
        assert!(matcher.record(' ', &only_prompted).is_none());
        assert!(matcher.buffer().is_empty());

        for character in "abcdefghijklmnop".chars() {
            matcher.record(character, &[]);
        }
        assert_eq!(matcher.buffer().chars().count(), 12);
    }

    #[test]
    fn keywords_match_case_insensitively_and_the_longest_wins() {
        let short = Snippet::new("Short", "s").with_keyword(";a");
        let long = Snippet::new("Long", "l").with_keyword(";ab");
        let snippets = [short, long.clone()];
        let mut matcher = SnippetKeywordMatcher::default();
        for character in ";AB".chars() {
            matcher.record(character, &snippets);
        }
        assert_eq!(
            matcher.record(' ', &snippets).map(|s| s.id.as_str()),
            Some(long.id.as_str())
        );
    }

    #[test]
    fn legacy_snippet_without_keyword_still_decodes_and_the_keyword_round_trips() {
        let legacy = r#"{"id":"legacy","name":"Legacy","template":"Hello {name}"}"#;
        let snippet: Snippet = serde_json::from_str(legacy).unwrap();
        assert_eq!(snippet.id, "legacy");
        assert_eq!(snippet.keyword, None);
        assert_eq!(
            snippet.render(&values(&[("name", "friend")])).unwrap(),
            "Hello friend"
        );
        assert!(!serde_json::to_string(&snippet).unwrap().contains("keyword"));
        let keyed = snippet.with_keyword(";hi");
        let json = serde_json::to_string(&keyed).unwrap();
        assert!(json.contains(r#""keyword":";hi""#), "{json}");
        assert_eq!(serde_json::from_str::<Snippet>(&json).unwrap(), keyed);
    }

    #[test]
    fn saving_rejects_keyword_conflicts_without_changing_saved_items() {
        let mut snippets = Vec::new();
        save_snippet_with_keyword(&mut snippets, None, "Address", "42 Main St", Some(";addr"))
            .unwrap();
        assert!(
            save_snippet_with_keyword(&mut snippets, None, "Other", "Other", Some(";ADDR"))
                .is_err()
        );
        assert_eq!(
            snippets.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
            ["Address"]
        );
        // Editing keeps the keyword unless a new one is given; blank clears it.
        let id = snippets[0].id.clone();
        save_snippet(&mut snippets, Some(&id), "Address 2", "43 Main St").unwrap();
        assert_eq!(snippets[0].keyword.as_deref(), Some(";addr"));
        save_snippet_with_keyword(
            &mut snippets,
            Some(&id),
            "Address 2",
            "43 Main St",
            Some("  "),
        )
        .unwrap();
        assert_eq!(snippets[0].keyword, None);
    }
}
