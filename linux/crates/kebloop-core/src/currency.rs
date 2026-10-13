//! Currency conversions such as `100 USD to EUR` or `$50 in PLN` from saved exchange rates.
//!
//! The engine never touches the network: it converts with [`CurrencyRateSnapshot`]s that
//! the caller (see `kebloop_services::currency`) downloaded and cached. Port of
//! `Sources/KebloopCore/CurrencyConversionEngine.swift`.

use std::collections::HashMap;
use std::sync::OnceLock;

use jiff::Timestamp;
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::calculator::SpecializedCalculation;
use crate::locale::Locale;
use crate::units::{UnitConversionEngine, find_separators};

/// Longest input considered by [`pair_in`].
const MAXIMUM_INPUT_CHARACTERS: usize = 512;

/// Cache limits shared by the engine (staleness) and the rate store (retention).
#[derive(Debug, Clone, Copy)]
pub struct CurrencyRateCachePolicy;

impl CurrencyRateCachePolicy {
    /// A rate downloaded longer ago than this (24 hours) is refreshed and marked `cached`.
    pub const FRESHNESS_SECONDS: i64 = 24 * 60 * 60;
    /// Saved rates older than this (30 days) are dropped.
    pub const MAXIMUM_AGE_SECONDS: i64 = 30 * 24 * 60 * 60;
    /// At most this many pairs are kept.
    pub const MAXIMUM_ENTRIES: usize = 64;
}

/// An ordered pair of supported ISO 4217 codes.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CurrencyPair {
    base: String,
    quote: String,
}

impl CurrencyPair {
    /// Creates a pair from two case-insensitive codes; `None` unless both are supported.
    pub fn new(base: &str, quote: &str) -> Option<Self> {
        let base = base.to_uppercase();
        let quote = quote.to_uppercase();
        (is_supported(&base) && is_supported(&quote)).then_some(Self { base, quote })
    }

    /// The currency being converted from (upper case).
    pub fn base(&self) -> &str {
        &self.base
    }

    /// The currency being converted to (upper case).
    pub fn quote(&self) -> &str {
        &self.quote
    }
}

/// One saved exchange rate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrencyRateSnapshot {
    /// The pair the rate applies to.
    pub pair: CurrencyPair,
    /// Number of quote currency units for one base currency unit.
    pub rate: f64,
    /// The observation date returned by the rate provider (midnight UTC).
    #[serde(with = "unix_seconds")]
    pub rate_date: Timestamp,
    /// When this device last downloaded the observation.
    #[serde(with = "unix_seconds")]
    pub fetched_at: Timestamp,
}

impl CurrencyRateSnapshot {
    /// Creates a snapshot.
    pub fn new(pair: CurrencyPair, rate: f64, rate_date: Timestamp, fetched_at: Timestamp) -> Self {
        Self {
            pair,
            rate,
            rate_date,
            fetched_at,
        }
    }

    /// Whether the rate was downloaded more than 24 hours before `at`.
    pub fn is_stale(&self, at: Timestamp) -> bool {
        at.as_second() - self.fetched_at.as_second() > CurrencyRateCachePolicy::FRESHNESS_SECONDS
    }
}

mod unix_seconds {
    use jiff::Timestamp;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(value: &Timestamp, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_i64(value.as_second())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Timestamp, D::Error> {
        let seconds = i64::deserialize(deserializer)?;
        Timestamp::from_second(seconds).map_err(serde::de::Error::custom)
    }
}

/// The currency pair of a conversion query (`100 USD in euros`), if it is one. Lets the UI
/// refresh rates in the background without doing network work in the typing path.
pub fn pair_in(input: &str) -> Option<CurrencyPair> {
    if input.chars().count() > MAXIMUM_INPUT_CHARACTERS {
        return None;
    }
    let (source_text, target_text) = conversion_parts(input)?;
    let (_, source) = parse_amount_and_currency(source_text)?;
    let target = currency_code(target_text)?;
    CurrencyPair::new(&source, &target)
}

/// Converts `<amount> <currency> to|in|as <currency>` using saved rates.
#[derive(Debug, Clone, Default)]
pub(crate) struct CurrencyConversionEngine {
    locale: Locale,
}

impl CurrencyConversionEngine {
    pub(crate) fn new(locale: Locale) -> Self {
        Self { locale }
    }

