//! Named time-zone lookups and clock conversions: `time in Tokyo` and
//! `9:30 AM in New York to London`. Port of `TimeZoneCalculationEngine.swift`.
//!
//! Zones come from the injected `jiff` time-zone database (the system `tzdata` unless the
//! application bundles one), so daylight-saving rules are applied for the current date.

use std::collections::HashMap;
use std::sync::OnceLock;

use jiff::civil::DateTime;
use jiff::tz::TimeZone;
use jiff::{Timestamp, Zoned};
use regex::Regex;

use crate::calculator::SpecializedCalculation;
use crate::locale::Locale;

/// `.` in the original patterns does not match line terminators.
const ANY: &str = r"[^\n\r\x0B\x0C\x{85}\x{2028}\x{2029}]";

fn compile(cell: &'static OnceLock<Regex>, pattern: &str) -> Option<&'static Regex> {
    if let Some(regex) = cell.get() {
        return Some(regex);
    }
    let regex = Regex::new(pattern).ok()?;
    Some(cell.get_or_init(|| regex))
}

fn time_in_pattern() -> Option<&'static Regex> {
    static CELL: OnceLock<Regex> = OnceLock::new();
    compile(&CELL, &format!(r"(?i)^time\s+in\s*({ANY}*?)\s*$"))
}

fn pair_pattern() -> Option<&'static Regex> {
    static CELL: OnceLock<Regex> = OnceLock::new();
    compile(
        &CELL,
        &format!(
            r"(?i)^\s*([0-9]{{1,2}}(?::[0-9]{{2}})?\s*(?:am|pm)?)\s+(?:in\s+)?({ANY}+?)\s+to\s+({ANY}+?)\s*$"
        ),
    )
}

fn incomplete_pattern() -> Option<&'static Regex> {
    static CELL: OnceLock<Regex> = OnceLock::new();
    compile(
        &CELL,
        &format!(
            r"(?i)^time\s+in\s*$|^[0-9]{{1,2}}(?::[0-9]{{2}})?\s*(?:am|pm)?\s+(?:in\s+{ANY}+)?\s+to\s*$"
        ),
    )
}

fn meridiem_pattern() -> Option<&'static Regex> {
    static CELL: OnceLock<Regex> = OnceLock::new();
    compile(&CELL, r"(?i)\b(?:am|pm)\b")
}

fn clock_pattern() -> Option<&'static Regex> {
    static CELL: OnceLock<Regex> = OnceLock::new();
    compile(
        &CELL,
        r"(?i)^\s*([0-9]{1,2})(?::([0-9]{2}))?\s*(am|pm)?\s*$",
    )
}

/// Evaluates time-zone expressions in a locale.
#[derive(Debug, Clone, Default)]
pub(crate) struct TimeZoneCalculationEngine {
    locale: Locale,
}

impl TimeZoneCalculationEngine {
    pub(crate) fn new(locale: Locale) -> Self {
        Self { locale }
    }

    pub(crate) fn evaluate(&self, input: &str, now: Timestamp) -> SpecializedCalculation {
        let text = input.trim();
        let (Some(time_in), Some(pair), Some(incomplete)) =
            (time_in_pattern(), pair_pattern(), incomplete_pattern())
        else {
            return SpecializedCalculation::NotHandled;
        };

        if let Some(captures) = time_in.captures(text) {
            let zone_name = captures[1].trim();
            if zone_name.is_empty() {
                return SpecializedCalculation::Incomplete;
            }
            let Some(zone) = time_zone_named(zone_name) else {
                return unknown_zone(zone_name);
            };
            return SpecializedCalculation::Result(self.format(now, &zone));
        }

        let Some(captures) = pair.captures(text) else {
            if incomplete.is_match(text) {
                return SpecializedCalculation::Incomplete;
            }
            return SpecializedCalculation::NotHandled;
        };

        let time_text = captures[1].trim();
        let source_name = captures[2].trim();
        let target_name = captures[3].trim();
        // A bare number followed by a unit is usually a unit conversion such as
        // "12 km to mi". Require a clock marker before interpreting a zone pair.
        let has_marker = time_text.contains(':')
            || meridiem_pattern().is_some_and(|pattern| pattern.is_match(time_text));
        if !has_marker {
            return SpecializedCalculation::NotHandled;
        }
        let Some(source_zone) = time_zone_named(source_name) else {
            return unknown_zone(source_name);
        };
        let Some(target_zone) = time_zone_named(target_name) else {
            return unknown_zone(target_name);
        };
        let Some(source_time) = instant_for(time_text, &source_zone, now) else {
            return SpecializedCalculation::Failure(
                "Use a time such as “9:30 AM” or “17:30”.".into(),
            );
        };
        SpecializedCalculation::Result(self.format(source_time, &target_zone))
    }

