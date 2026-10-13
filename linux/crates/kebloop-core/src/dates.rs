//! Date differences and date arithmetic such as `days between 2026-10-12 and 2026-10-20`,
//! `today + 10 days`, `2026-01-31 + 1 month`, `now`, `in 2h` or `3 days from now`.
//!
//! "Now" and the time zone are injected, so results are deterministic. Port of
//! `DateCalculationEngine.swift`; the patterns are the original regular expressions.

use std::sync::OnceLock;

use jiff::civil::{Date, DateTime};
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Span, Timestamp, Zoned};
use regex::Regex;

use crate::calculator::SpecializedCalculation;
use crate::locale::{DateOrder, Locale};

/// Inputs longer than this are not date expressions (the calculator caps input at 512 characters).
const MAXIMUM_INPUT_BYTES: usize = 2048;

/// Bound for elapsed-time arithmetic: a wide interval around the supported Gregorian range.
const MAXIMUM_ELAPSED_SECONDS: f64 = 315_537_897_599.0;

/// `.` in the original patterns does not match line terminators.
const ANY: &str = r"[^\n\r\x0B\x0C\x{85}\x{2028}\x{2029}]";
const UNIT: &str = r"(seconds?|secs?|s|minutes?|mins?|m|hours?|hrs?|h|days?|d|weeks?|wks?|w|wk|months?|mos?|mths?|mo|mth|years?|yrs?|yr|y)";
const AMOUNT: &str = r"[0-9]+(?:[.,][0-9]+)?";

const OUT_OF_RANGE: &str = "That date is outside the supported range.";
const WHOLE_NUMBER: &str = "Date additions need a whole number of units.";

struct Patterns {
    difference_between: Regex,
    days_between: Regex,
    difference_intent: Regex,
    to_split: Regex,
    and_split: Regex,
    now_offset: Regex,
    from_now: Regex,
    in_offset: Regex,
    now_operator: Regex,
    addition: Regex,
    trailing_operation: Regex,
    operand: Regex,
    iso_local: Regex,
    seconds: Regex,
}

fn patterns() -> Option<&'static Patterns> {
    static PATTERNS: OnceLock<Option<Patterns>> = OnceLock::new();
    PATTERNS
        .get_or_init(|| {
            let compile = |pattern: String| Regex::new(&pattern).ok();
            Some(Patterns {
                difference_between: compile(r"(?i)^date\s+difference\s+between\s+".into())?,
                days_between: compile(r"(?i)^days?\s+between\s+".into())?,
                difference_intent: compile(
                    r"(?i)^(?:date\s+difference\s+between|days?\s+between)\b".into(),
                )?,
                to_split: compile(format!(r"(?i)^({ANY}+?)\s+to\s+({ANY}+?)\s*$"))?,
                and_split: compile(format!(r"(?i)^({ANY}+?)\s+and\s+({ANY}+?)\s*$"))?,
                now_offset: compile(format!(
                    r"(?i)^(now|today)\s*([+-])\s*({AMOUNT})\s*{UNIT}\s*$"
                ))?,
                from_now: compile(format!(
                    r"(?i)^([+-]?{AMOUNT})\s*{UNIT}\s+from\s+(now|today)\s*$"
                ))?,
                in_offset: compile(format!(r"(?i)^in\s+([+-]?{AMOUNT})\s*{UNIT}\s*$"))?,
                now_operator: compile(r"(?i)^(?:now|today)\s*[+-]\s*$".into())?,
                addition: compile(format!(
                    r"(?i)^({ANY}+?)\s+([+-])\s*({AMOUNT})\s*{UNIT}\s*$"
                ))?,
                trailing_operation: compile(r"(?i)(?:\s+to\s*|\s+[+-]\s*)$".into())?,
                operand: compile(format!(r"(?i)^({ANY}+?)\s+(?:to\s*|[+-]\s*)$"))?,
                iso_local: compile(
                    r"^([0-9]{4})-([0-9]{2})-([0-9]{2})(?:[ T]([0-9]{2}):([0-9]{2})(?::([0-9]{2}))?)?$"
                        .into(),
                )?,
                seconds: compile(r"[0-9]{2}:[0-9]{2}:[0-9]{2}".into())?,
            })
        })
        .as_ref()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DateUnit {
    Second,
    Minute,
    Hour,
    Day,
    Week,
    Month,
    Year,
}