    pub(crate) fn evaluate(
        &self,
        input: &str,
        rates: &[CurrencyRateSnapshot],
        now: Timestamp,
    ) -> SpecializedCalculation {
        let Some((source_text, target_text)) = conversion_parts(input) else {
            return SpecializedCalculation::NotHandled;
        };
        let Some((amount, source_code)) = parse_amount_and_currency(source_text) else {
            return SpecializedCalculation::NotHandled;
        };
        if target_text.is_empty() {
            return SpecializedCalculation::Incomplete;
        }
        let Some(target_code) = currency_code(target_text) else {
            if UnitConversionEngine::new(self.locale.clone()).recognizes_unit(target_text) {
                return SpecializedCalculation::NotHandled;
            }
            return SpecializedCalculation::Failure(format!("Unknown currency “{target_text}”."));
        };
        let Some(pair) = CurrencyPair::new(&source_code, &target_code) else {
            return SpecializedCalculation::NotHandled;
        };

        if pair.base == pair.quote {
            return SpecializedCalculation::Result(format!(
                "{} {}",
                self.format_amount(amount),
                pair.quote
            ));
        }

        let inverse = CurrencyPair {
            base: pair.quote.clone(),
            quote: pair.base.clone(),
        };
        // Newest download first; on equal timestamps the earlier entry wins (stable sort).
        let mut candidates: Vec<&CurrencyRateSnapshot> = rates
            .iter()
            .filter(|snapshot| snapshot.rate.is_finite() && snapshot.rate > 0.0)
            .collect();
        candidates.sort_by_key(|snapshot| std::cmp::Reverse(snapshot.fetched_at));
        let Some(snapshot) = candidates
            .into_iter()
            .find(|snapshot| snapshot.pair == pair || snapshot.pair == inverse)
        else {
            return SpecializedCalculation::Failure(
                "No saved exchange rate is available. Connect to the internet and try again."
                    .into(),
            );
        };

        let rate = if snapshot.pair == pair {
            snapshot.rate
        } else {
            1.0 / snapshot.rate
        };
        let converted = amount * rate;
        if !converted.is_finite() {
            return SpecializedCalculation::Failure(
                "The converted value is outside the supported range.".into(),
            );
        }

        let cached_suffix = if snapshot.is_stale(now) {
            ", cached"
        } else {
            ""
        };
        let rate_date = snapshot.rate_date.to_zoned(jiff::tz::TimeZone::UTC);
        let date = self.locale.format_date(
            i32::from(rate_date.year()),
            u8::try_from(rate_date.month()).unwrap_or(1),
            u8::try_from(rate_date.day()).unwrap_or(1),
            None,
        );
        let amount_text = format!("{} {}", self.format_amount(converted), pair.quote);
        SpecializedCalculation::ResultWithClipboard {
            display: format!("{amount_text} (rate dated {date}{cached_suffix})"),
            clipboard: amount_text,
        }
    }

