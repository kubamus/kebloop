//! The launcher calculator: arithmetic, percentages, a few functions, plus inline date,
//! time-zone, currency and unit conversions (dispatched to [`crate::dates`], [`crate::currency`]
//! and [`crate::units`]).

use jiff::Timestamp;
use jiff::tz::TimeZone;
use thiserror::Error;

use crate::currency::{CurrencyConversionEngine, CurrencyPair, CurrencyRateSnapshot};
use crate::dates::DateCalculationEngine;
use crate::locale::Locale;
use crate::timezones::TimeZoneCalculationEngine;
use crate::units::UnitConversionEngine;

const MAXIMUM_INPUT_LENGTH: usize = 512;
const MAXIMUM_TOKEN_COUNT: usize = 256;
const MAXIMUM_NESTING_DEPTH: usize = 64;

/// What the calculator made of an input string.
#[derive(Debug, Clone, PartialEq)]
pub enum CalculationOutcome {
    /// The input is not a calculation (ordinary search text) or is empty.
    None,
    /// The input looks like an unfinished calculation; show nothing yet.
    Incomplete,
    /// A computed answer.
    Result(CalculationResult),
    /// A calculation that cannot be evaluated; the message is user-facing English.
    Failure(String),
}

/// A computed answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalculationResult {
    /// The trimmed input.
    pub expression: String,
    /// The text shown to the user (locale-formatted for conversions and dates).
    pub display: String,
    /// The text copied to the clipboard (equal to `display` except for currency conversions).
    pub clipboard: String,
}

/// Outcome of a specialized sub-engine ([`UnitConversionEngine`], [`DateCalculationEngine`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpecializedCalculation {
    /// The input is not this engine's business.
    NotHandled,
    /// The input is an unfinished expression of this engine.
    Incomplete,
    /// The formatted answer.
    Result(String),
    /// An answer whose clipboard text differs from the displayed text (a currency amount
    /// without its rate note).
    ResultWithClipboard {
        /// The text shown to the user.
        display: String,
        /// The text copied to the clipboard.
        clipboard: String,
    },
    /// A user-facing English error message.
    Failure(String),
}

/// Evaluates the small, self-contained calculator language used by the launcher.
#[derive(Debug, Clone)]
pub struct CalculatorEngine {
    locale: Locale,
    time_zone: TimeZone,
    reference_time: Option<Timestamp>,
}

impl Default for CalculatorEngine {
    /// `en_US` and the system time zone, using the real clock.
    fn default() -> Self {
        Self::new(Locale::default(), TimeZone::system())
    }
}

impl CalculatorEngine {
    /// Creates an engine that formats with `locale` and resolves dates in `time_zone`.
    pub fn new(locale: Locale, time_zone: TimeZone) -> Self {
        Self {
            locale,
            time_zone,
            reference_time: None,
        }
    }

    /// Fixes "now" (for `today`, `in 3 weeks`, ...). Without it the system clock is used.
    #[must_use]
    pub fn with_reference_time(mut self, reference_time: Timestamp) -> Self {
        self.reference_time = Some(reference_time);
        self
    }

    /// Evaluates `input` without any saved exchange rates (currency conversions then fail with
    /// "No saved exchange rate..."). Never panics; evaluation is bounded by input, token and
    /// nesting limits.
    pub fn evaluate(&self, input: &str) -> CalculationOutcome {
        self.evaluate_with_rates(input, &[])
    }

    /// Evaluates `input`, converting currencies with `currency_rates`.
    pub fn evaluate_with_rates(
        &self,
        input: &str,
        currency_rates: &[CurrencyRateSnapshot],
    ) -> CalculationOutcome {
        let expression = input.trim();
        if expression.is_empty() {
            return CalculationOutcome::None;
        }
        if expression.chars().count() > MAXIMUM_INPUT_LENGTH {
            return CalculationOutcome::Failure("Expression is too long.".into());
        }

        let now = self.reference_time.unwrap_or_else(Timestamp::now);
        let dates = DateCalculationEngine::new(self.locale.clone(), self.time_zone.clone());
        if let Some(outcome) = specialized(dates.evaluate(expression, now), expression) {
            return outcome;
        }
        let zones = TimeZoneCalculationEngine::new(self.locale.clone());
        if let Some(outcome) = specialized(zones.evaluate(expression, now), expression) {
            return outcome;
        }
        let currencies = CurrencyConversionEngine::new(self.locale.clone());
        let converted = currencies.evaluate(expression, currency_rates, now);
        if let Some(outcome) = specialized(converted, expression) {
            return outcome;
        }
        let units = UnitConversionEngine::new(self.locale.clone());
        if let Some(outcome) = specialized(units.evaluate(expression), expression) {
            return outcome;
        }

        if !looks_like_math(expression) {
            return CalculationOutcome::None;
        }
        match Parser::new(expression).and_then(|mut parser| parser.evaluate()) {
            Ok(value) => {
                let display = format_value(value);
                CalculationOutcome::Result(CalculationResult {
                    expression: expression.to_owned(),
                    clipboard: display.clone(),
                    display,
                })
            }
            Err(ParseIssue::Incomplete) => CalculationOutcome::Incomplete,
            Err(ParseIssue::Failure(message)) => CalculationOutcome::Failure(message),
        }
    }

