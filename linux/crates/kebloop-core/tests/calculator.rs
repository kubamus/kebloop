//! Port of `Tests/KebloopCoreTests/CalculatorTests.swift`, plus Linux locale coverage.

use jiff::civil;
use jiff::tz::TimeZone;
use kebloop_core::{
    CalculationOutcome, CalculatorEngine, CurrencyPair, CurrencyRateSnapshot, Locale,
};

fn utc_calculator() -> CalculatorEngine {
    CalculatorEngine::new(Locale::en_us(), TimeZone::UTC)
}

fn zone(name: &str) -> TimeZone {
    TimeZone::get(name).unwrap_or_else(|error| panic!("missing test time zone {name}: {error}"))
}

fn result_with(calculator: &CalculatorEngine, expression: &str) -> String {
    match calculator.evaluate(expression) {
        CalculationOutcome::Result(result) => result.display,
        other => panic!("expected a result for {expression:?}, got {other:?}"),
    }
}

fn result(expression: &str) -> String {
    result_with(&utc_calculator(), expression)
}

fn assert_failure(calculator: &CalculatorEngine, expression: &str) {
    assert!(
        matches!(
            calculator.evaluate(expression),
            CalculationOutcome::Failure(_)
        ),
        "expected a failure for {:?}, got {:?}",
        expression.chars().take(40).collect::<String>(),
        calculator.evaluate(expression)
    );
}

#[test]
fn operator_precedence_and_associativity() {
    assert_eq!(result("-2^2"), "-4");
    assert_eq!(result("2^3^2"), "512");
    assert_eq!(result("2^-2"), "0.25");
    assert_eq!(result("(2 + 3) * 4"), "20");
}

#[test]
fn decimal_separators_scientific_notation_and_constants() {
    assert_eq!(result("1,5 + 2.25 + 1e2"), "103.75");
    assert_eq!(result("pi * 2"), "6.283185307179586");
    assert_eq!(result("e"), "2.718281828459045");
}

#[test]
fn functions_and_percentage_expressions() {
    assert_eq!(result("sqrt(81) + abs(-2)"), "11");
    assert_eq!(result("round(2.6)"), "3");
    assert_eq!(result("15% of 200"), "30");
    assert_eq!(result("20% off 80"), "64");
    assert_eq!(result("200 + 10%"), "200.1");
    assert_eq!(result("80 - 20%"), "79.8");
    assert_eq!(result("15%"), "0.15");
}

#[test]
fn unit_conversions_cover_aliases_dimensions_and_temperature_offsets() {
    assert_eq!(result("1 kilometer to miles"), "0.621371192237 mi");
    assert_eq!(result("12 in to centimeters"), "30.48 cm");
    assert_eq!(result("2 pounds to kg"), "0.90718474 kg");
    assert_eq!(result("5 kilometers in miles"), "3.10685596119 mi");
    assert_eq!(result("32 °F to °C"), "0 °C");
    assert_eq!(result("0 celsius to fahrenheit"), "32 °F");
    assert_eq!(result("1 MB to MiB"), "0.953674316406 MiB");
    assert_eq!(result("1 MiB to bytes"), "1048576 B");
    assert_eq!(result("1 MB as MiB"), "0.953674316406 MiB");
    assert_eq!(result("8 bits to bytes"), "1 B");
    assert_eq!(result("1 byte to bits"), "8 b");

    let calculator = utc_calculator();
    assert_failure(&calculator, "1 meter to kilograms"); // incompatible dimensions
    assert_failure(&calculator, "1 fathom to meters"); // unknown unit
    assert_failure(&calculator, "-300 C to K"); // below absolute zero
}

#[test]
fn inches_can_be_a_conversion_target() {
    // Regression for a Swift quirk: the trailing "in" was read as an unfinished separator.
    assert_eq!(result("12 cm to in"), "4.72440944882 in");
    assert_eq!(result("1 ft to in"), "12 in");
}

fn instant(text: &str) -> jiff::Timestamp {
    text.parse()
        .unwrap_or_else(|error| panic!("invalid test timestamp {text}: {error}"))
}