    /// Two fraction digits, no grouping, the locale's decimal mark.
    fn format_amount(&self, value: f64) -> String {
        format_fixed(value, 2, self.locale.decimal_separator())
    }
}

/// Formats with exactly `fraction_digits` digits after the decimal mark, rounding half-even on
/// the shortest decimal representation of `value` (what `NumberFormatter` does) and never
/// using grouping or an exponent.
pub(crate) fn format_fixed(value: f64, fraction_digits: usize, decimal_separator: char) -> String {
    if !value.is_finite() {
        return "0".into();
    }
    let scientific = format!("{:e}", value.abs());
    let (mantissa, exponent) = scientific.split_once('e').unwrap_or((&scientific, "0"));
    let exponent: i64 = exponent.parse().unwrap_or(0);
    let mut digits: Vec<u8> = mantissa
        .bytes()
        .filter(u8::is_ascii_digit)
        .map(|b| b - b'0')
        .collect();
    if value == 0.0 {
        digits = vec![0];
    }
    // `digits` holds d0.d1d2... * 10^exponent; lay them out around the decimal point.
    let integer_len = exponent + 1;
    let mut integer: Vec<u8> = Vec::new();
    let mut fraction: Vec<u8> = Vec::new();
    if integer_len <= 0 {
        integer.push(0);
        let zeros = usize::try_from(-integer_len).unwrap_or(0);
        fraction.extend(std::iter::repeat_n(0, zeros));
        fraction.extend(&digits);
    } else {
        let integer_len = usize::try_from(integer_len).unwrap_or(0);
        for index in 0..integer_len {
            integer.push(digits.get(index).copied().unwrap_or(0));
        }
        if digits.len() > integer_len {
            fraction.extend(&digits[integer_len..]);
        }
    }

    if fraction.len() > fraction_digits {
        let first_dropped = fraction[fraction_digits];
        let rest_nonzero = fraction[fraction_digits + 1..].iter().any(|&d| d != 0);
        let last_kept = if fraction_digits == 0 {
            integer.last().copied().unwrap_or(0)
        } else {
            fraction[fraction_digits - 1]
        };
        let round_up =
            first_dropped > 5 || (first_dropped == 5 && (rest_nonzero || last_kept % 2 == 1));
        fraction.truncate(fraction_digits);
        if round_up {
            let mut carry = true;
            for digit in fraction.iter_mut().rev().chain(integer.iter_mut().rev()) {
                if *digit == 9 {
                    *digit = 0;
                } else {
                    *digit += 1;
                    carry = false;
                    break;
                }
            }
            if carry {
                integer.insert(0, 1);
            }
        }
    }
    fraction.resize(fraction_digits, 0);

    let mut output = String::new();
    if value.is_sign_negative() {
        output.push('-');
    }
    output.extend(integer.iter().map(|&d| char::from(b'0' + d)));
    if fraction_digits > 0 {
        output.push(decimal_separator);
        output.extend(fraction.iter().map(|&d| char::from(b'0' + d)));
    }
    output
}

/// `(?i)\s+(to|in|as)(?=\s|$)`: the source (before the last separator) and the target text,
/// both trimmed. `None` when there is no separator or the source is empty.
fn conversion_parts(input: &str) -> Option<(&str, &str)> {
    let &(start, end) = find_separators(input).last()?;
    let source = input[..start].trim();
    let target = input[end..].trim();
    (!source.is_empty()).then_some((source, target))
}

/// The ICU number syntax used for amounts.
const NUMBER: &str = r"[+-]?(?:[0-9]+(?:[.,][0-9]*)?|[.,][0-9]+)(?:[eE][+-]?[0-9]+)?";
/// `.` in the original patterns does not match line terminators.
const ANY: &str = r"[^\n\r\x0B\x0C\x{85}\x{2028}\x{2029}]";

fn amount_first() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    #[allow(clippy::expect_used)] // The pattern is a constant that is validated by tests.
    PATTERN.get_or_init(|| {
        Regex::new(&format!(r"^\s*({NUMBER})\s*({ANY}*?)\s*$")).expect("valid amount pattern")
    })
}

fn currency_first() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    #[allow(clippy::expect_used)] // The pattern is a constant that is validated by tests.
    PATTERN.get_or_init(|| {
        Regex::new(&format!(r"^\s*({ANY}*?)\s*({NUMBER})\s*$")).expect("valid currency pattern")
    })
}

/// `100 USD`, `$50` or `50 dollars` / `USD 100`: the amount and the currency code.
fn parse_amount_and_currency(text: &str) -> Option<(f64, String)> {
    if let Some(captures) = amount_first().captures(text)
        && let Some(code) = currency_code(&captures[2])
        && let Some(amount) = parse_amount(&captures[1])
    {
        return Some((amount, code));
    }
    if let Some(captures) = currency_first().captures(text)
        && let Some(code) = currency_code(&captures[1])
        && let Some(amount) = parse_amount(&captures[2])
    {
        return Some((amount, code));
    }
    None
}

fn parse_amount(literal: &str) -> Option<f64> {
    literal
        .replace(',', ".")
        .parse::<f64>()
        .ok()
        .filter(|amount| amount.is_finite())
}

fn currency_code(raw_name: &str) -> Option<String> {
    let lowered = raw_name.trim().to_lowercase().replace('.', "");
    let normalized = lowered.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() == 3 {
        let upper = normalized.to_uppercase();
        if is_supported(&upper) {
            return Some(upper);
        }
    }
    aliases()
        .get(normalized.as_str())
        .map(|&code| code.to_owned())
}