impl DateUnit {
    fn parse(name: &str) -> Option<Self> {
        Some(match name.to_lowercase().as_str() {
            "second" | "seconds" | "sec" | "secs" | "s" => Self::Second,
            "minute" | "minutes" | "min" | "mins" | "m" => Self::Minute,
            "hour" | "hours" | "hr" | "hrs" | "h" => Self::Hour,
            "day" | "days" | "d" => Self::Day,
            "week" | "weeks" | "w" | "wk" | "wks" => Self::Week,
            "month" | "months" | "mo" | "mos" | "mth" | "mths" => Self::Month,
            "year" | "years" | "y" | "yr" | "yrs" => Self::Year,
            _ => return None,
        })
    }

    fn is_duration(self) -> bool {
        matches!(self, Self::Second | Self::Minute | Self::Hour)
    }

    fn seconds(self) -> i64 {
        match self {
            Self::Second => 1,
            Self::Minute => 60,
            Self::Hour => 3_600,
            _ => 0,
        }
    }

    /// Largest accepted calendar offset (in the unit) before the result is certainly unsupported.
    fn maximum_offset(self) -> i64 {
        match self {
            Self::Day => 3_660_000,
            Self::Week => 522_000,
            Self::Month => 119_988,
            _ => 9_999,
        }
    }
}

struct ParsedDate {
    zoned: Zoned,
    includes_time: bool,
    includes_seconds: bool,
}

impl ParsedDate {
    fn date(zoned: Zoned) -> Self {
        Self {
            zoned,
            includes_time: false,
            includes_seconds: false,
        }
    }
}

/// Evaluates date expressions in a locale and time zone.
#[derive(Debug, Clone)]
pub struct DateCalculationEngine {
    locale: Locale,
    time_zone: TimeZone,
}

impl DateCalculationEngine {
    /// Creates an engine for `locale` (numeric date order, output format) and `time_zone`.
    pub fn new(locale: Locale, time_zone: TimeZone) -> Self {
        Self { locale, time_zone }
    }

    /// Evaluates `input` relative to `now`. Returns
    /// [`SpecializedCalculation::NotHandled`] for text that is not a date expression.
    pub fn evaluate(&self, input: &str, now: Timestamp) -> SpecializedCalculation {
        let text = input.trim();
        if text.len() > MAXIMUM_INPUT_BYTES {
            return SpecializedCalculation::NotHandled;
        }
        let Some(p) = patterns() else {
            return SpecializedCalculation::NotHandled;
        };

        let lowered = text.to_lowercase();
        if (lowered == "now" || lowered == "today")
            && let Some(parsed) = self.parse_date(text, now)
        {
            return SpecializedCalculation::Result(self.format(
                &parsed.zoned,
                parsed.includes_time,
                parsed.includes_seconds,
            ));
        }

        if let Some(found) = p
            .difference_between
            .find(text)
            .or_else(|| p.days_between.find(text))
        {
            return self.evaluate_explicit_difference(p, &text[found.end()..], now);
        }

        if let Some(captures) = p.to_split.captures(text) {
            let (left, right) = (captures[1].trim(), captures[2].trim());
            if is_date_candidate(left) {
                return self.difference(left, right, now);
            }
        }

        if p.difference_intent.is_match(text) {
            return SpecializedCalculation::Incomplete;
        }

        if let Some(outcome) = self.evaluate_relative_offset(p, text, now) {
            return outcome;
        }

        if let Some(captures) = p.addition.captures(text) {
            let date_text = captures[1].trim();
            if !is_date_candidate(date_text) {
                return SpecializedCalculation::NotHandled;
            }
            let Some(parsed) = self.parse_date(date_text, now) else {
                if is_beyond_timestamp_range(p, date_text) {
                    return out_of_range();
                }
                return SpecializedCalculation::Failure(
                    "Use a valid date, such as 2026-10-12.".into(),
                );
            };
            let literal = &captures[3];
            let Some(amount) = parse_whole_amount(literal) else {
                return whole_amount_failure(literal);
            };
            let negative = &captures[2] == "-";
            let Some(signed) = signed_amount(amount, negative) else {
                return out_of_range();
            };
            let Some(unit) = DateUnit::parse(&captures[4]) else {
                return SpecializedCalculation::NotHandled;
            };
            return self.add(
                signed,
                unit,
                &parsed.zoned,
                parsed.includes_time,
                parsed.includes_seconds,
            );
        }

        if p.trailing_operation.is_match(text)
            && let Some(captures) = p.operand.captures(text)
            && is_date_candidate(captures[1].trim())
        {
            return SpecializedCalculation::Incomplete;
        }
        SpecializedCalculation::NotHandled
    }