#[test]
fn currency_conversions_use_cached_rates_and_show_their_observation_date() {
    let now = instant("2026-10-13T12:00:00Z");
    let observation = instant("2026-10-12T00:00:00Z");
    let pair = CurrencyPair::new("USD", "EUR").expect("supported pair");
    let current = CurrencyRateSnapshot::new(pair.clone(), 0.92, observation, now);
    let calculator = utc_calculator().with_reference_time(now);

    let CalculationOutcome::Result(calculation) =
        calculator.evaluate_with_rates("$100 to EUR", &[current])
    else {
        panic!("expected a currency conversion from a cached rate");
    };
    assert_eq!(calculation.display, "92.00 EUR (rate dated Oct 12, 2026)");
    assert_eq!(calculation.clipboard, "92.00 EUR");
    assert_eq!(
        CalculatorEngine::currency_pair("100 USD in euros"),
        Some(pair.clone())
    );
    assert_eq!(CalculatorEngine::currency_pair("search for USD"), None);

    let stale = CurrencyRateSnapshot::new(
        pair.clone(),
        0.92,
        observation,
        instant("2026-10-12T11:00:00Z"),
    );
    let CalculationOutcome::Result(stale_result) =
        calculator.evaluate_with_rates("100 USD to EUR", &[stale])
    else {
        panic!("a saved rate should still be usable offline");
    };
    assert!(stale_result.display.contains("cached"), "{stale_result:?}");
    assert_eq!(stale_result.clipboard, "92.00 EUR");

    let CalculationOutcome::Failure(message) = calculator.evaluate("100 USD to EUR") else {
        panic!("a missing pair should report that a rate is unavailable");
    };
    assert!(message.contains("No saved exchange rate"), "{message}");
    assert_eq!(result_with(&calculator, "10 GBP to GBP"), "10.00 GBP");
}

#[test]
fn currency_conversions_accept_the_inverse_rate_and_name_unknown_currencies() {
    let now = instant("2026-10-13T12:00:00Z");
    let observation = instant("2026-10-12T00:00:00Z");
    let usd_pln = CurrencyPair::new("USD", "PLN").expect("supported pair");
    let rates = [CurrencyRateSnapshot::new(usd_pln, 4.0, observation, now)];
    let calculator = utc_calculator().with_reference_time(now);
    let CalculationOutcome::Result(result) =
        calculator.evaluate_with_rates("100 zł to usd", &rates)
    else {
        panic!("the inverse of a saved rate converts");
    };
    assert_eq!(result.display, "25.00 USD (rate dated Oct 12, 2026)");

    let polish = CalculatorEngine::new(Locale::pl(), TimeZone::UTC).with_reference_time(now);
    let CalculationOutcome::Result(result) = polish.evaluate_with_rates("$1,5 to PLN", &rates)
    else {
        panic!("decimal comma amounts convert");
    };
    assert_eq!(result.clipboard, "6,00 PLN");
    assert!(result.display.contains("12 paź 2026"), "{}", result.display);

    assert_failure(&calculator, "100 USD to Narnia");
    // A real unit target is not a currency query.
    assert_eq!(result_with(&calculator, "100 m to km"), "0.1 km");
    assert_eq!(
        calculator.evaluate("100 USD to"),
        CalculationOutcome::Incomplete
    );
}

#[test]
fn duration_conversions_and_mixed_unit_arithmetic() {
    assert_eq!(result("90 minutes to hours"), "1.5 h");
    assert_eq!(result("2 weeks in seconds"), "1209600 s");
    assert_eq!(result("2 km + 300 m"), "2.3 km");
    assert_eq!(result("2 m + 30 cm to cm"), "230 cm");
    assert_eq!(result("1 hour + 30 minutes"), "1.5 h");
    assert_eq!(result("3 ft - 4 in"), "2.66666666667 ft");

    let calculator = utc_calculator();
    assert_failure(&calculator, "2 m + 3 seconds"); // mixed dimensions
    assert_failure(&calculator, "20 °C + 5 °F"); // absolute temperatures
}