    /// The currency pair of a conversion query, so the UI can refresh rates in the background
    /// without doing network work in the typing path.
    pub fn currency_pair(input: &str) -> Option<CurrencyPair> {
        crate::currency::pair_in(input)
    }
}

fn specialized(outcome: SpecializedCalculation, expression: &str) -> Option<CalculationOutcome> {
    match outcome {
        SpecializedCalculation::NotHandled => None,
        SpecializedCalculation::Incomplete => Some(CalculationOutcome::Incomplete),
        SpecializedCalculation::Result(display) => {
            Some(CalculationOutcome::Result(CalculationResult {
                expression: expression.to_owned(),
                clipboard: display.clone(),
                display,
            }))
        }
        SpecializedCalculation::ResultWithClipboard { display, clipboard } => {
            Some(CalculationOutcome::Result(CalculationResult {
                expression: expression.to_owned(),
                display,
                clipboard,
            }))
        }
        SpecializedCalculation::Failure(message) => Some(CalculationOutcome::Failure(message)),
    }
}

fn looks_like_math(input: &str) -> bool {
    const KNOWN_NAMES: [&str; 7] = ["pi", "e", "sqrt", "abs", "round", "of", "off"];
    let lowered = input.to_lowercase();
    let names: Vec<&str> = lowered
        .split(|c: char| !c.is_alphabetic())
        .filter(|name| !name.is_empty())
        .collect();
    let has_unknown_name = names.iter().any(|name| !KNOWN_NAMES.contains(name));
    let has_known_name = names.iter().any(|name| KNOWN_NAMES.contains(name));
    let has_number = input.chars().any(char::is_numeric);

    if !has_unknown_name {
        return has_number || has_known_name || input.chars().any(|c| "+-*/^%=().,".contains(c));
    }

    // Search queries often contain digits and punctuation (for example "1Password" or
    // "Visual Studio Code.app"). Treat mixed text as a calculation only when it also has
    // calculator syntax and a number or a recognized calculator name.
    let has_operator = input.chars().any(|c| "+-*/^%×÷−".contains(c));
    let has_parenthesis = input.chars().any(|c| "()".contains(c));
    (has_operator || has_parenthesis) && (has_number || has_known_name)
}

/// Formats like C's `%.16g` (what Swift's `String(format:)` produced), with zero as `0`.
fn format_value(value: f64) -> String {
    if value == 0.0 {
        return "0".into();
    }
    let scientific = format!("{value:.15e}");
    let Some((mantissa, exponent)) = scientific.split_once('e') else {
        return scientific;
    };
    let Ok(exponent) = exponent.parse::<i32>() else {
        return scientific;
    };
    if !(-4..16).contains(&exponent) {
        let mantissa = strip_trailing_zeros(mantissa);
        let sign = if exponent < 0 { '-' } else { '+' };
        return format!("{mantissa}e{sign}{:02}", exponent.abs());
    }
    let precision = usize::try_from(15 - exponent).unwrap_or(0);
    strip_trailing_zeros(&format!("{value:.precision$}")).to_owned()
}

fn strip_trailing_zeros(text: &str) -> &str {
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.')
    } else {
        text
    }
}

#[derive(Debug, Error)]
enum ParseIssue {
    #[error("the expression is incomplete")]
    Incomplete,
    #[error("{0}")]
    Failure(String),
}

fn fail<T>(message: impl Into<String>) -> Result<T, ParseIssue> {
    Err(ParseIssue::Failure(message.into()))
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Number(f64),
    Identifier(String),
    Plus,
    Minus,
    Multiply,
    Divide,
    Power,
    Percent,
    LeftParen,
    RightParen,
    End,
}

#[derive(Debug, Clone, Copy)]
struct Evaluated {
    value: f64,
    /// Retained only for a direct percentage value, so percentage phrases can use it.
    is_percentage: bool,
}

impl Evaluated {
    fn plain(value: f64) -> Self {
        Self {
            value,
            is_percentage: false,
        }
    }
}

struct Parser {
    tokens: Vec<Token>,
    index: usize,
    nesting_depth: usize,
}

impl Parser {
    fn new(source: &str) -> Result<Self, ParseIssue> {
        Ok(Self {
            tokens: tokenize(source)?,
            index: 0,
            nesting_depth: 0,
        })
    }

