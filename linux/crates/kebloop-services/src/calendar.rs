//! Explicitly selected local iCalendar files; no account access or automatic joining.
use crate::{ConfigError, util::atomic_write};
use jiff::{Timestamp, civil::DateTime, tz::TimeZone};
use serde::{Deserialize, Serialize};
use std::{fs, io::Read, os::unix::fs::OpenOptionsExt, path::Path};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarEvent {
    pub title: String,
    pub start: Timestamp,
    pub end: Timestamp,
    pub meeting_url: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct CalendarSettings {
    pub files: Vec<String>,
}
impl CalendarSettings {
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let data = read_regular(path, 64 * 1024).map_err(ConfigError::invalid)?;
        if data.len() > 64 * 1024 {
            return Err(ConfigError::Invalid(
                "Calendar configuration is too large".into(),
            ));
        }
        let settings: Self = serde_json::from_slice(&data)?;
        if settings.files.len() > 32 {
            return Err(ConfigError::invalid("Choose at most 32 calendars"));
        }
        Ok(settings)
    }
    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        if self.files.len() > 32 {
            return Err(ConfigError::invalid("Choose at most 32 calendars"));
        }
        atomic_write(path, &serde_json::to_vec(self)?, 0o600)
    }
}

pub fn read_events(
    settings: &CalendarSettings,
    now: Timestamp,
    zone: &TimeZone,
) -> Result<Vec<CalendarEvent>, String> {
    let mut events = Vec::new();
    for path in &settings.files {
        let data = read_regular(Path::new(path), 2 * 1024 * 1024)?;
        let text = String::from_utf8(data).map_err(|_| "Calendar must be UTF-8".to_owned())?;
        events.extend(parse_events(&text, now, zone)?);
        if events.len() > 5000 {
            return Err("Calendar contains too many events".into());
        }
    }
    events.sort_by_key(|event| event.start);
    Ok(events)
}

fn read_regular(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_NONBLOCK)
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("Choose a regular calendar file".into());
    }
    let mut data = Vec::new();
    file.take(limit as u64 + 1)
        .read_to_end(&mut data)
        .map_err(|e| e.to_string())?;
    if data.len() > limit {
        return Err("Calendar file is too large".into());
    }
    Ok(data)
}

pub fn meeting_url(text: &str) -> Option<String> {
    text.split_whitespace()
        .filter_map(|part| {
            let part = part.trim_matches(['<', '>', '"', '(', ')', ',', ';']);
            let url = url::Url::parse(part).ok()?;
            if url.scheme() != "https" {
                return None;
            }
            let host = url.host_str()?;
            let supported = [
                "zoom.us",
                "meet.google.com",
                "teams.microsoft.com",
                "teams.live.com",
                "webex.com",
                "whereby.com",
            ];
            supported
                .iter()
                .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")))
                .then(|| url.to_string())
        })
        .next()
}