const SUPPORTED_CODES: [&str; 37] = [
    "AED", "AUD", "BGN", "BRL", "CAD", "CHF", "CNY", "CZK", "DKK", "EUR", "GBP", "HKD", "HUF",
    "IDR", "ILS", "INR", "ISK", "JPY", "KRW", "MXN", "MYR", "NOK", "NZD", "PHP", "PLN", "RON",
    "RUB", "SAR", "SEK", "SGD", "THB", "TRY", "TWD", "UAH", "USD", "VND", "ZAR",
];

fn is_supported(code: &str) -> bool {
    SUPPORTED_CODES.contains(&code)
}

fn aliases() -> &'static HashMap<&'static str, &'static str> {
    static ALIASES: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    ALIASES.get_or_init(|| ALIASES_TABLE.iter().copied().collect())
}

#[rustfmt::skip]
const ALIASES_TABLE: &[(&str, &str)] = &[
    ("$", "USD"), ("us$", "USD"), ("usd", "USD"), ("dollar", "USD"), ("dollars", "USD"), ("us dollar", "USD"), ("us dollars", "USD"), ("🇺🇸", "USD"),
    ("€", "EUR"), ("eur", "EUR"), ("euro", "EUR"), ("euros", "EUR"),
    ("£", "GBP"), ("gbp", "GBP"), ("pound", "GBP"), ("pounds", "GBP"), ("british pound", "GBP"), ("sterling", "GBP"),
    ("¥", "JPY"), ("jp¥", "JPY"), ("jpy", "JPY"), ("yen", "JPY"), ("yens", "JPY"),
    ("cn¥", "CNY"), ("cny", "CNY"), ("yuan", "CNY"), ("renminbi", "CNY"), ("rmb", "CNY"),
    ("ca$", "CAD"), ("cad", "CAD"), ("canadian dollar", "CAD"), ("canadian dollars", "CAD"),
    ("a$", "AUD"), ("aud", "AUD"), ("australian dollar", "AUD"), ("australian dollars", "AUD"),
    ("nzd", "NZD"), ("new zealand dollar", "NZD"), ("new zealand dollars", "NZD"),
    ("chf", "CHF"), ("swiss franc", "CHF"), ("swiss francs", "CHF"),
    ("pln", "PLN"), ("zł", "PLN"), ("zloty", "PLN"), ("złoty", "PLN"), ("zlotys", "PLN"), ("złote", "PLN"),
    ("sek", "SEK"), ("swedish krona", "SEK"), ("swedish kronor", "SEK"),
    ("nok", "NOK"), ("norwegian krone", "NOK"), ("norwegian kroner", "NOK"),
    ("dkk", "DKK"), ("danish krone", "DKK"), ("danish kroner", "DKK"),
    ("czk", "CZK"), ("czech koruna", "CZK"), ("czech korunas", "CZK"),
    ("huf", "HUF"), ("forint", "HUF"), ("forints", "HUF"),
    ("ron", "RON"), ("romanian leu", "RON"), ("lei", "RON"),
    ("bgn", "BGN"), ("bulgarian lev", "BGN"), ("leva", "BGN"),
    ("inr", "INR"), ("₹", "INR"), ("rupee", "INR"), ("rupees", "INR"), ("indian rupee", "INR"), ("indian rupees", "INR"),
    ("sgd", "SGD"), ("s$", "SGD"), ("singapore dollar", "SGD"), ("singapore dollars", "SGD"),
    ("hkd", "HKD"), ("hk$", "HKD"), ("hong kong dollar", "HKD"), ("hong kong dollars", "HKD"),
    ("krw", "KRW"), ("₩", "KRW"), ("south korean won", "KRW"), ("won", "KRW"),
    ("mxn", "MXN"), ("mexican peso", "MXN"), ("mexican pesos", "MXN"),
    ("brl", "BRL"), ("r$", "BRL"), ("brazilian real", "BRL"), ("reais", "BRL"),
    ("zar", "ZAR"), ("south african rand", "ZAR"), ("rand", "ZAR"), ("rands", "ZAR"),
    ("rub", "RUB"), ("₽", "RUB"), ("russian ruble", "RUB"), ("ruble", "RUB"), ("rubles", "RUB"),
    ("thb", "THB"), ("฿", "THB"), ("thai baht", "THB"), ("baht", "THB"),
    ("idr", "IDR"), ("indonesian rupiah", "IDR"), ("rupiah", "IDR"),
    ("myr", "MYR"), ("rm", "MYR"), ("malaysian ringgit", "MYR"), ("ringgit", "MYR"),
    ("php", "PHP"), ("₱", "PHP"), ("philippine peso", "PHP"), ("philippine pesos", "PHP"),
    ("ils", "ILS"), ("₪", "ILS"), ("israeli new shekel", "ILS"), ("shekel", "ILS"), ("shekels", "ILS"),
    ("aed", "AED"), ("د.إ", "AED"), ("uae dirham", "AED"), ("dirham", "AED"), ("dirhams", "AED"),
    ("sar", "SAR"), ("﷼", "SAR"), ("saudi riyal", "SAR"), ("riyal", "SAR"), ("riyals", "SAR"),
    ("twd", "TWD"), ("nt$", "TWD"), ("taiwan dollar", "TWD"), ("taiwan dollars", "TWD"),
    ("isk", "ISK"), ("icelandic krona", "ISK"), ("icelandic kronur", "ISK"),
    ("try", "TRY"), ("₺", "TRY"), ("turkish lira", "TRY"), ("lira", "TRY"),
    ("uah", "UAH"), ("₴", "UAH"), ("ukrainian hryvnia", "UAH"), ("hryvnia", "UAH"),
    ("vnd", "VND"), ("₫", "VND"), ("vietnamese dong", "VND"), ("dong", "VND"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_formatting_rounds_half_even_on_the_shortest_representation() {
        assert_eq!(format_fixed(92.0, 2, '.'), "92.00");
        assert_eq!(format_fixed(0.005, 2, '.'), "0.00");
        assert_eq!(format_fixed(0.015, 2, '.'), "0.02");
        assert_eq!(format_fixed(0.025, 2, '.'), "0.02");
        assert_eq!(format_fixed(1.005, 2, ','), "1,00");
        assert_eq!(format_fixed(9.999, 2, '.'), "10.00");
        assert_eq!(format_fixed(99.995, 2, '.'), "100.00");
        assert_eq!(format_fixed(-4.6, 2, '.'), "-4.60");
        assert_eq!(format_fixed(0.0, 2, '.'), "0.00");
        assert_eq!(format_fixed(1234567.891, 2, '.'), "1234567.89");
        assert_eq!(format_fixed(1e21, 2, '.'), "1000000000000000000000.00");
        assert_eq!(format_fixed(0.00001, 2, '.'), "0.00");
    }

    #[test]
    fn every_alias_points_at_a_supported_code() {
        for (name, code) in ALIASES_TABLE {
            assert!(is_supported(code), "{name} -> {code}");
        }
        assert_eq!(aliases().len(), ALIASES_TABLE.len(), "duplicate alias");
    }

    #[test]
    fn amounts_and_currencies_parse_in_either_order() {
        assert_eq!(
            parse_amount_and_currency("$100"),
            Some((100.0, "USD".into()))
        );
        assert_eq!(
            parse_amount_and_currency("USD 100"),
            Some((100.0, "USD".into()))
        );
        assert_eq!(
            parse_amount_and_currency("1,5 euros"),
            Some((1.5, "EUR".into()))
        );
        assert_eq!(
            parse_amount_and_currency("100 yen"),
            Some((100.0, "JPY".into()))
        );
        assert_eq!(
            parse_amount_and_currency("100 zł"),
            Some((100.0, "PLN".into()))
        );
        assert_eq!(parse_amount_and_currency("100 km"), None);
        assert_eq!(parse_amount_and_currency("USD"), None);
    }

    #[test]
    fn pairs_come_from_conversion_queries_only() {
        let usd_eur = CurrencyPair::new("USD", "EUR");
        assert_eq!(pair_in("100 USD in euros"), usd_eur);
        assert_eq!(pair_in("$5 to EUR"), usd_eur);
        assert_eq!(pair_in("search for USD"), None);
        assert_eq!(pair_in("100 USD to"), None);
        assert_eq!(pair_in(&"x".repeat(513)), None);
        assert_eq!(CurrencyPair::new("usd", "xxx"), None);
    }
}