    fn evaluate(&mut self) -> Result<f64, ParseIssue> {
        let result = self.parse_expression()?;
        match self.current() {
            Token::End => Ok(result.value),
            Token::RightParen => fail("There is an unmatched closing parenthesis."),
            Token::Identifier(name) => fail(format!("Unexpected “{name}” after the expression.")),
            _ => fail("There is unexpected content after the expression."),
        }
    }

    fn current(&self) -> Token {
        self.tokens.get(self.index).cloned().unwrap_or(Token::End)
    }

    fn advance(&mut self) {
        if self.index + 1 < self.tokens.len() {
            self.index += 1;
        }
    }

    fn parse_expression(&mut self) -> Result<Evaluated, ParseIssue> {
        let mut lhs = self.parse_term()?;
        loop {
            let operation = self.current();
            if !matches!(operation, Token::Plus | Token::Minus) {
                return Ok(lhs);
            }
            self.advance();
            let rhs = self.parse_term()?;
            if operation == Token::Plus {
                lhs.value += rhs.value;
            } else {
                lhs.value -= rhs.value;
            }
            lhs.is_percentage = false;
            require_finite(lhs.value)?;
        }
    }

    fn parse_term(&mut self) -> Result<Evaluated, ParseIssue> {
        let mut lhs = self.parse_unary()?;
        loop {
            match self.current() {
                Token::Multiply => {
                    self.advance();
                    let rhs = self.parse_unary()?;
                    lhs = Evaluated::plain(lhs.value * rhs.value);
                }
                Token::Divide => {
                    self.advance();
                    let rhs = self.parse_unary()?;
                    if rhs.value == 0.0 {
                        return fail("Cannot divide by zero.");
                    }
                    lhs = Evaluated::plain(lhs.value / rhs.value);
                }
                Token::Identifier(name) if name == "of" => {
                    if !lhs.is_percentage {
                        return fail("Use a percentage before “of”.");
                    }
                    self.advance();
                    let rhs = self.parse_unary()?;
                    lhs = Evaluated::plain(lhs.value * rhs.value);
                }
                Token::Identifier(name) if name == "off" => {
                    if !lhs.is_percentage {
                        return fail("Use a percentage before “off”.");
                    }
                    self.advance();
                    let base = self.parse_unary()?;
                    lhs = Evaluated::plain(base.value * (1.0 - lhs.value));
                }
                _ => {
                    require_finite(lhs.value)?;
                    return Ok(lhs);
                }
            }
            lhs.is_percentage = false;
            require_finite(lhs.value)?;
        }
    }

    /// Unary signs bind less tightly than exponentiation, while the exponent may itself be signed.
    fn parse_unary(&mut self) -> Result<Evaluated, ParseIssue> {
        match self.current() {
            Token::Plus => {
                self.advance();
                self.parse_unary()
            }
            Token::Minus => {
                self.advance();
                let mut value = self.parse_unary()?;
                value.value = -value.value;
                require_finite(value.value)?;
                Ok(value)
            }
            _ => self.parse_power(),
        }
    }

    /// Recursing through the right operand makes exponentiation right-associative.
    fn parse_power(&mut self) -> Result<Evaluated, ParseIssue> {
        let base = self.parse_postfix()?;
        if self.current() != Token::Power {
            return Ok(base);
        }
        self.advance();
        let exponent = self.parse_unary()?;
        let value = base.value.powf(exponent.value);
        require_finite(value)?;
        Ok(Evaluated::plain(value))
    }

    fn parse_postfix(&mut self) -> Result<Evaluated, ParseIssue> {
        let mut value = self.parse_primary()?;
        while self.current() == Token::Percent {
            self.advance();
            value.value /= 100.0;
            value.is_percentage = true;
        }
        require_finite(value.value)?;
        Ok(value)
    }

    fn parse_primary(&mut self) -> Result<Evaluated, ParseIssue> {
        match self.current() {
            Token::Number(value) => {
                self.advance();
                Ok(Evaluated::plain(value))
            }
            Token::LeftParen => {
                self.enter_nested()?;
                self.advance();
                let value = self.parse_expression()?;
                self.consume_closing_parenthesis()?;
                self.nesting_depth -= 1;
                Ok(value)
            }
            Token::Identifier(name) => {
                self.advance();
                match name.as_str() {
                    "pi" => Ok(Evaluated::plain(std::f64::consts::PI)),
                    "e" => Ok(Evaluated::plain(1.0_f64.exp())),
                    "sqrt" | "abs" | "round" => self.parse_function(&name),
                    _ => fail(format!("Unknown name “{name}”.")),
                }
            }
            Token::End | Token::RightParen => Err(ParseIssue::Incomplete),
            _ => fail("Expected a number or a parenthesized expression."),
        }
    }