#[test]
fn named_time_zone_lookups_and_conversions_use_current_date_rules() {
    let now = instant("2026-10-13T12:00:00Z");
    let calculator = utc_calculator().with_reference_time(now);

    let tokyo = result_with(&calculator, "time in Tokyo");
    assert!(tokyo.contains("21:00"), "{tokyo}");
    assert!(tokyo.contains("GMT+09:00"), "{tokyo}");

    let london = result_with(&calculator, "9:30 AM in New York to London");
    assert!(london.contains("14:30"), "{london}");
    assert!(london.contains("GMT+01:00"), "{london}");

    let daylight_saving = result_with(&calculator, "9 AM PST to CET");
    assert!(daylight_saving.contains("18:00"), "{daylight_saving}");
    assert!(daylight_saving.contains("GMT+02:00"), "{daylight_saving}");

    assert_eq!(
        calculator.evaluate("time in"),
        CalculationOutcome::Incomplete
    );
    assert_failure(&calculator, "10:00 AM in Nowhere to London");
    // A bare number with a unit is a unit conversion, not a clock time.
    assert_eq!(result_with(&calculator, "12 km to mi"), "7.45645430685 mi");
}

#[test]
fn every_documented_unit_alias_is_recognized() {
    let length = [
        "m",
        "meter",
        "meters",
        "metre",
        "metres",
        "km",
        "kilometer",
        "kilometers",
        "kilometre",
        "kilometres",
        "cm",
        "centimeter",
        "centimeters",
        "centimetre",
        "centimetres",
        "mm",
        "millimeter",
        "millimeters",
        "millimetre",
        "millimetres",
        "µm",
        "um",
        "μm",
        "micrometer",
        "micrometers",
        "micrometre",
        "micrometres",
        "micron",
        "microns",
        "nm",
        "nanometer",
        "nanometers",
        "nanometre",
        "nanometres",
        "mi",
        "mile",
        "miles",
        "yd",
        "yard",
        "yards",
        "ft",
        "foot",
        "feet",
        "in",
        "inch",
        "inches",
        "inchs",
    ];
    let mass = [
        "g",
        "gram",
        "grams",
        "kg",
        "kilogram",
        "kilograms",
        "mg",
        "milligram",
        "milligrams",
        "µg",
        "μg",
        "ug",
        "mcg",
        "microgram",
        "micrograms",
        "lb",
        "lbs",
        "pound",
        "pounds",
        "oz",
        "ounce",
        "ounces",
        "st",
        "stone",
        "stones",
    ];
    let temperature = [
        "C",
        "°C",
        "ºC",
        "celsius",
        "centigrade",
        "degree C",
        "degrees C",
        "degree celsius",
        "degrees celsius",
        "F",
        "°F",
        "ºF",
        "fahrenheit",
        "degree F",
        "degrees F",
        "degree fahrenheit",
        "degrees fahrenheit",
        "K",
        "kelvin",
        "kelvins",
        "degree K",
        "degrees K",
    ];
    let data = [
        "b",
        "B",
        "kb",
        "kB",
        "Mb",
        "MB",
        "Gb",
        "GB",
        "Tb",
        "TB",
        "Kib",
        "KiB",
        "Mib",
        "MiB",
        "Gib",
        "GiB",
        "Tib",
        "TiB",
        "bit",
        "bits",
        "byte",
        "bytes",
        "kilobit",
        "kilobits",
        "kilobyte",
        "kilobytes",
        "megabit",
        "megabits",
        "megabyte",
        "megabytes",
        "gigabit",
        "gigabits",
        "gigabyte",
        "gigabytes",
        "terabit",
        "terabits",
        "terabyte",
        "terabytes",
        "kibibit",
        "kibibits",
        "kibibyte",
        "kibibytes",
        "mebibit",
        "mebibits",
        "mebibyte",
        "mebibytes",
        "gibibit",
        "gibibits",
        "gibibyte",
        "gibibytes",
        "tebibit",
        "tebibits",
        "tebibyte",
        "tebibytes",
    ];
    let duration = [
        "ms",
        "millisecond",
        "milliseconds",
        "s",
        "sec",
        "second",
        "seconds",
        "min",
        "minute",
        "minutes",
        "h",
        "hr",
        "hrs",
        "hour",
        "hours",
        "d",
        "day",
        "days",
        "wk",
        "wks",
        "week",
        "weeks",
    ];
    let groups: [(&[&str], &str); 5] = [
        (&length, "m"),
        (&mass, "g"),
        (&temperature, "K"),
        (&data, "B"),
        (&duration, "s"),
    ];

    let calculator = utc_calculator();
    let mut checked = 0;
    for (aliases, target) in groups {
        for alias in aliases {
            let display = match calculator.evaluate(&format!("1 {alias} to {target}")) {
                CalculationOutcome::Result(result) => result.display,
                other => panic!("documented unit alias was not recognized: {alias} ({other:?})"),
            };
            assert!(
                display.ends_with(&format!(" {target}")),
                "unexpected target for unit alias {alias}: {display}"
            );
            checked += 1;
        }
    }
    assert_eq!(
        checked,
        length.len() + mass.len() + temperature.len() + data.len() + duration.len()
    );
}