    /// `Oct 13, 2026 at 21:00 (Japan Time, GMT+09:00)`.
    fn format(&self, instant: Timestamp, zone: &TimeZone) -> String {
        let local = instant.to_zoned(zone.clone());
        let date_time = self.locale.format_date_time(
            i32::from(local.year()),
            clamp_u8(local.month()),
            clamp_u8(local.day()),
            clamp_u8(local.hour()),
            clamp_u8(local.minute()),
            None,
        );
        let name = generic_name(zone);
        let offset = local.offset().seconds();
        let sign = if offset >= 0 { '+' } else { '−' };
        let absolute = offset.unsigned_abs();
        format!(
            "{date_time} ({name}, GMT{sign}{:02}:{:02})",
            absolute / 3_600,
            (absolute % 3_600) / 60
        )
    }
}

fn clamp_u8(value: i8) -> u8 {
    u8::try_from(value).unwrap_or(0)
}

fn unknown_zone(name: &str) -> SpecializedCalculation {
    SpecializedCalculation::Failure(format!("Unknown time zone “{name}”."))
}

/// The instant at which the wall clock in `zone` shows `raw_time` on the current local date of
/// `zone`; `None` for an invalid time or one that does not exist (daylight-saving gap).
fn instant_for(raw_time: &str, zone: &TimeZone, now: Timestamp) -> Option<Timestamp> {
    let captures = clock_pattern()?.captures(raw_time)?;
    let raw_hour: i8 = captures[1].parse().ok()?;
    let minute: i8 = captures
        .get(2)
        .map_or(Some(0), |m| m.as_str().parse().ok())?;
    if minute >= 60 {
        return None;
    }
    let hour = match captures.get(3) {
        Some(meridiem) => {
            if !(1..=12).contains(&raw_hour) {
                return None;
            }
            let pm = meridiem.as_str().eq_ignore_ascii_case("pm");
            (raw_hour % 12) + if pm { 12 } else { 0 }
        }
        None if (0..=23).contains(&raw_hour) => raw_hour,
        None => return None,
    };

    let today: Zoned = now.to_zoned(zone.clone());
    let wanted =
        DateTime::new(today.year(), today.month(), today.day(), hour, minute, 0, 0).ok()?;
    let zoned = wanted.to_zoned(zone.clone()).ok()?;
    (zoned.datetime() == wanted).then(|| zoned.timestamp())
}

fn time_zone_named(raw_name: &str) -> Option<TimeZone> {
    let lowered = raw_name.trim().to_lowercase().replace(['_', '-'], " ");
    let normalized = lowered.split_whitespace().collect::<Vec<_>>().join(" ");
    if let Some(&identifier) = aliases().get(normalized.as_str()) {
        return zone(identifier);
    }
    let identifier = raw_name.trim();
    let known = jiff::tz::db()
        .available()
        .any(|name| name.as_str() == identifier);
    if known { zone(identifier) } else { None }
}

fn zone(identifier: &str) -> Option<TimeZone> {
    if identifier == "UTC" {
        return Some(TimeZone::UTC);
    }
    match TimeZone::get(identifier) {
        Ok(zone) => Some(zone),
        // "GMT" is an alias of UTC; tolerate trimmed tzdata installations without the link.
        Err(_) if identifier == "GMT" => Some(TimeZone::UTC),
        Err(_) => None,
    }
}