    fn parse_function(&mut self, name: &str) -> Result<Evaluated, ParseIssue> {
        if self.current() != Token::LeftParen {
            if self.current() == Token::End {
                return Err(ParseIssue::Incomplete);
            }
            return fail(format!(
                "Write “{name}” with parentheses, such as {name}(2)."
            ));
        }
        self.enter_nested()?;
        self.advance();
        let argument = self.parse_expression()?.value;
        self.consume_closing_parenthesis()?;
        self.nesting_depth -= 1;
        let result = match name {
            "sqrt" => {
                if argument < 0.0 {
                    return fail("Square root needs a non-negative value.");
                }
                argument.sqrt()
            }
            "abs" => argument.abs(),
            _ => argument.round(),
        };
        require_finite(result)?;
        Ok(Evaluated::plain(result))
    }

    fn enter_nested(&mut self) -> Result<(), ParseIssue> {
        self.nesting_depth += 1;
        if self.nesting_depth > MAXIMUM_NESTING_DEPTH {
            return fail("Expression is nested too deeply.");
        }
        Ok(())
    }

    fn consume_closing_parenthesis(&mut self) -> Result<(), ParseIssue> {
        match self.current() {
            Token::RightParen => {
                self.advance();
                Ok(())
            }
            Token::End => Err(ParseIssue::Incomplete),
            _ => fail("Expected a closing parenthesis."),
        }
    }
}

fn require_finite(value: f64) -> Result<(), ParseIssue> {
    if value.is_finite() {
        Ok(())
    } else {
        fail("The result is outside the supported numeric range.")
    }
}

fn tokenize(source: &str) -> Result<Vec<Token>, ParseIssue> {
    let characters: Vec<char> = source.to_lowercase().chars().collect();
    let mut output: Vec<Token> = Vec::new();
    let mut cursor = 0;

    fn push(output: &mut Vec<Token>, token: Token) -> Result<(), ParseIssue> {
        if output.len() >= MAXIMUM_TOKEN_COUNT {
            return fail("Expression has too many parts.");
        }
        output.push(token);
        Ok(())
    }

    while let Some(&character) = characters.get(cursor) {
        if character.is_whitespace() {
            cursor += 1;
            continue;
        }

        if character.is_numeric() || character == '.' || character == ',' {
            let start = cursor;
            let mut separator_seen = false;
            let mut digit_seen = false;
            while let Some(&next) = characters.get(cursor) {
                if next.is_numeric() {
                    digit_seen = true;
                } else if (next == '.' || next == ',') && !separator_seen {
                    separator_seen = true;
                } else {
                    break;
                }
                cursor += 1;
            }
            if !digit_seen {
                return fail("Expected digits in the number.");
            }

            if characters.get(cursor) == Some(&'e') {
                cursor += 1;
                if matches!(characters.get(cursor), Some('+' | '-')) {
                    cursor += 1;
                }
                let exponent_start = cursor;
                while characters.get(cursor).is_some_and(|c| c.is_numeric()) {
                    cursor += 1;
                }
                if cursor == exponent_start {
                    return Err(ParseIssue::Incomplete);
                }
            }

            let literal: String = characters[start..cursor]
                .iter()
                .map(|&c| if c == ',' { '.' } else { c })
                .collect();
            match literal.parse::<f64>() {
                Ok(number) if number.is_finite() => push(&mut output, Token::Number(number))?,
                _ => return fail("That number is outside the supported range."),
            }
            continue;
        }

        if character.is_alphabetic() {
            let start = cursor;
            while characters.get(cursor).is_some_and(|c| c.is_alphabetic()) {
                cursor += 1;
            }
            let name: String = characters[start..cursor].iter().collect();
            push(&mut output, Token::Identifier(name))?;
            continue;
        }

        let token = match character {
            '+' => Token::Plus,
            '-' | '−' => Token::Minus,
            '*' | '×' => Token::Multiply,
            '/' | '÷' => Token::Divide,
            '^' => Token::Power,
            '%' => Token::Percent,
            '(' => Token::LeftParen,
            ')' => Token::RightParen,
            other => return fail(format!("Unsupported character “{other}”.")),
        };
        push(&mut output, token)?;
        cursor += 1;
    }

    output.push(Token::End);
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn g16_formatting_matches_printf() {
        assert_eq!(format_value(0.1 + 0.2), "0.3");
        assert_eq!(format_value(1e20), "1e+20");
        assert_eq!(format_value(1.5e-7), "1.5e-07");
        assert_eq!(format_value(123456789012345.0), "123456789012345");
        assert_eq!(format_value(1e15), "1000000000000000");
        assert_eq!(format_value(1e16), "1e+16");
        assert_eq!(format_value(-0.0001), "-0.0001");
        assert_eq!(format_value(0.00001), "1e-05");
        assert_eq!(format_value(-0.0), "0");
    }
}
