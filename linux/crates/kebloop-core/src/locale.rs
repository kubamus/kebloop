//! A small, injectable locale for number and date formatting and numeric date input.
//!
//! macOS delegates this to `Locale`/`DateFormatter`; Linux has no equivalent in the
//! standard library, so the few conventions Kebloop needs are tabulated here for
//! English (US and UK), Polish and German. Unknown tags fall back to `en_US`.

/// Order of the numeric components in a short date such as `01/02/2026`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateOrder {
    /// Month, day, year (`en_US`).
    MonthDayYear,
    /// Day, month, year (`en_GB`, `pl`, `de`).
    DayMonthYear,
}

/// Shape of the "medium" date shown in results.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MediumStyle {
    /// `Feb 28, 2026`
    MonthNameDayYear,
    /// `28 Feb 2026`
    DayMonthNameYear,
    /// `28.02.2026`
    DottedNumeric,
}

const MONTHS_EN: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const MONTHS_EN_GB: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sept", "Oct", "Nov", "Dec",
];
const MONTHS_PL: [&str; 12] = [
    "sty", "lut", "mar", "kwi", "maj", "cze", "lip", "sie", "wrz", "paź", "lis", "gru",
];

/// Formatting and parsing conventions for one locale.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Locale {
    tag: String,
    decimal_separator: char,
    date_order: DateOrder,
    date_separator: char,
    medium: MediumStyle,
    /// 12-hour clock for the short time style (`3:04 PM`); the date-time forms are always 24-hour.
    hour12: bool,
    /// Text between the date and the time in date-time results.
    date_time_joiner: &'static str,
    months: [&'static str; 12],
}

impl Default for Locale {
    fn default() -> Self {
        Self::en_us()
    }
}

impl Locale {
    /// American English: `Feb 28, 2026`, `12:00 PM`, `M/d/y` input.
    pub fn en_us() -> Self {
        Self {
            tag: "en_US".into(),
            decimal_separator: '.',
            date_order: DateOrder::MonthDayYear,
            date_separator: '/',
            medium: MediumStyle::MonthNameDayYear,
            hour12: true,
            date_time_joiner: " at ",
            months: MONTHS_EN,
        }
    }

    /// British English: `28 Feb 2026`, `12:00`, `d/M/y` input.
    pub fn en_gb() -> Self {
        Self {
            tag: "en_GB".into(),
            decimal_separator: '.',
            date_order: DateOrder::DayMonthYear,
            date_separator: '/',
            medium: MediumStyle::DayMonthNameYear,
            hour12: false,
            date_time_joiner: " at ",
            months: MONTHS_EN_GB,
        }
    }

    /// Polish: `28 lut 2026`, `12:00`, decimal comma, `d.M.y` input.
    pub fn pl() -> Self {
        Self {
            tag: "pl_PL".into(),
            decimal_separator: ',',
            date_order: DateOrder::DayMonthYear,
            date_separator: '.',
            medium: MediumStyle::DayMonthNameYear,
            hour12: false,
            date_time_joiner: ", ",
            months: MONTHS_PL,
        }
    }

    /// German: `28.02.2026`, `12:00`, decimal comma, `d.M.y` input.
    pub fn de() -> Self {
        Self {
            tag: "de_DE".into(),
            decimal_separator: ',',
            date_order: DateOrder::DayMonthYear,
            date_separator: '.',
            medium: MediumStyle::DottedNumeric,
            hour12: false,
            date_time_joiner: ", ",
            months: MONTHS_EN,
        }
    }

    /// Resolves a POSIX or BCP 47 tag such as `pl_PL.UTF-8`, `en-GB` or `de`.
    /// Unknown languages fall back to [`Locale::en_us`].
    pub fn from_tag(tag: &str) -> Self {
        let base = tag.split(['.', '@']).next().unwrap_or("");
        let normalized = base.replace('-', "_");
        let mut parts = normalized.split('_');
        let language = parts.next().unwrap_or("").to_ascii_lowercase();
        let region = parts.next().unwrap_or("").to_ascii_uppercase();
        match language.as_str() {
            "pl" => Self::pl(),
            "de" => Self::de(),
            "en" if matches!(region.as_str(), "GB" | "AU" | "NZ" | "IE" | "ZA" | "IN") => {
                Self::en_gb()
            }
            _ => Self::en_us(),
        }
    }

