//! Localization: `.strings` catalogs shared with the macOS app, a Linux overlay, language
//! selection from the environment and `%@` / `%d` / `%1$@` style formatting.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::sync::OnceLock;

use crate::error::ConfigError;

const EN_MAIN: &str =
    include_str!("../../../../Sources/Kebloop/Resources/en.lproj/Localizable.strings");
const PL_MAIN: &str =
    include_str!("../../../../Sources/Kebloop/Resources/pl.lproj/Localizable.strings");
const EN_LINUX: &str = include_str!("../../../resources/en.strings");
const PL_LINUX: &str = include_str!("../../../resources/pl.strings");

/// A parsed catalog.
pub type Catalog = BTreeMap<String, String>;

/// Parse failures with 1-based line numbers.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StringsError {
    /// Input ended inside an entry or string.
    #[error("unexpected end of .strings data")]
    UnexpectedEnd,
    /// A character that does not belong at this position.
    #[error("line {line}: unexpected character {found:?}")]
    Unexpected {
        /// Line of the problem.
        line: usize,
        /// Offending character.
        found: char,
    },
    /// A `/* … */` comment was not closed.
    #[error("line {0}: unterminated comment")]
    UnterminatedComment(usize),
    /// An invalid escape sequence.
    #[error("line {0}: invalid escape sequence")]
    BadEscape(usize),
}

struct Cursor<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
    line: usize,
}

impl<'a> Cursor<'a> {
    fn bump(&mut self) -> Option<char> {
        let c = self.chars.next();
        if c == Some('\n') {
            self.line += 1;
        }
        c
    }

    fn skip_trivia(&mut self) -> Result<(), StringsError> {
        loop {
            match self.chars.peek().copied() {
                Some(c) if c.is_whitespace() || c == '\u{feff}' => {
                    self.bump();
                }
                Some('/') => {
                    let start = self.line;
                    self.bump();
                    match self.chars.peek().copied() {
                        Some('/') => {
                            while let Some(c) = self.bump() {
                                if c == '\n' {
                                    break;
                                }
                            }
                        }
                        Some('*') => {
                            self.bump();
                            let mut previous = '\0';
                            loop {
                                match self.bump() {
                                    Some('/') if previous == '*' => break,
                                    Some(c) => previous = c,
                                    None => return Err(StringsError::UnterminatedComment(start)),
                                }
                            }
                        }
                        Some(found) => {
                            return Err(StringsError::Unexpected {
                                line: self.line,
                                found,
                            });
                        }
                        None => return Err(StringsError::UnexpectedEnd),
                    }
                }
                _ => return Ok(()),
            }
        }
    }

    fn string(&mut self) -> Result<String, StringsError> {
        match self.chars.peek().copied() {
            Some('"') => {
                self.bump();
                let mut out = String::new();
                loop {
                    match self.bump() {
                        None => return Err(StringsError::UnexpectedEnd),
                        Some('"') => return Ok(out),
                        Some('\\') => out.push(self.escape()?),
                        Some(c) => out.push(c),
                    }
                }
            }
            Some(c) if c.is_alphanumeric() || "_.-/$".contains(c) => {
                let mut out = String::new();
                while let Some(c) = self.chars.peek().copied() {
                    if c.is_alphanumeric() || "_.-/$".contains(c) {
                        out.push(c);
                        self.bump();
                    } else {
                        break;
                    }
                }
                Ok(out)
            }
            Some(found) => Err(StringsError::Unexpected {
                line: self.line,
                found,
            }),
            None => Err(StringsError::UnexpectedEnd),
        }
    }

