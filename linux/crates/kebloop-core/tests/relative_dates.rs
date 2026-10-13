//! Ports of `Tests/KebloopCoreTests/RelativeDateTests.swift` and `RelativeDateSyntaxTests.swift`.

use jiff::civil;
use jiff::tz::TimeZone;
use kebloop_core::{CalculationOutcome, CalculationResult, CalculatorEngine, Locale};

fn zone(name: &str) -> TimeZone {
    TimeZone::get(name).unwrap_or_else(|error| panic!("missing test time zone {name}: {error}"))
}

/// 2026-10-13T02:34:56Z in `zone_name`.
fn calculator(zone_name: &str) -> CalculatorEngine {
    let now = "2026-10-13T02:34:56Z".parse().expect("valid timestamp");
    CalculatorEngine::new(Locale::en_us(), zone(zone_name)).with_reference_time(now)
}

/// A local wall-clock "now" in `zone_name`.
fn calculator_at(zone_name: &str, local: (i16, i8, i8, i8, i8, i8)) -> CalculatorEngine {
    let time_zone = zone(zone_name);
    let now = civil::date(local.0, local.1, local.2)
        .at(local.3, local.4, local.5, 0)
        .to_zoned(time_zone.clone())
        .expect("valid local time")
        .timestamp();
    CalculatorEngine::new(Locale::en_us(), time_zone).with_reference_time(now)
}

fn result(expression: &str, engine: &CalculatorEngine) -> CalculationResult {
    match engine.evaluate(expression) {
        CalculationOutcome::Result(result) => result,
        other => panic!("expected a result for {expression:?}, got {other:?}"),
    }
}

fn display(expression: &str, engine: &CalculatorEngine) -> String {
    result(expression, engine).display
}

fn failure(expression: &str, engine: &CalculatorEngine) -> String {
    match engine.evaluate(expression) {
        CalculationOutcome::Failure(message) => message,
        other => panic!("expected a failure for {expression:?}, got {other:?}"),
    }
}

// ----- RelativeDateTests -----

#[test]
fn standalone_now_and_today_respect_local_date_and_time() {
    let engine = calculator("America/New_York");
    let today = result("  TODAY  ", &engine);
    assert_eq!(today.display, "Oct 12, 2026");
    assert_eq!(today.clipboard, today.display);
    let now = result("now", &engine);
    assert!(now.display.starts_with("Oct 12, 2026"), "{}", now.display);
    assert!(now.display.contains("22:34"), "{}", now.display);
    assert!(
        !(now.display.ends_with("AM") || now.display.ends_with("PM")),
        "{}",
        now.display
    );
    assert_eq!(now.clipboard, now.display);
    assert_eq!(
        display("now + 1 day", &engine),
        now.display.replace("Oct 12, 2026", "Oct 13, 2026")
    );
    assert_eq!(
        engine.evaluate("today notes"),
        CalculationOutcome::None,
        "only exact relative keywords are calculations"
    );
    assert_eq!(
        engine.evaluate("now +"),
        CalculationOutcome::Incomplete,
        "an unfinished now offset stays incomplete"
    );
}

#[test]
fn overflow_and_unrepresentable_dates_fail_instead_of_returning_today() {
    let engine = calculator("UTC");
    let huge = "9".repeat(100);
    let amounts = [
        "9223372036854775807",
        "9223372036854775808",
        "18446744073709551616",
        huge.as_str(),
        "3660001",
    ];
    for amount in amounts {
        for expression in [
            format!("{amount} days from today"),
            format!("in {amount} days"),
            format!("today + {amount} days"),
            format!("today - {amount} days"),
        ] {
            let message = failure(&expression, &engine);
            assert!(
                message.contains("supported range"),
                "{expression}: {message}"
            );
        }
    }
    for expression in [
        "9999 years from today",
        "119988 months from today",
        "522000 weeks from today",
        "today + 3650000 days",
        "9999-12-31 + 1 day",
        "0001-01-01 - 1 day",
    ] {
        failure(expression, &engine);
    }
}

#[test]
fn valid_large_offsets_zero_and_integer_decimals_keep_working() {
    let engine = calculator("UTC");
    assert_eq!(display("0 days from today", &engine), "Oct 13, 2026");
    assert_eq!(display("3 days from today", &engine), "Oct 16, 2026");
    assert_eq!(display("today + 3.0 days", &engine), "Oct 16, 2026");
    assert_eq!(display("today - 3,0 days", &engine), "Oct 10, 2026");
    assert_eq!(
        display("1000000 days from today", &engine),
        display("today + 1000000 days", &engine)
    );
    assert_ne!(
        display("1000000 days from today", &engine),
        display("today", &engine)
    );
    failure("today + 1.5 days", &engine); // fractional calendar days remain unsupported
}

// ----- RelativeDateSyntaxTests -----