#[test]
fn conversions_accept_decimal_comma_and_format_for_locale() {
    let german = CalculatorEngine::new(Locale::from_tag("de_DE"), TimeZone::UTC);
    let CalculationOutcome::Result(result) = german.evaluate("1,5 m to ft") else {
        panic!("a decimal comma should be accepted");
    };
    assert_eq!(result.display, "4,92125984252 ft");
    assert_eq!(result.clipboard, result.display);

    let polish = CalculatorEngine::new(Locale::pl(), TimeZone::UTC);
    assert_eq!(result_with(&polish, "1.5 m to ft"), "4,92125984252 ft");
    assert_eq!(
        result_with(&utc_calculator(), "1,5 m to ft"),
        "4.92125984252 ft"
    );
}

#[test]
fn date_differences_additions_locale_ambiguity_and_daylight_saving() {
    assert_eq!(result("days between 2026-03-07 and 2026-03-09"), "2 days");
    assert_eq!(
        result("date difference between 2026-03-07 and 2026-03-09"),
        "2 days"
    );
    assert_eq!(result("2026-03-07 to 2026-03-09"), "2 days");
    assert_eq!(result("days between 2026-03-09 and 2026-03-07"), "-2 days");
    assert_eq!(result("2026-01-31 + 1 month"), "Feb 28, 2026");
    assert_eq!(result("2026-01-01 + 1 week"), "Jan 8, 2026");
    assert_eq!(result("2026-01-01 - 1 year"), "Jan 1, 2025");

    let us = CalculatorEngine::new(Locale::en_us(), zone("UTC"));
    let uk = CalculatorEngine::new(Locale::en_gb(), zone("UTC"));
    assert_eq!(result_with(&us, "01/02/2026 + 1 day"), "Jan 3, 2026");
    assert_eq!(result_with(&uk, "01/02/2026 + 1 day"), "2 Feb 2026");

    let new_york = zone("America/New_York");
    let reference = civil::date(2026, 1, 15)
        .at(12, 0, 0, 0)
        .to_zoned(new_york.clone())
        .expect("valid reference time")
        .timestamp();
    let ny = CalculatorEngine::new(Locale::en_us(), new_york).with_reference_time(reference);
    assert_eq!(result_with(&ny, "today + 10 days"), "Jan 25, 2026");
    assert_eq!(result_with(&ny, "in 3 weeks"), "Feb 5, 2026");
    assert_eq!(result_with(&ny, "3 days from today"), "Jan 18, 2026");
    let day_after_dst = result_with(&ny, "2026-03-07 12:00 + 1 day");
    assert!(day_after_dst.starts_with("Mar 8, 2026"), "{day_after_dst}");
    assert!(day_after_dst.contains("12:00"), "{day_after_dst}");
    assert!(
        !(day_after_dst.ends_with("AM") || day_after_dst.ends_with("PM")),
        "times use a 24-hour clock: {day_after_dst}"
    );
    assert_eq!(
        result_with(
            &ny,
            "days between 2026-03-08T00:00:00-05:00 and 2026-03-09T00:00:00-04:00"
        ),
        "1 day"
    );
    assert_eq!(
        result_with(
            &ny,
            "days between 2026-03-08T00:00:00Z and 2026-03-09T00:00:00Z"
        ),
        "1 day"
    );
}