    fn escape(&mut self) -> Result<char, StringsError> {
        let line = self.line;
        match self.bump().ok_or(StringsError::UnexpectedEnd)? {
            'n' => Ok('\n'),
            't' => Ok('\t'),
            'r' => Ok('\r'),
            '"' => Ok('"'),
            '\'' => Ok('\''),
            '\\' => Ok('\\'),
            '0' => Ok('\0'),
            'u' | 'U' => {
                let mut value = 0u32;
                for _ in 0..4 {
                    let digit = self
                        .bump()
                        .and_then(|c| c.to_digit(16))
                        .ok_or(StringsError::BadEscape(line))?;
                    value = value * 16 + digit;
                }
                char::from_u32(value).ok_or(StringsError::BadEscape(line))
            }
            _ => Err(StringsError::BadEscape(line)),
        }
    }

    fn expect(&mut self, wanted: char) -> Result<(), StringsError> {
        match self.bump() {
            Some(c) if c == wanted => Ok(()),
            Some(found) => Err(StringsError::Unexpected {
                line: self.line,
                found,
            }),
            None => Err(StringsError::UnexpectedEnd),
        }
    }
}

/// Parse `.strings` text (`"key" = "value";` entries, `/* */` and `//` comments, C escapes).
/// Later duplicates replace earlier ones.
pub fn parse_strings(text: &str) -> Result<Catalog, StringsError> {
    let mut cursor = Cursor {
        chars: text.chars().peekable(),
        line: 1,
    };
    let mut catalog = Catalog::new();
    loop {
        cursor.skip_trivia()?;
        if cursor.chars.peek().is_none() {
            return Ok(catalog);
        }
        let key = cursor.string()?;
        cursor.skip_trivia()?;
        if cursor.chars.peek() == Some(&';') {
            // `"key";` means the key is its own value.
            cursor.bump();
            catalog.insert(key.clone(), key);
            continue;
        }
        cursor.expect('=')?;
        cursor.skip_trivia()?;
        let value = cursor.string()?;
        cursor.skip_trivia()?;
        cursor.expect(';')?;
        catalog.insert(key, value);
    }
}

/// Supported UI languages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Language {
    /// English (also the fallback).
    #[default]
    English,
    /// Polish.
    Polish,
}

impl Language {
    /// Every language with a catalog.
    pub const ALL: [Language; 2] = [Language::English, Language::Polish];

    /// ISO code (`en`, `pl`).
    pub fn code(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::Polish => "pl",
        }
    }

    /// Recognise `pl`, `pl_PL.UTF-8`, `en-US`, … ; `None` for unsupported languages.
    pub fn from_tag(tag: &str) -> Option<Self> {
        let primary = tag.split(['_', '.', '@', '-']).next()?.to_ascii_lowercase();
        match primary.as_str() {
            "pl" => Some(Self::Polish),
            "en" => Some(Self::English),
            _ => None,
        }
    }
}

/// Choose a language from an injected environment: an explicit `C`/`POSIX` locale
/// (`LC_ALL`, `LC_MESSAGES`, `LANG`, first non-empty) means English; otherwise the first
/// supported entry of `LANGUAGE`, then the locale itself, then English.
pub fn select_language(env: &dyn Fn(&str) -> Option<String>) -> Language {
    let get = |key: &str| env(key).filter(|v| !v.is_empty());
    let locale = get("LC_ALL")
        .or_else(|| get("LC_MESSAGES"))
        .or_else(|| get("LANG"));
    if let Some(locale) = &locale {
        let base = locale.split(['.', '@']).next().unwrap_or("");
        if base == "C" || base == "POSIX" {
            return Language::English;
        }
    }
    if let Some(list) = get("LANGUAGE")
        && let Some(language) = list.split(':').find_map(Language::from_tag)
    {
        return language;
    }
    locale
        .as_deref()
        .and_then(Language::from_tag)
        .unwrap_or_default()
}

struct Embedded {
    en_main: Catalog,
    pl_main: Catalog,
    en_linux: Catalog,
    pl_linux: Catalog,
}

fn embedded() -> &'static Embedded {
    static CATALOGS: OnceLock<Embedded> = OnceLock::new();
    CATALOGS.get_or_init(|| Embedded {
        en_main: parse_strings(EN_MAIN).unwrap_or_default(),
        pl_main: parse_strings(PL_MAIN).unwrap_or_default(),
        en_linux: parse_strings(EN_LINUX).unwrap_or_default(),
        pl_linux: parse_strings(PL_LINUX).unwrap_or_default(),
    })
}