#[test]
fn compact_and_spaced_unit_aliases_are_accepted() {
    let engine = calculator("UTC");
    let aliases = [
        ("s", "sec", "second"),
        ("secs", "seconds", "second"),
        ("m", "min", "minute"),
        ("mins", "minutes", "minute"),
        ("h", "hr", "hour"),
        ("hrs", "hours", "hour"),
        ("d", "day", "day"),
        ("days", "day", "day"),
        ("w", "wk", "week"),
        ("wks", "weeks", "week"),
        ("mo", "mth", "month"),
        ("mos", "months", "month"),
        ("yr", "y", "year"),
        ("yrs", "years", "year"),
    ];
    for (compact, spaced, canonical) in aliases {
        let base = if ["second", "minute", "hour"].contains(&canonical) {
            "now"
        } else {
            "today"
        };
        let expected = display(&format!("1 {canonical} from {base}"), &engine);
        assert_eq!(
            display(&format!("1{compact} from {base}"), &engine),
            expected,
            "{compact}"
        );
        assert_eq!(
            display(&format!("1 {spaced} from {base}"), &engine),
            expected,
            "{spaced}"
        );
    }
}

#[test]
fn relative_forms_use_the_selected_now_or_today_base() {
    let engine = calculator("UTC");
    assert_eq!(
        display("3days from now", &engine),
        display("now + 3 d", &engine)
    );
    assert_eq!(
        display("3d from now", &engine),
        display("now + 3days", &engine)
    );
    assert_eq!(
        display("3 day from today", &engine),
        display("today + 3days", &engine)
    );
    assert_eq!(display("in 2h", &engine), display("now + 2 hours", &engine));
    assert_eq!(display("now-3d", &engine), "Oct 10, 2026 at 02:34:56");
    assert_eq!(display("today + 1d", &engine), "Oct 14, 2026");
    assert_eq!(
        display("-1 day from now", &engine),
        "Oct 12, 2026 at 02:34:56"
    );
}

#[test]
fn today_subday_offsets_show_clock_time_and_second_precision() {
    let engine = calculator("UTC");
    assert_eq!(display("today + 1h", &engine), "Oct 13, 2026 at 01:00");
    assert_eq!(display("today + 1min", &engine), "Oct 13, 2026 at 00:01");
    assert_eq!(display("today+1s", &engine), "Oct 13, 2026 at 00:00:01");
    assert_eq!(display("now", &engine), "Oct 13, 2026 at 02:34:56");
    assert_eq!(display("now + 1sec", &engine), "Oct 13, 2026 at 02:34:57");

    assert_eq!(
        display("2hours from today", &engine),
        "Oct 13, 2026 at 02:00"
    );
    assert_eq!(display("today + 2 hours", &engine), "Oct 13, 2026 at 02:00");
    assert_eq!(
        display("30 minutes from today", &engine),
        "Oct 13, 2026 at 00:30"
    );
    assert_eq!(display("today + 30min", &engine), "Oct 13, 2026 at 00:30");
    assert_eq!(
        display("45 seconds from today", &engine),
        "Oct 13, 2026 at 00:00:45"
    );
    assert_eq!(display("today + 45s", &engine), "Oct 13, 2026 at 00:00:45");
}

#[test]
fn from_now_calendar_offsets_retain_local_time_across_both_dst_changes() {
    assert_eq!(
        display(
            "1 day from now",
            &calculator_at("America/New_York", (2026, 3, 7, 12, 0, 0))
        ),
        "Mar 8, 2026 at 12:00:00"
    );
    assert_eq!(
        display(
            "1 day from now",
            &calculator_at("America/New_York", (2026, 10, 31, 12, 0, 0))
        ),
        "Nov 1, 2026 at 12:00:00"
    );
}

#[test]
fn overflow_and_unsupported_years_fail_for_every_relative_form() {
    let engine = calculator("UTC");
    for expression in [
        "9223372036854775808 days from now",
        "now + 9223372036854775808d",
        "in 999999999999999999999 seconds",
        "today + 315537897600 seconds",
        "9999-12-31 + 1d",
        "0001-01-01 - 1d",
    ] {
        let message = failure(expression, &engine);
        assert!(
            message.contains("supported range"),
            "{expression}: {message}"
        );
    }
}

#[test]
fn fractional_offsets_report_that_whole_amounts_are_required() {
    let engine = calculator("UTC");
    for expression in [
        "1.5 seconds from now",
        "today + 1.5 hours",
        "now + 1.5d",
        "in 1.5h",
    ] {
        let message = failure(expression, &engine);
        assert!(message.contains("whole number"), "{expression}: {message}");
    }
}

#[test]
fn date_ranges_and_duration_conversions_still_reach_their_own_calculators() {
    let engine = calculator("UTC");
    assert_eq!(display("2026-10-12 to 2026-10-13", &engine), "1 day");
    assert_eq!(display("3 days to hours", &engine), "72 h");
}