    /// `now - 3d`, `3 days from now`, `in 2h`, plus the unfinished `now +`.
    fn evaluate_relative_offset(
        &self,
        p: &Patterns,
        text: &str,
        now: Timestamp,
    ) -> Option<SpecializedCalculation> {
        if let Some(captures) = p.now_offset.captures(text) {
            let literal = &captures[3];
            let Some(amount) = parse_whole_amount(literal) else {
                return Some(whole_amount_failure(literal));
            };
            let unit = DateUnit::parse(&captures[4])?;
            let Some(parsed) = self.parse_date(&captures[1], now) else {
                return Some(out_of_range());
            };
            let Some(signed) = signed_amount(amount, &captures[2] == "-") else {
                return Some(out_of_range());
            };
            return Some(self.add(
                signed,
                unit,
                &parsed.zoned,
                parsed.includes_time,
                parsed.includes_seconds,
            ));
        }

        if let Some(captures) = p.from_now.captures(text) {
            let literal = &captures[1];
            let Some(amount) = parse_whole_amount(literal) else {
                return Some(whole_amount_failure(literal));
            };
            let unit = DateUnit::parse(&captures[2])?;
            let Some(parsed) = self.parse_date(&captures[3], now) else {
                return Some(out_of_range());
            };
            return Some(self.add(
                amount,
                unit,
                &parsed.zoned,
                parsed.includes_time,
                parsed.includes_seconds,
            ));
        }

        if let Some(captures) = p.in_offset.captures(text) {
            let literal = &captures[1];
            let Some(amount) = parse_whole_amount(literal) else {
                return Some(whole_amount_failure(literal));
            };
            let unit = DateUnit::parse(&captures[2])?;
            // `in 3 days` keeps the calendar-date behaviour; short durations count forward
            // from the current instant.
            let is_duration = unit.is_duration();
            let base = if is_duration {
                now.to_zoned(self.time_zone.clone())
            } else {
                match self.start_of_day(now) {
                    Some(start) => start,
                    None => return Some(out_of_range()),
                }
            };
            return Some(self.add(amount, unit, &base, is_duration, is_duration));
        }

        if p.now_operator.is_match(text) {
            return Some(SpecializedCalculation::Incomplete);
        }
        None
    }

    fn evaluate_explicit_difference(
        &self,
        p: &Patterns,
        text: &str,
        now: Timestamp,
    ) -> SpecializedCalculation {
        match p.and_split.captures(text) {
            Some(captures) => self.difference(captures[1].trim(), captures[2].trim(), now),
            None => SpecializedCalculation::Incomplete,
        }
    }

    fn difference(&self, left: &str, right: &str, now: Timestamp) -> SpecializedCalculation {
        let (Some(first), Some(second)) = (self.parse_date(left, now), self.parse_date(right, now))
        else {
            return SpecializedCalculation::Failure("Use valid dates, such as 2026-10-12.".into());
        };
        let span = second.zoned.date() - first.zoned.date();
        let days = i64::from(span.get_days());
        SpecializedCalculation::Result(if days == 1 {
            "1 day".into()
        } else {
            format!("{days} days")
        })
    }