pub fn parse_events(
    text: &str,
    now: Timestamp,
    zone: &TimeZone,
) -> Result<Vec<CalendarEvent>, String> {
    let mut lines: Vec<String> = Vec::new();
    for line in text.lines() {
        if line.starts_with([' ', '\t']) {
            if let Some(previous) = lines.last_mut() {
                previous.push_str(&line[1..]);
            }
        } else {
            lines.push(line.trim_end_matches('\r').into());
        }
    }
    let mut fields = Vec::new();
    let mut active = false;
    let mut events = Vec::new();
    for line in lines {
        if line == "BEGIN:VEVENT" {
            active = true;
            fields.clear();
            continue;
        }
        if line == "END:VEVENT" && active {
            active = false;
            let get = |name: &str| {
                fields
                    .iter()
                    .find(|(key, _): &&(String, String)| key.split(';').next() == Some(name))
            };
            if get("STATUS").is_some_and(|(_, v)| v == "CANCELLED") {
                continue;
            }
            if get("RECURRENCE-ID").is_some() {
                return Err("Calendar recurrence exceptions require an expanded export.".into());
            }
            let Some((start_key, start_value)) = get("DTSTART") else {
                continue;
            };
            if start_key.contains("VALUE=DATE") || start_value.len() == 8 {
                continue;
            }
            let start = parse_time(start_key, start_value, zone)?;
            let end = get("DTEND")
                .map(|(k, v)| parse_time(k, v, zone))
                .transpose()?
                .unwrap_or(
                    start
                        .checked_add(jiff::Span::new().hours(1))
                        .map_err(|e| e.to_string())?,
                );
            let starts = if let Some((_, rule)) = get("RRULE") {
                expand_recurrence(start, rule, start_key, start_value, now, zone)?
            } else {
                vec![start]
            };
            let duration = end
                .as_second()
                .checked_sub(start.as_second())
                .filter(|v| *v > 0)
                .ok_or("Calendar event has an invalid duration")?;
            let exclusions = fields
                .iter()
                .filter(|(key, _)| key.split(';').next() == Some("EXDATE"))
                .flat_map(|(key, value)| {
                    value
                        .split(',')
                        .map(move |value| parse_time(key, value, zone))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let title = get("SUMMARY")
                .map(|(_, v)| unescape(v))
                .unwrap_or_else(|| "Untitled event".into());
            let links = ["URL", "LOCATION", "DESCRIPTION"]
                .iter()
                .filter_map(|name| get(name).map(|(_, v)| unescape(v)))
                .collect::<Vec<_>>()
                .join(" ");
            let horizon = now
                .checked_add(jiff::Span::new().hours(24 * 30))
                .map_err(|e| e.to_string())?;
            for occurrence in starts {
                let end = occurrence
                    .checked_add(jiff::Span::new().seconds(duration))
                    .map_err(|e| e.to_string())?;
                if end <= now || occurrence > horizon || exclusions.contains(&occurrence) {
                    continue;
                }
                events.push(CalendarEvent {
                    title: title.clone(),
                    start: occurrence,
                    end,
                    meeting_url: meeting_url(&links),
                });
            }
            continue;
        }
        if active && let Some((key, value)) = line.split_once(':') {
            fields.push((key.into(), value.into()));
        }
    }
    Ok(events)
}
fn unescape(value: &str) -> String {
    value
        .replace("\\n", "\n")
        .replace("\\N", "\n")
        .replace("\\,", ",")
        .replace("\\;", ";")
        .replace("\\\\", "\\")
}
fn parse_time(key: &str, value: &str, zone: &TimeZone) -> Result<Timestamp, String> {
    let utc = value.ends_with('Z');
    let value = value.trim_end_matches('Z');
    if value.len() != 15 || !value.is_ascii() || value.as_bytes()[8] != b'T' {
        return Err("Unsupported calendar date".into());
    }
    let iso = format!(
        "{}-{}-{}T{}:{}:{}",
        &value[..4],
        &value[4..6],
        &value[6..8],
        &value[9..11],
        &value[11..13],
        &value[13..]
    );
    let date: DateTime = iso.parse().map_err(|e: jiff::Error| e.to_string())?;
    let tz = if utc {
        TimeZone::UTC
    } else if let Some(tz) = key.split("TZID=").nth(1) {
        TimeZone::get(tz.split(';').next().unwrap_or("")).map_err(|e| e.to_string())?
    } else {
        zone.clone()
    };
    date.to_zoned(tz)
        .map(|v| v.timestamp())
        .map_err(|e| e.to_string())
}

/// Expand common daily/weekly exports using civil dates so DST retains the event's wall time.
fn expand_recurrence(
    start: Timestamp,
    rule: &str,
    key: &str,
    value: &str,
    now: Timestamp,
    zone: &TimeZone,
) -> Result<Vec<Timestamp>, String> {
    let properties = rule
        .split(';')
        .map(|part| part.split_once('=').ok_or("Invalid recurrence rule"))
        .collect::<Result<std::collections::BTreeMap<_, _>, _>>()?;
    if properties
        .keys()
        .any(|key| !["FREQ", "INTERVAL", "COUNT", "UNTIL", "BYDAY", "WKST"].contains(key))
    {
        return Err(
            "This calendar uses an unsupported recurrence rule. Export expanded occurrences."
                .into(),
        );
    }
    let frequency = properties
        .get("FREQ")
        .copied()
        .ok_or("Recurrence has no frequency")?;
    if !["DAILY", "WEEKLY"].contains(&frequency) {
        return Err("Only daily and weekly recurrence is supported; export expanded occurrences for other rules.".into());
    }
    let interval = properties
        .get("INTERVAL")
        .map(|s| s.parse::<i64>())
        .transpose()
        .map_err(|e| e.to_string())?
        .unwrap_or(1);
    if !(1..=366).contains(&interval) {
        return Err("Invalid recurrence interval".into());
    }
    let count = properties
        .get("COUNT")
        .map(|s| s.parse::<usize>())
        .transpose()
        .map_err(|e| e.to_string())?
        .unwrap_or(usize::MAX);
    if count == 0 {
        return Err("Invalid recurrence count".into());
    }
    let until = properties
        .get("UNTIL")
        .map(|v| parse_time(key, v, zone))
        .transpose()?;
    let weekdays = properties
        .get("BYDAY")
        .map(|v| {
            v.split(',')
                .map(|d| match d {
                    "MO" => Ok(0),
                    "TU" => Ok(1),
                    "WE" => Ok(2),
                    "TH" => Ok(3),
                    "FR" => Ok(4),
                    "SA" => Ok(5),
                    "SU" => Ok(6),
                    _ => Err("Unsupported recurrence weekday"),
                })
                .collect::<Result<Vec<i8>, _>>()
        })
        .transpose()?;
    let week_start = properties.get("WKST").copied().unwrap_or("MO");
    let week_offset = match week_start {
        "MO" => 0,
        "TU" => 1,
        "WE" => 2,
        "TH" => 3,
        "FR" => 4,
        "SA" => 5,
        "SU" => 6,
        _ => return Err("Invalid recurrence week start".into()),
    };
    let event_zone = if value.ends_with('Z') {
        TimeZone::UTC
    } else if let Some(tz) = key.split("TZID=").nth(1) {
        TimeZone::get(tz.split(';').next().unwrap_or("")).map_err(|e| e.to_string())?
    } else {
        zone.clone()
    };
    let first = start.to_zoned(event_zone);
    let anchor_day = first.date().weekday().to_monday_zero_offset();
    let horizon = now
        .checked_add(jiff::Span::new().hours(24 * 30))
        .map_err(|e| e.to_string())?;
    let mut output = Vec::new();
    let mut occurrences = 0usize;
    let mut finished = false;
    for day in 0..40_000i64 {
        let candidate = first
            .checked_add(jiff::Span::new().days(day))
            .map_err(|e| e.to_string())?;
        if candidate.timestamp() > horizon
            || until.is_some_and(|until| candidate.timestamp() > until)
        {
            finished = true;
            break;
        }
        let weekday = candidate.date().weekday().to_monday_zero_offset();
        let permitted = if frequency == "DAILY" {
            day % interval == 0 && weekdays.as_ref().is_none_or(|days| days.contains(&weekday))
        } else {
            let week = (day + i64::from((anchor_day - week_offset + 7) % 7)) / 7;
            week % interval == 0
                && weekdays
                    .as_ref()
                    .map_or(weekday == anchor_day, |days| days.contains(&weekday))
        };
        if !permitted {
            continue;
        }
        occurrences += 1;
        if occurrences > count {
            finished = true;
            break;
        }
        // Include ongoing occurrences: the caller filters by the original event duration.
        if candidate.timestamp().as_second() >= now.as_second() - 7 * 24 * 3600 {
            output.push(candidate.timestamp());
        }
        if output.len() > 5000 {
            return Err("Calendar recurrence has too many occurrences".into());
        }
    }
    if !finished {
        return Err("Calendar recurrence exceeds the supported date range".into());
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn calendar_reads_are_bounded_and_declines_do_not_hide_other_attendees() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("large.ics");
        std::fs::write(&path, vec![b'x'; 100]).unwrap();
        assert!(read_regular(&path, 99).is_err());
        let fifo = directory.path().join("pipe.ics");
        nix::unistd::mkfifo(&fifo, nix::sys::stat::Mode::S_IRUSR).unwrap();
        assert!(read_regular(&fifo, 100).is_err());
        let now: Timestamp = "2026-10-13T12:00:00Z".parse().unwrap();
        let text = "BEGIN:VEVENT\nDTSTART:20261013T130000Z\nDTEND:20261013T140000Z\nATTENDEE;PARTSTAT=DECLINED:mailto:another@example.com\nEND:VEVENT";
        assert_eq!(parse_events(text, now, &TimeZone::UTC).unwrap().len(), 1);
    }
    #[test]
    fn weekly_expansion_retains_wall_time_across_dst_and_excludes_dates() {
        let now: Timestamp = "2026-10-20T00:00:00Z".parse().unwrap();
        let text = "BEGIN:VEVENT\nDTSTART;TZID=Europe/Warsaw:20261019T100000\nDTEND;TZID=Europe/Warsaw:20261019T110000\nRRULE:FREQ=WEEKLY;BYDAY=MO,WE;COUNT=4\nEXDATE;TZID=Europe/Warsaw:20261021T100000\nSUMMARY:Weekly\nEND:VEVENT";
        let events = parse_events(text, now, &TimeZone::UTC).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].start.to_string(), "2026-10-26T09:00:00Z");
        assert_eq!(events[1].start.to_string(), "2026-10-28T09:00:00Z");
    }
    #[test]
    fn malformed_dates_and_unsupported_rules_are_explicit() {
        let now: Timestamp = "2026-10-13T12:00:00Z".parse().unwrap();
        assert!(parse_time("DTSTART", "bad", &TimeZone::UTC).is_err());
        assert!(
            expand_recurrence(
                now,
                "FREQ=YEARLY",
                "DTSTART",
                "20261013T120000Z",
                now,
                &TimeZone::UTC
            )
            .is_err()
        );
    }
    #[test]
    fn filters_finished_all_day_and_cancelled_and_checks_join_urls() {
        let now: Timestamp = "2026-10-13T12:00:00Z".parse().unwrap();
        let text = "BEGIN:VEVENT\nDTSTART:20261013T130000Z\nDTEND:20261013T140000Z\nSUMMARY:Meeting\nDESCRIPTION:Join https://meet.google.com/abc-defg-hij\nEND:VEVENT\nBEGIN:VEVENT\nDTSTART;VALUE=DATE:20261013\nEND:VEVENT\nBEGIN:VEVENT\nDTSTART:20261013T090000Z\nDTEND:20261013T100000Z\nEND:VEVENT";
        let events = parse_events(text, now, &TimeZone::UTC).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].meeting_url.as_deref(),
            Some("https://meet.google.com/abc-defg-hij")
        );
        assert!(meeting_url("javascript:alert(1) https://evilzoom.us/join").is_none());
    }
}