    /// The canonical tag of this locale, for example `en_US`.
    pub fn tag(&self) -> &str {
        &self.tag
    }

    /// The decimal mark used in displayed numbers.
    pub fn decimal_separator(&self) -> char {
        self.decimal_separator
    }

    /// The order of components in numeric short dates.
    pub fn date_order(&self) -> DateOrder {
        self.date_order
    }

    /// The separator between components in numeric short dates.
    pub fn date_separator(&self) -> char {
        self.date_separator
    }

    /// Formats a date (and optionally a time) like `DateFormatter` with the `yMMMd` /
    /// `yMMMdHHmm` skeletons: a medium date and a 24-hour `HH:mm` time. Plain ASCII spaces
    /// are used so results paste cleanly.
    pub fn format_date(&self, year: i32, month: u8, day: u8, time: Option<(u8, u8)>) -> String {
        match time {
            Some((hour, minute)) => self.format_date_time(year, month, day, hour, minute, None),
            None => self.medium_date(year, month, day),
        }
    }

    /// Like [`Locale::format_date`] with a time, optionally down to the second (`yMMMdHHmmss`).
    pub fn format_date_time(
        &self,
        year: i32,
        month: u8,
        day: u8,
        hour: u8,
        minute: u8,
        second: Option<u8>,
    ) -> String {
        let date = self.medium_date(year, month, day);
        let joiner = self.date_time_joiner;
        match second {
            Some(second) => format!("{date}{joiner}{hour:02}:{minute:02}:{second:02}"),
            None => format!("{date}{joiner}{hour:02}:{minute:02}"),
        }
    }

    /// The short time style used by snippets: `3:04 AM` (`en_US`) or `03:04`.
    pub fn format_time_short(&self, hour: u8, minute: u8) -> String {
        if self.hour12 {
            let suffix = if hour < 12 { "AM" } else { "PM" };
            let hour12 = match hour % 12 {
                0 => 12,
                other => other,
            };
            format!("{hour12}:{minute:02} {suffix}")
        } else {
            format!("{hour:02}:{minute:02}")
        }
    }

    fn medium_date(&self, year: i32, month: u8, day: u8) -> String {
        let month_name = self.months[usize::from(month.clamp(1, 12)) - 1];
        match self.medium {
            MediumStyle::MonthNameDayYear => format!("{month_name} {day}, {year}"),
            MediumStyle::DayMonthNameYear => format!("{day} {month_name} {year}"),
            MediumStyle::DottedNumeric => format!("{day:02}.{month:02}.{year}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_resolve_to_known_locales() {
        assert_eq!(Locale::from_tag("pl_PL.UTF-8").tag(), "pl_PL");
        assert_eq!(Locale::from_tag("en-GB").tag(), "en_GB");
        assert_eq!(Locale::from_tag("de").tag(), "de_DE");
        assert_eq!(Locale::from_tag("C").tag(), "en_US");
        assert_eq!(Locale::from_tag("").tag(), "en_US");
    }

    #[test]
    fn formats_dates_and_times() {
        assert_eq!(
            Locale::en_us().format_date(2026, 2, 28, None),
            "Feb 28, 2026"
        );
        assert_eq!(
            Locale::en_us().format_date(2026, 3, 8, Some((0, 5))),
            "Mar 8, 2026 at 00:05"
        );
        assert_eq!(
            Locale::en_us().format_date_time(2026, 10, 13, 2, 34, Some(56)),
            "Oct 13, 2026 at 02:34:56"
        );
        assert_eq!(Locale::en_us().format_time_short(15, 4), "3:04 PM");
        assert_eq!(Locale::en_us().format_time_short(0, 4), "12:04 AM");
        assert_eq!(Locale::pl().format_time_short(3, 4), "03:04");
        assert_eq!(Locale::en_gb().format_date(2026, 2, 2, None), "2 Feb 2026");
        assert_eq!(
            Locale::pl().format_date(2026, 10, 12, Some((9, 30))),
            "12 paź 2026, 09:30"
        );
        assert_eq!(Locale::de().format_date(2026, 2, 8, None), "08.02.2026");
    }
}