    fn add(
        &self,
        amount: i64,
        unit: DateUnit,
        date: &Zoned,
        includes_time: bool,
        includes_seconds: bool,
    ) -> SpecializedCalculation {
        if !is_supported(date) {
            return out_of_range();
        }
        let result = if unit.is_duration() {
            // Bound elapsed-time arithmetic before building a duration.
            #[allow(clippy::cast_precision_loss)]
            let seconds = amount as f64 * unit.seconds() as f64;
            if seconds.abs() > MAXIMUM_ELAPSED_SECONDS {
                return out_of_range();
            }
            amount
                .checked_mul(unit.seconds())
                .and_then(|seconds| date.checked_add(SignedDuration::from_secs(seconds)).ok())
        } else {
            let maximum = unit.maximum_offset();
            if !(-maximum..=maximum).contains(&amount) {
                return out_of_range();
            }
            add_calendar(date, amount, unit)
        };

        let Some(result) = result else {
            return out_of_range();
        };
        let moves_correctly = amount == 0
            || if amount > 0 {
                result.timestamp() > date.timestamp()
            } else {
                result.timestamp() < date.timestamp()
            };
        if !is_supported(&result) || !moves_correctly {
            return out_of_range();
        }
        let shows_time = includes_time || unit.is_duration();
        SpecializedCalculation::Result(self.format(
            &result,
            shows_time,
            includes_seconds || unit == DateUnit::Second,
        ))
    }

    fn start_of_day(&self, now: Timestamp) -> Option<Zoned> {
        now.to_zoned(self.time_zone.clone()).start_of_day().ok()
    }

    fn parse_date(&self, raw_text: &str, now: Timestamp) -> Option<ParsedDate> {
        let text = raw_text.trim();
        let relative_days = match text.to_lowercase().as_str() {
            "now" => {
                let zoned = now.to_zoned(self.time_zone.clone());
                return is_supported(&zoned).then_some(ParsedDate {
                    zoned,
                    includes_time: true,
                    includes_seconds: true,
                });
            }
            "today" => Some(0),
            "tomorrow" => Some(1),
            "yesterday" => Some(-1),
            _ => None,
        };
        if let Some(days) = relative_days {
            let start = self.start_of_day(now)?;
            let zoned = if days == 0 {
                start
            } else {
                add_calendar(&start, days, DateUnit::Day)?
            };
            return Some(ParsedDate::date(zoned));
        }

        let p = patterns()?;
        if text.contains('T')
            && let Ok(timestamp) = text.parse::<Timestamp>()
        {
            return Some(ParsedDate {
                zoned: timestamp.to_zoned(self.time_zone.clone()),
                includes_time: true,
                includes_seconds: p.seconds.is_match(text),
            });
        }

        if let Some(captures) = p.iso_local.captures(text) {
            let number = |index: usize| -> Option<i32> {
                captures
                    .get(index)
                    .map_or(Some(0), |m| m.as_str().parse().ok())
            };
            let year = i16::try_from(number(1)?).ok()?;
            let month = i8::try_from(number(2)?).ok()?;
            let day = i8::try_from(number(3)?).ok()?;
            let hour = i8::try_from(number(4)?).ok()?;
            let minute = i8::try_from(number(5)?).ok()?;
            let second = i8::try_from(number(6)?).ok()?;
            let zoned = self.make_date(year, month, day, hour, minute, second)?;
            return Some(ParsedDate {
                zoned,
                includes_time: captures.get(4).is_some(),
                includes_seconds: captures.get(6).is_some(),
            });
        }

        self.parse_localized_date(text)
    }