/// English generic zone names (what Foundation's `localizedName(for: .generic)` returns for
/// the zones in the alias table). Any other zone falls back to its identifier.
fn generic_name(zone: &TimeZone) -> String {
    let Some(identifier) = zone.iana_name() else {
        return "GMT".into();
    };
    let name = match identifier {
        "UTC" | "GMT" => "GMT",
        "Europe/London" => "United Kingdom Time",
        "America/New_York" | "America/Toronto" => "Eastern Time",
        "America/Chicago" => "Central Time",
        "America/Denver" => "Mountain Time",
        "America/Phoenix" => "Mountain Time",
        "America/Los_Angeles" | "America/Vancouver" => "Pacific Time",
        "Pacific/Honolulu" => "Hawaii-Aleutian Time",
        "Europe/Warsaw" | "Europe/Berlin" | "Europe/Paris" | "Europe/Rome" | "Europe/Madrid"
        | "Europe/Amsterdam" | "Europe/Zurich" => "Central European Time",
        "Asia/Tokyo" => "Japan Time",
        "Asia/Shanghai" => "China Time",
        "Asia/Hong_Kong" => "Hong Kong Time",
        "Asia/Singapore" => "Singapore Standard Time",
        "Asia/Kolkata" => "India Standard Time",
        "Asia/Dubai" => "Gulf Standard Time",
        "Australia/Sydney" | "Australia/Melbourne" => "Eastern Australia Time",
        "Pacific/Auckland" => "New Zealand Time",
        other => other,
    };
    name.to_owned()
}

fn aliases() -> &'static HashMap<&'static str, &'static str> {
    static ALIASES: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    ALIASES.get_or_init(|| ALIASES_TABLE.iter().copied().collect())
}

#[rustfmt::skip]
const ALIASES_TABLE: &[(&str, &str)] = &[
    ("utc", "UTC"), ("gmt", "GMT"), ("zulu", "UTC"),
    ("london", "Europe/London"), ("uk", "Europe/London"), ("britain", "Europe/London"), ("bst", "Europe/London"),
    ("new york", "America/New_York"), ("new york city", "America/New_York"), ("nyc", "America/New_York"), ("eastern", "America/New_York"), ("et", "America/New_York"), ("est", "America/New_York"), ("edt", "America/New_York"),
    ("chicago", "America/Chicago"), ("central", "America/Chicago"), ("ct", "America/Chicago"), ("cst", "America/Chicago"), ("cdt", "America/Chicago"),
    ("denver", "America/Denver"), ("mountain", "America/Denver"), ("mt", "America/Denver"), ("mst", "America/Denver"), ("mdt", "America/Denver"),
    ("phoenix", "America/Phoenix"),
    ("los angeles", "America/Los_Angeles"), ("la", "America/Los_Angeles"), ("san francisco", "America/Los_Angeles"), ("pacific", "America/Los_Angeles"), ("pt", "America/Los_Angeles"), ("pst", "America/Los_Angeles"), ("pdt", "America/Los_Angeles"),
    ("toronto", "America/Toronto"), ("vancouver", "America/Vancouver"), ("honolulu", "Pacific/Honolulu"),
    ("warsaw", "Europe/Warsaw"), ("poland", "Europe/Warsaw"),
    ("berlin", "Europe/Berlin"), ("paris", "Europe/Paris"), ("rome", "Europe/Rome"), ("madrid", "Europe/Madrid"), ("amsterdam", "Europe/Amsterdam"), ("zurich", "Europe/Zurich"), ("cet", "Europe/Paris"), ("cest", "Europe/Paris"),
    ("tokyo", "Asia/Tokyo"), ("japan", "Asia/Tokyo"), ("osaka", "Asia/Tokyo"),
    ("beijing", "Asia/Shanghai"), ("shanghai", "Asia/Shanghai"), ("china", "Asia/Shanghai"), ("hong kong", "Asia/Hong_Kong"), ("singapore", "Asia/Singapore"),
    ("mumbai", "Asia/Kolkata"), ("delhi", "Asia/Kolkata"), ("kolkata", "Asia/Kolkata"), ("india", "Asia/Kolkata"), ("ist", "Asia/Kolkata"),
    ("dubai", "Asia/Dubai"), ("sydney", "Australia/Sydney"), ("melbourne", "Australia/Melbourne"), ("auckland", "Pacific/Auckland"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aliases_are_unique_and_resolve_when_tzdata_is_present() {
        assert_eq!(aliases().len(), ALIASES_TABLE.len(), "duplicate alias");
        assert!(time_zone_named("New_York").is_some() || jiff::tz::db().available().count() == 0);
    }

    #[test]
    fn generic_names_fall_back_to_the_identifier() {
        assert_eq!(generic_name(&TimeZone::UTC), "GMT");
        let zone = TimeZone::get("Europe/Lisbon");
        if let Ok(zone) = zone {
            assert_eq!(generic_name(&zone), "Europe/Lisbon");
        }
    }
}