#[test]
fn dates_follow_the_injected_locale() {
    let reference = "2026-10-12T10:00:00Z".parse().expect("valid timestamp");
    let polish =
        CalculatorEngine::new(Locale::pl(), zone("Europe/Warsaw")).with_reference_time(reference);
    assert_eq!(result_with(&polish, "2026-01-31 + 1 month"), "28 lut 2026");
    assert_eq!(result_with(&polish, "01.02.2026 + 1 day"), "2 lut 2026");
    assert_eq!(result_with(&polish, "tomorrow + 0 days"), "13 paź 2026");
    assert_eq!(
        result_with(&polish, "2026-03-07 12:00 + 1 day"),
        "8 mar 2026, 12:00"
    );
    let english = CalculatorEngine::new(Locale::en_us(), zone("Europe/Warsaw"))
        .with_reference_time(reference);
    assert_eq!(result_with(&english, "yesterday + 1 day"), "Oct 12, 2026");
    // Differences are counted on the local calendar, not in elapsed hours.
    assert_eq!(result_with(&english, "today to 2026-12-25"), "74 days");
}

#[test]
fn incomplete_conversions_and_date_expressions_stay_quiet() {
    let calculator = utc_calculator();
    for expression in [
        "2 km to",
        "2 m + 30 cm to",
        "today +",
        "days between 2026-10-12 and",
        "2026-10-12 to",
        "1 m to in",
    ] {
        let outcome = calculator.evaluate(expression);
        if expression == "1 m to in" {
            assert!(
                matches!(outcome, CalculationOutcome::Result(_)),
                "{expression}: {outcome:?}"
            );
        } else {
            assert_eq!(outcome, CalculationOutcome::Incomplete, "{expression}");
        }
    }
    assert_eq!(
        calculator.evaluate("search for 2026"),
        CalculationOutcome::None
    );
}

#[test]
fn results_are_useful_for_display_and_clipboard() {
    let CalculationOutcome::Result(calculation) = utc_calculator().evaluate(" 0.1 + 0.2 ") else {
        panic!("expected a calculated result");
    };
    assert_eq!(calculation.expression, "0.1 + 0.2");
    assert_eq!(calculation.display, "0.3");
    assert_eq!(calculation.clipboard, "0.3");
}

#[test]
fn incomplete_and_non_math_inputs_stay_quiet() {
    let calculator = utc_calculator();
    assert_eq!(calculator.evaluate("2 +"), CalculationOutcome::Incomplete);
    assert_eq!(calculator.evaluate("sqrt("), CalculationOutcome::Incomplete);
    assert_eq!(calculator.evaluate("1e"), CalculationOutcome::Incomplete);
    assert_eq!(calculator.evaluate("mail"), CalculationOutcome::None);
    assert_eq!(calculator.evaluate("   "), CalculationOutcome::None);
    for query in ["1Password", "Visual Studio Code.app", "PowerPoint 365"] {
        assert_eq!(
            calculator.evaluate(query),
            CalculationOutcome::None,
            "{query}"
        );
    }
}

#[test]
fn invalid_expressions_report_failures() {
    let calculator = utc_calculator();
    for expression in ["2 / 0", "sqrt(-1)", "2 + * 3", "1..2", "unknown(2)"] {
        assert_failure(&calculator, expression);
    }
}

#[test]
fn input_and_parser_limits_reject_excessive_work_and_overflow() {
    let too_long = "9".repeat(513);
    let too_many_tokens = "1+".repeat(256);
    let too_deep = format!("{}1{}", "(".repeat(65), ")".repeat(65));
    let calculator = utc_calculator();
    for expression in [
        too_long.as_str(),
        &too_many_tokens,
        &too_deep,
        "1e309",
        "10^10000",
        "0^-1",
    ] {
        assert_failure(&calculator, expression);
    }
}