    /// Numeric short dates in the locale's order with a four-digit year, for example
    /// `01/02/2026` (`en_US` January 2nd, `en_GB` February 1st).
    fn parse_localized_date(&self, text: &str) -> Option<ParsedDate> {
        let parts: Vec<&str> = text.split(self.locale.date_separator()).collect();
        let [first, second, third] = parts.as_slice() else {
            return None;
        };
        let number = |part: &str, min: usize, max: usize| -> Option<i32> {
            let valid =
                (min..=max).contains(&part.len()) && part.bytes().all(|b| b.is_ascii_digit());
            if valid { part.parse().ok() } else { None }
        };
        let (month, day) = match self.locale.date_order() {
            DateOrder::MonthDayYear => (number(first, 1, 2)?, number(second, 1, 2)?),
            DateOrder::DayMonthYear => (number(second, 1, 2)?, number(first, 1, 2)?),
        };
        let year = number(third, 4, 4)?;
        let date = Date::new(
            i16::try_from(year).ok()?,
            i8::try_from(month).ok()?,
            i8::try_from(day).ok()?,
        )
        .ok()?;
        let zoned = date.to_zoned(self.time_zone.clone()).ok()?;
        Some(ParsedDate::date(zoned))
    }

    /// Builds a local date-time, rejecting invalid components, year 0 and wall-clock times
    /// that do not exist in the zone (a daylight-saving gap).
    fn make_date(
        &self,
        year: i16,
        month: i8,
        day: i8,
        hour: i8,
        minute: i8,
        second: i8,
    ) -> Option<Zoned> {
        if year < 1 {
            return None;
        }
        let local = DateTime::new(year, month, day, hour, minute, second, 0).ok()?;
        let zoned = local.to_zoned(self.time_zone.clone()).ok()?;
        (zoned.datetime() == local).then_some(zoned)
    }

    fn format(&self, date: &Zoned, includes_time: bool, includes_seconds: bool) -> String {
        let (year, month, day) = (
            i32::from(date.year()),
            clamp_u8(date.month()),
            clamp_u8(date.day()),
        );
        if !includes_time {
            return self.locale.format_date(year, month, day, None);
        }
        let second = includes_seconds.then(|| clamp_u8(date.second()));
        self.locale.format_date_time(
            year,
            month,
            day,
            clamp_u8(date.hour()),
            clamp_u8(date.minute()),
            second,
        )
    }
}

fn clamp_u8(value: i8) -> u8 {
    u8::try_from(value).unwrap_or(0)
}

fn out_of_range() -> SpecializedCalculation {
    SpecializedCalculation::Failure(OUT_OF_RANGE.into())
}

/// A well-formed ISO date in year 9999 whose start of day lies after the last instant `jiff`
/// can represent (`9999-12-30T22:00Z`). Swift accepts such a date, but any offset from it
/// leaves the supported range, so callers report that instead of "invalid date".
fn is_beyond_timestamp_range(p: &Patterns, text: &str) -> bool {
    let Some(captures) = p.iso_local.captures(text) else {
        return false;
    };
    let number = |index: usize| -> Option<i8> {
        captures
            .get(index)
            .map_or(Some(0), |m| m.as_str().parse().ok())
    };
    let (Some(month), Some(day)) = (number(2), number(3)) else {
        return false;
    };
    &captures[1] == "9999" && Date::new(9_999, month, day).is_ok()
}

/// Era 1 (CE), years 1 through 9999.
fn is_supported(date: &Zoned) -> bool {
    (1..=9_999).contains(&date.year())
}

fn is_whole_amount(literal: &str) -> bool {
    let normalized = literal.replace(',', ".");
    let mut parts = normalized.split('.');
    parts.next();
    parts
        .next()
        .is_none_or(|fraction| fraction.chars().all(|c| c == '0'))
}

/// The integer part of a whole amount (`3`, `3.0`, `3,00`); `None` for fractions and for
/// values beyond `i64`.
fn parse_whole_amount(literal: &str) -> Option<i64> {
    if !is_whole_amount(literal) {
        return None;
    }
    let normalized = literal.replace(',', ".");
    let integer = normalized.split('.').next()?;
    integer.strip_prefix('+').unwrap_or(integer).parse().ok()
}

fn signed_amount(amount: i64, negative: bool) -> Option<i64> {
    if negative {
        amount.checked_neg()
    } else {
        Some(amount)
    }
}