/// The shared macOS catalog of a language (without the Linux overlay).
pub fn shared_catalog(language: Language) -> &'static Catalog {
    match language {
        Language::English => &embedded().en_main,
        Language::Polish => &embedded().pl_main,
    }
}

/// The Linux-only overlay catalog of a language.
pub fn linux_catalog(language: Language) -> &'static Catalog {
    match language {
        Language::English => &embedded().en_linux,
        Language::Polish => &embedded().pl_linux,
    }
}

/// All keys known for a language (shared plus overlay).
pub fn catalog_keys(language: Language) -> BTreeSet<String> {
    shared_catalog(language)
        .keys()
        .chain(linux_catalog(language).keys())
        .cloned()
        .collect()
}

/// An argument for [`format_template`].
#[derive(Debug, Clone, PartialEq)]
pub enum FormatArg {
    /// Text (`%@`, `%s`).
    Str(String),
    /// Integer (`%d`, `%i`, `%u`).
    Int(i64),
    /// Floating point (`%f`).
    Float(f64),
}

impl From<&str> for FormatArg {
    fn from(v: &str) -> Self {
        Self::Str(v.to_string())
    }
}
impl From<String> for FormatArg {
    fn from(v: String) -> Self {
        Self::Str(v)
    }
}
impl From<i64> for FormatArg {
    fn from(v: i64) -> Self {
        Self::Int(v)
    }
}
impl From<i32> for FormatArg {
    fn from(v: i32) -> Self {
        Self::Int(i64::from(v))
    }
}
impl From<u32> for FormatArg {
    fn from(v: u32) -> Self {
        Self::Int(i64::from(v))
    }
}
impl From<usize> for FormatArg {
    fn from(v: usize) -> Self {
        Self::Int(i64::try_from(v).unwrap_or(i64::MAX))
    }
}
impl From<f64> for FormatArg {
    fn from(v: f64) -> Self {
        Self::Float(v)
    }
}

fn render(arg: &FormatArg, precision: Option<usize>) -> String {
    match arg {
        FormatArg::Str(s) => s.clone(),
        FormatArg::Int(i) => i.to_string(),
        FormatArg::Float(f) => format!("{f:.*}", precision.unwrap_or(6)),
    }
}

/// Substitute `%@ %s %d %i %u %f`, positional `%1$@`, width/length modifiers and `%%`.
/// Specifiers without a matching argument or with an unknown conversion are kept verbatim.
pub fn format_template(template: &str, args: &[FormatArg]) -> String {
    let chars: Vec<char> = template.chars().collect();
    let mut out = String::with_capacity(template.len());
    let mut next_arg = 0usize;
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '%' {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let start = i;
        i += 1;
        if chars.get(i) == Some(&'%') {
            out.push('%');
            i += 1;
            continue;
        }
        // Optional positional index `N$`.
        let mut position = None;
        let mut j = i;
        while chars.get(j).is_some_and(char::is_ascii_digit) {
            j += 1;
        }
        if j > i && chars.get(j) == Some(&'$') {
            position = chars[i..j].iter().collect::<String>().parse::<usize>().ok();
            i = j + 1;
        }
        // Flags, width, precision.
        while chars.get(i).is_some_and(|c| "-+ #0".contains(*c)) {
            i += 1;
        }
        while chars.get(i).is_some_and(char::is_ascii_digit) {
            i += 1;
        }
        let mut precision = None;
        if chars.get(i) == Some(&'.') {
            i += 1;
            let begin = i;
            while chars.get(i).is_some_and(char::is_ascii_digit) {
                i += 1;
            }
            precision = Some(
                chars[begin..i]
                    .iter()
                    .collect::<String>()
                    .parse::<usize>()
                    .unwrap_or(0),
            );
        }
        while chars.get(i).is_some_and(|c| "hlqzjt".contains(*c)) {
            i += 1;
        }
        let verbatim =
            |out: &mut String, end: usize| out.extend(&chars[start..end.min(chars.len())]);
        match chars.get(i) {
            Some('@' | 's' | 'd' | 'i' | 'u' | 'f') => {
                let index = match position {
                    Some(p) if p >= 1 => p - 1,
                    Some(_) => usize::MAX,
                    None => {
                        let n = next_arg;
                        next_arg += 1;
                        n
                    }
                };
                match args.get(index) {
                    Some(arg) => out.push_str(&render(arg, precision)),
                    None => verbatim(&mut out, i + 1),
                }
                i += 1;
            }
            _ => {
                verbatim(&mut out, i);
            }
        }
    }
    out
}

