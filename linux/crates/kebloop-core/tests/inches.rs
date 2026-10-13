//! Port of `Tests/KebloopCoreTests/InchConversionTests.swift`.
//!
//! Upstream fixed the trailing `in` of `1 m to in` being read as an unfinished separator.
//! Our earlier port had its own guard for the same bug; the upstream logic (a trailing
//! separator is "incomplete" only after a known unit or a finished mixed sum) now replaces it.

use jiff::tz::TimeZone;
use kebloop_core::{CalculationOutcome, CalculatorEngine, Locale};

fn assert_result(expression: &str, expected: &str) {
    let calculator = CalculatorEngine::new(Locale::en_us(), TimeZone::UTC);
    match calculator.evaluate(expression) {
        CalculationOutcome::Result(result) => assert_eq!(result.display, expected, "{expression}"),
        other => panic!("expected a conversion result for {expression}, got {other:?}"),
    }
}

#[test]
fn conversions_can_target_inches_using_symbols_and_names() {
    assert_result("1 cm to in", "0.393700787402 in");
    assert_result("1 cm to inch", "0.393700787402 in");
    assert_result("1 cm to inches", "0.393700787402 in");
    assert_result("1 cm to inchs", "0.393700787402 in");
    assert_result("1 m in inches", "39.3700787402 in");
    assert_result("1 m in in", "39.3700787402 in");
    assert_result("1 in to in", "1 in");
}

#[test]
fn inches_work_as_source_and_mixed_arithmetic_target() {
    assert_result("12 inches to cm", "30.48 cm");
    assert_result("2 m + 30 cm to in", "90.5511811024 in");
}

#[test]
fn scientific_notation_still_converts_to_inches() {
    assert_result("1e-2 m to in", "0.393700787402 in");
    assert_result("1e-3 m to in", "0.0393700787402 in");
    assert_result("1e-3 m + 1 cm to in", "0.433070866142 in");
}

#[test]
fn a_trailing_separator_after_an_unknown_unit_is_not_a_conversion() {
    let calculator = CalculatorEngine::new(Locale::en_us(), TimeZone::UTC);
    assert_eq!(calculator.evaluate("1 fathom to"), CalculationOutcome::None);
    assert_eq!(
        calculator.evaluate("1 m to"),
        CalculationOutcome::Incomplete
    );
    assert_eq!(
        calculator.evaluate("1 m in"),
        CalculationOutcome::Incomplete
    );
}