/// A literal that is not a usable whole number: fractional, or too large for an `i64`.
fn whole_amount_failure(literal: &str) -> SpecializedCalculation {
    if is_whole_amount(literal) {
        out_of_range()
    } else {
        SpecializedCalculation::Failure(WHOLE_NUMBER.into())
    }
}

/// Calendar-aware addition: wall-clock time is kept and month ends are clamped.
fn add_calendar(date: &Zoned, amount: i64, unit: DateUnit) -> Option<Zoned> {
    let span = match unit {
        DateUnit::Day => Span::new().try_days(amount),
        DateUnit::Week => Span::new().try_weeks(amount),
        DateUnit::Month => Span::new().try_months(amount),
        DateUnit::Year => Span::new().try_years(amount),
        _ => return None,
    }
    .ok()?;
    date.checked_add(span).ok()
}

/// `^(?:[0-9]{4}-[0-9]{2}-[0-9]{2}|[0-9]{1,2}[./-][0-9]{1,2}[./-][0-9]{2,4})(?:$|[ T])`
fn is_date_candidate(text: &str) -> bool {
    let value = text.trim();
    if matches!(
        value.to_lowercase().as_str(),
        "now" | "today" | "tomorrow" | "yesterday"
    ) {
        return true;
    }
    let bytes = value.as_bytes();
    let digit_run = |from: usize| {
        bytes
            .get(from..)
            .map_or(0, |s| s.iter().take_while(|b| b.is_ascii_digit()).count())
    };
    let terminated = |at: usize| matches!(bytes.get(at), None | Some(b' ' | b'T'));

    // ISO form.
    if digit_run(0) == 4
        && bytes.get(4) == Some(&b'-')
        && digit_run(5) == 2
        && bytes.get(7) == Some(&b'-')
        && digit_run(8) == 2
        && terminated(10)
    {
        return true;
    }

    // Numeric short form.
    let is_separator = |at: usize| matches!(bytes.get(at), Some(b'.' | b'/' | b'-'));
    let first = digit_run(0);
    if !(1..=2).contains(&first) || !is_separator(first) {
        return false;
    }
    let second_start = first + 1;
    let second = digit_run(second_start);
    if !(1..=2).contains(&second) || !is_separator(second_start + second) {
        return false;
    }
    let third_start = second_start + second + 1;
    let third = digit_run(third_start);
    (2..=4).contains(&third) && terminated(third_start + third)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidates() {
        for yes in [
            "now",
            "today",
            "2026-10-12",
            "2026-10-12 10:00",
            "2026-10-12T10:00",
            "1/2/2026",
            "01.02.26 x",
            "1-2-2026",
        ] {
            assert!(is_date_candidate(yes), "{yes}");
        }
        for no in [
            "",
            "2026-10-1",
            "20261-10-12",
            "123/1/2026",
            "1/2/20265",
            "1/2/2",
            "km",
            "2026-10-12x",
        ] {
            assert!(!is_date_candidate(no), "{no}");
        }
    }

    #[test]
    fn amounts() {
        assert_eq!(parse_whole_amount("3"), Some(3));
        assert_eq!(parse_whole_amount("3.0"), Some(3));
        assert_eq!(parse_whole_amount("3,00"), Some(3));
        assert_eq!(parse_whole_amount("+7"), Some(7));
        assert_eq!(parse_whole_amount("-7"), Some(-7));
        assert_eq!(parse_whole_amount("1.5"), None);
        assert_eq!(parse_whole_amount("9223372036854775808"), None);
        assert!(is_whole_amount("9223372036854775808"));
        assert_eq!(signed_amount(i64::MIN, true), None);
    }

    #[test]
    fn unit_names() {
        for (name, unit) in [
            ("secs", DateUnit::Second),
            ("MIN", DateUnit::Minute),
            ("hrs", DateUnit::Hour),
            ("d", DateUnit::Day),
            ("wk", DateUnit::Week),
            ("mth", DateUnit::Month),
            ("Y", DateUnit::Year),
        ] {
            assert_eq!(DateUnit::parse(name), Some(unit), "{name}");
        }
        assert_eq!(DateUnit::parse("dayz"), None);
    }
}