/// Looks up translations for one language, falling back to English and finally to the key.
#[derive(Debug, Clone, Default)]
pub struct Localizer {
    language: Language,
    overlay: Catalog,
}

impl Localizer {
    /// Localizer for a language using only the embedded catalogs.
    pub fn new(language: Language) -> Self {
        Self {
            language,
            overlay: Catalog::new(),
        }
    }

    /// English localizer.
    pub fn english() -> Self {
        Self::new(Language::English)
    }

    /// Localizer for the language selected by an injected environment lookup.
    pub fn from_env(env: &dyn Fn(&str) -> Option<String>) -> Self {
        Self::new(select_language(env))
    }

    /// Add `<dir>/<code>.strings` (e.g. user-provided translations) on top of the embedded
    /// catalogs. A missing file is not an error; an unreadable or malformed one is.
    pub fn with_overlay_dir(mut self, dir: &Path) -> Result<Self, ConfigError> {
        let path = dir.join(format!("{}.strings", self.language.code()));
        match fs::read_to_string(&path) {
            Ok(text) => {
                self.overlay = parse_strings(&text)
                    .map_err(|e| ConfigError::invalid(format!("{}: {e}", path.display())))?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(ConfigError::io(path, e)),
        }
        Ok(self)
    }

    /// Selected language.
    pub fn language(&self) -> Language {
        self.language
    }

    /// Translation of `key` (the English source text); unknown keys are returned unchanged.
    pub fn tr(&self, key: &str) -> String {
        let en = Language::English;
        self.overlay
            .get(key)
            .or_else(|| linux_catalog(self.language).get(key))
            .or_else(|| shared_catalog(self.language).get(key))
            .or_else(|| linux_catalog(en).get(key))
            .or_else(|| shared_catalog(en).get(key))
            .cloned()
            .unwrap_or_else(|| key.to_string())
    }

    /// Translate and format.
    pub fn format(&self, key: &str, args: &[FormatArg]) -> String {
        format_template(&self.tr(key), args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: BTreeMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |key| map.get(key).cloned()
    }

    #[test]
    fn parser_handles_comments_escapes_and_unicode() {
        let text = "/* header */\n// line\n\"a\" = \"x\\ny\\\"z\\\\\";\n\"b\" = \"\\U0105\\u00e9\";\n\"c\";\n";
        let catalog = parse_strings(text).unwrap();
        assert_eq!(catalog["a"], "x\ny\"z\\");
        assert_eq!(catalog["b"], "ąé");
        assert_eq!(catalog["c"], "c");
    }

    #[test]
    fn parser_reports_errors() {
        assert!(matches!(
            parse_strings("\"a\" = \"b\""),
            Err(StringsError::UnexpectedEnd)
        ));
        assert!(matches!(
            parse_strings("/* open"),
            Err(StringsError::UnterminatedComment(1))
        ));
        assert!(matches!(
            parse_strings("\"a\" = \"\\q\";"),
            Err(StringsError::BadEscape(1))
        ));
        assert!(matches!(
            parse_strings("\"a\" \"b\";"),
            Err(StringsError::Unexpected { .. })
        ));
    }

    #[test]
    fn english_and_polish_catalogs_have_matching_keys_and_real_translations() {
        let english = shared_catalog(Language::English);
        let polish = shared_catalog(Language::Polish);
        assert_eq!(
            english.keys().collect::<Vec<_>>(),
            polish.keys().collect::<Vec<_>>()
        );
        assert_eq!(polish["Open Settings"], "Otwórz ustawienia");
        assert_eq!(polish["Clipboard History"], "Historia schowka");
        assert!(english.len() > 290);
        for (key, value) in polish {
            assert!(!value.is_empty(), "Empty translation for {key}");
        }
    }

    #[test]
    fn packaged_polish_lookup() {
        let polish = Localizer::new(Language::Polish);
        for (key, translated) in [
            ("Open Settings", "Otwórz ustawienia"),
            ("Clipboard History", "Historia schowka"),
            ("Remove Item", "Usuń element"),
        ] {
            assert_eq!(polish.tr(key), translated);
        }
        assert_eq!(Localizer::english().tr("Open Settings"), "Open Settings");
        assert_eq!(polish.tr("not in any catalog"), "not in any catalog");
    }

    #[test]
    fn language_selection_follows_environment() {
        assert_eq!(select_language(&env_of(&[])), Language::English);
        assert_eq!(
            select_language(&env_of(&[("LANG", "pl_PL.UTF-8")])),
            Language::Polish
        );
        assert_eq!(
            select_language(&env_of(&[("LANG", "de_DE.UTF-8")])),
            Language::English
        );
        assert_eq!(
            select_language(&env_of(&[
                ("LANG", "en_US.UTF-8"),
                ("LC_MESSAGES", "pl_PL")
            ])),
            Language::Polish
        );
        assert_eq!(
            select_language(&env_of(&[("LANG", "en_US"), ("LC_ALL", "pl_PL")])),
            Language::Polish
        );
        assert_eq!(
            select_language(&env_of(&[("LANGUAGE", "de:pl:en"), ("LANG", "en_US")])),
            Language::Polish
        );
        assert_eq!(
            select_language(&env_of(&[("LANGUAGE", "pl"), ("LC_ALL", "C")])),
            Language::English
        );
        assert_eq!(
            select_language(&env_of(&[("LANGUAGE", "pl")])),
            Language::Polish
        );
        assert_eq!(
            select_language(&env_of(&[("LANG", "pl"), ("LC_ALL", "")])),
            Language::Polish
        );
        assert_eq!(
            select_language(&env_of(&[("LANG", "C.UTF-8")])),
            Language::English
        );
    }

    #[test]
    fn format_placeholders() {
        let args = [FormatArg::from("Łódź"), FormatArg::from(3)];
        assert_eq!(
            format_template("%@ has %d items (100%%)", &args),
            "Łódź has 3 items (100%)"
        );
        assert_eq!(format_template("%2$d of %1$@", &args), "3 of Łódź");
        assert_eq!(format_template("%lld/%ld", &[1.into(), 2.into()]), "1/2");
        assert_eq!(format_template("%.1f", &[2.25_f64.into()]), "2.2");
        assert_eq!(format_template("%@ and %@", &["a".into()]), "a and %@");
        assert_eq!(format_template("100% done", &[]), "100% done");
        assert_eq!(format_template("%", &[]), "%");
    }

    #[test]
    fn overlay_directory_overrides_embedded_strings() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("pl.strings"), "\"Help\" = \"Pomoc!\";\n").unwrap();
        let localizer = Localizer::new(Language::Polish)
            .with_overlay_dir(dir.path())
            .unwrap();
        assert_eq!(localizer.tr("Help"), "Pomoc!");
        assert_eq!(localizer.tr("Open Settings"), "Otwórz ustawienia");
        let english = Localizer::english().with_overlay_dir(dir.path()).unwrap();
        assert_eq!(english.tr("Help"), "Help");
        fs::write(dir.path().join("en.strings"), "broken").unwrap();
        assert!(Localizer::english().with_overlay_dir(dir.path()).is_err());
    }
}
