//! Unit conversions such as `12 km to mi` or `32 °F to °C`.
//!
//! Supported dimensions: length, mass, duration, temperature and data (bits/bytes, decimal
//! and binary prefixes), plus mixed-unit addition and subtraction of one dimension
//! (`2 km + 300 m`). See `docs/conversions.md` for the accepted names.

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::calculator::SpecializedCalculation;
use crate::locale::Locale;

const SIGNIFICANT_DIGITS: usize = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dimension {
    Length,
    Mass,
    Temperature,
    Data,
    Duration,
}

#[derive(Debug, Clone, Copy)]
struct UnitDefinition {
    symbol: &'static str,
    dimension: Dimension,
    scale: f64,
    offset: f64,
}

const fn unit(symbol: &'static str, dimension: Dimension, scale: f64) -> UnitDefinition {
    UnitDefinition {
        symbol,
        dimension,
        scale,
        offset: 0.0,
    }
}

/// Converts quantities written as `<number> <unit> to|in|as <unit>`.
#[derive(Debug, Clone, Default)]
pub struct UnitConversionEngine {
    locale: Locale,
}

impl UnitConversionEngine {
    /// Creates an engine that formats results with `locale`'s decimal mark.
    pub fn new(locale: Locale) -> Self {
        Self { locale }
    }

    /// `true` when `name` is a known unit name or symbol (used by the currency engine to
    /// leave `12 km to mi`-style text to this engine).
    pub(crate) fn recognizes_unit(&self, name: &str) -> bool {
        lookup_unit(name).is_some()
    }

    /// Evaluates a conversion. Returns [`SpecializedCalculation::NotHandled`] for any text
    /// that is not a conversion.
    pub fn evaluate(&self, input: &str) -> SpecializedCalculation {
        let separators = find_separators(input);
        for &(start, end) in separators.iter().rev() {
            let prefix = input[..start].trim();
            let target_text = input[end..].trim();
            if target_text.is_empty() {
                // "1 m to in": a trailing "in" that is not preceded by a unit is the unit.
                if split_arithmetic(prefix).is_some()
                    && matches!(
                        self.evaluate_mixed_arithmetic(prefix, None),
                        SpecializedCalculation::Result(_)
                    )
                {
                    return SpecializedCalculation::Incomplete;
                }
                if separators.len() == 1 && split_arithmetic(input).is_some() {
                    return self.evaluate_mixed_arithmetic(input, None);
                }
                if let Some((_, unit)) = parse_quantity(prefix)
                    && lookup_unit(unit).is_some()
                {
                    return SpecializedCalculation::Incomplete;
                }
                continue;
            }
            if split_arithmetic(prefix).is_some() {
                return self.evaluate_mixed_arithmetic(prefix, Some(target_text));
            }
            let Some((value, source_unit)) = parse_quantity(prefix) else {
                continue;
            };

            let Some(source) = lookup_unit(source_unit) else {
                return SpecializedCalculation::Failure(format!("Unknown unit “{source_unit}”."));
            };
            let Some(target) = lookup_unit(target_text) else {
                return SpecializedCalculation::Failure(format!("Unknown unit “{target_text}”."));
            };
            if source.dimension != target.dimension {
                return SpecializedCalculation::Failure(
                    "Those units measure different things.".into(),
                );
            }

            let base_value = value * source.scale + source.offset;
            if !base_value.is_finite() {
                return SpecializedCalculation::Failure(
                    "The converted value is outside the supported range.".into(),
                );
            }
            if source.dimension == Dimension::Temperature && base_value < -0.000_000_1 {
                return SpecializedCalculation::Failure(
                    "That temperature is below absolute zero.".into(),
                );
            }
            let converted = (base_value - target.offset) / target.scale;
            if !converted.is_finite() {
                return SpecializedCalculation::Failure(
                    "The converted value is outside the supported range.".into(),
                );
            }
            return SpecializedCalculation::Result(format!(
                "{} {}",
                format_number(converted, self.locale.decimal_separator()),
                target.symbol
            ));
        }
        self.evaluate_mixed_arithmetic(input, None)
    }

    /// `2 km + 300 m`, optionally converted to `target_unit_name`; the result uses the first
    /// term's unit otherwise.
    fn evaluate_mixed_arithmetic(
        &self,
        input: &str,
        target_unit_name: Option<&str>,
    ) -> SpecializedCalculation {
        let failure = |message: &str| SpecializedCalculation::Failure(message.to_owned());
        let Some((terms, operators)) = split_arithmetic(input) else {
            return SpecializedCalculation::NotHandled;
        };
        let Some((first_value, first_unit_name)) = terms.first().and_then(|t| parse_quantity(t))
        else {
            return SpecializedCalculation::NotHandled;
        };
        let Some(first_unit) = lookup_unit(first_unit_name) else {
            return SpecializedCalculation::NotHandled;
        };
        if first_unit.dimension == Dimension::Temperature {
            return failure("Adding or subtracting absolute temperatures is not supported.");
        }

        let out_of_range = "The result is outside the supported numeric range.";
        let mut total = first_value * first_unit.scale;
        if !total.is_finite() {
            return failure(out_of_range);
        }
        for (index, operator) in operators.iter().enumerate() {
            let Some((value, unit_name)) = terms.get(index + 1).and_then(|t| parse_quantity(t))
            else {
                return failure("Each value needs a unit, such as “2 m + 30 cm”.");
            };
            let Some(unit) = lookup_unit(unit_name) else {
                return SpecializedCalculation::Failure(format!("Unknown unit “{unit_name}”."));
            };
            if unit.dimension != first_unit.dimension {
                return failure("Those units measure different things.");
            }
            let base_value = value * unit.scale;
            if !base_value.is_finite() {
                return failure(out_of_range);
            }
            if *operator == '+' {
                total += base_value;
            } else {
                total -= base_value;
            }
            if !total.is_finite() {
                return failure(out_of_range);
            }
        }

        let result_unit = match target_unit_name {
            Some(name) => {
                let Some(target) = lookup_unit(name) else {
                    return SpecializedCalculation::Failure(format!("Unknown unit “{name}”."));
                };
                if target.dimension != first_unit.dimension {
                    return failure("Those units measure different things.");
                }
                target
            }
            None => first_unit,
        };
        let value = total / result_unit.scale;
        if !value.is_finite() {
            return failure(out_of_range);
        }
        SpecializedCalculation::Result(format!(
            "{} {}",
            format_number(value, self.locale.decimal_separator()),
            result_unit.symbol
        ))
    }
}

/// Splits `2 m + 30 cm - 1 in` into terms and operators. A sign directly after `e`/`E`
/// (scientific notation) or after another operator, `(` or the start belongs to the number.
fn split_arithmetic(input: &str) -> Option<(Vec<String>, Vec<char>)> {
    let mut terms: Vec<String> = Vec::new();
    let mut operators: Vec<char> = Vec::new();
    let mut term_start = 0;
    let mut previous: Option<char> = None;
    for (index, character) in input.char_indices() {
        let before = previous.replace(character);
        if character != '+' && character != '-' {
            continue;
        }
        if matches!(before, Some('e' | 'E')) {
            continue;
        }
        let prefix = input[..index].trim();
        if prefix.is_empty() || prefix.ends_with(['+', '-', '*', '/', '(']) {
            continue;
        }
        terms.push(input[term_start..index].trim().to_owned());
        operators.push(character);
        term_start = index + character.len_utf8();
    }
    if operators.is_empty() {
        return None;
    }
    terms.push(input[term_start..].trim().to_owned());
    Some((terms, operators))
}

/// Byte ranges of every `\s+(to|in|as)(?=\s|$)` match, case-insensitive, non-overlapping.
pub(crate) fn find_separators(input: &str) -> Vec<(usize, usize)> {
    let mut found = Vec::new();
    let mut cursor = 0;
    while cursor < input.len() {
        let rest = &input[cursor..];
        let Some(offset) = rest.find(char::is_whitespace) else {
            break;
        };
        let start = cursor + offset;
        let after_space = input[start..].trim_start();
        let word_start = input.len() - after_space.len();
        let word_end = word_start + 2;
        let is_keyword = after_space.get(..2).is_some_and(|word| {
            ["to", "in", "as"]
                .iter()
                .any(|keyword| word.eq_ignore_ascii_case(keyword))
        });
        let ends_word = is_keyword
            && input[word_end..]
                .chars()
                .next()
                .is_none_or(char::is_whitespace);
        if is_keyword && ends_word {
            found.push((start, word_end));
            cursor = word_end;
        } else {
            cursor = word_start.max(start + 1);
            while cursor < input.len() && !input.is_char_boundary(cursor) {
                cursor += 1;
            }
        }
    }
    found
}

/// Splits `^\s*([+-]?(?:[0-9]+(?:[.,][0-9]*)?|[.,][0-9]+)(?:[eE][+-]?[0-9]+)?)\s*(.*?)\s*$`.
fn parse_quantity(text: &str) -> Option<(f64, &str)> {
    let bytes = text.trim_start().as_bytes();
    let text = text.trim_start();
    let mut i = 0;
    if matches!(bytes.first(), Some(b'+' | b'-')) {
        i += 1;
    }
    let digits = |from: usize| {
        bytes[from..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count()
    };
    let integer_digits = digits(i);
    if integer_digits > 0 {
        i += integer_digits;
        if matches!(bytes.get(i), Some(b'.' | b',')) {
            i += 1;
            i += digits(i);
        }
    } else if matches!(bytes.get(i), Some(b'.' | b',')) && digits(i + 1) > 0 {
        i += 1 + digits(i + 1);
    } else {
        return None;
    }
    if matches!(bytes.get(i), Some(b'e' | b'E')) {
        let mut j = i + 1;
        if matches!(bytes.get(j), Some(b'+' | b'-')) {
            j += 1;
        }
        let exponent_digits = digits(j);
        if exponent_digits > 0 {
            i = j + exponent_digits;
        }
    }
    let literal = text[..i].replace(',', ".");
    let unit = text[i..].trim();
    // `.` in the original pattern does not match line terminators.
    if unit.is_empty()
        || unit.contains([
            '\n', '\r', '\u{0B}', '\u{0C}', '\u{85}', '\u{2028}', '\u{2029}',
        ])
    {
        return None;
    }
    let value = literal.parse::<f64>().ok().filter(|v| v.is_finite())?;
    Some((value, unit))
}

fn lookup_unit(raw_name: &str) -> Option<UnitDefinition> {
    let trimmed = raw_name.trim();
    let compact: String = trimmed.chars().filter(|&c| c != ' ').collect();
    let tables = tables();

    // Data symbols are case-sensitive: b is a bit and B is a byte.
    if let Some(definition) = tables.data_symbols.get(compact.as_str()) {
        return Some(*definition);
    }
    let normalized = normalize(trimmed);
    tables
        .data_names
        .get(&normalized)
        .or_else(|| tables.units.get(&normalized))
        .copied()
}

fn normalize(name: &str) -> String {
    let cleaned = name
        .to_lowercase()
        .replace(['°', 'º'], "")
        .replace(['µ', 'μ'], "u");
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Formats with up to 12 significant digits, no grouping and no exponent, rounding
/// half-even on the shortest decimal representation (what `NumberFormatter` does).
fn format_number(value: f64, decimal_separator: char) -> String {
    if value == 0.0 || !value.is_finite() {
        return "0".into();
    }
    let scientific = format!("{:e}", value.abs());
    let (mantissa, exponent) = scientific.split_once('e').unwrap_or((&scientific, "0"));
    let mut exponent: i32 = exponent.parse().unwrap_or(0);
    let mut digits: Vec<u8> = mantissa
        .bytes()
        .filter(u8::is_ascii_digit)
        .map(|b| b - b'0')
        .collect();

    if digits.len() > SIGNIFICANT_DIGITS {
        let first_dropped = digits[SIGNIFICANT_DIGITS];
        let rest_nonzero = digits[SIGNIFICANT_DIGITS + 1..].iter().any(|&d| d != 0);
        let round_up = first_dropped > 5
            || (first_dropped == 5 && (rest_nonzero || digits[SIGNIFICANT_DIGITS - 1] % 2 == 1));
        digits.truncate(SIGNIFICANT_DIGITS);
        if round_up {
            let mut position = digits.len();
            loop {
                if position == 0 {
                    digits.insert(0, 1);
                    digits.truncate(SIGNIFICANT_DIGITS);
                    exponent += 1;
                    break;
                }
                position -= 1;
                if digits[position] == 9 {
                    digits[position] = 0;
                } else {
                    digits[position] += 1;
                    break;
                }
            }
        }
    }
    while digits.len() > 1 && digits.last() == Some(&0) {
        digits.pop();
    }

    let text: String = digits.iter().map(|&d| char::from(b'0' + d)).collect();
    let integer_digits = exponent + 1;
    let mut output = String::new();
    if value < 0.0 {
        output.push('-');
    }
    if integer_digits <= 0 {
        output.push('0');
        output.push(decimal_separator);
        output.extend(std::iter::repeat_n(
            '0',
            usize::try_from(-integer_digits).unwrap_or(0),
        ));
        output.push_str(&text);
    } else {
        let integer_digits = usize::try_from(integer_digits).unwrap_or(0);
        if integer_digits >= text.len() {
            output.push_str(&text);
            output.extend(std::iter::repeat_n('0', integer_digits - text.len()));
        } else {
            output.push_str(&text[..integer_digits]);
            output.push(decimal_separator);
            output.push_str(&text[integer_digits..]);
        }
    }
    output
}

struct Tables {
    units: HashMap<String, UnitDefinition>,
    data_symbols: HashMap<&'static str, UnitDefinition>,
    data_names: HashMap<String, UnitDefinition>,
}

fn tables() -> &'static Tables {
    static TABLES: OnceLock<Tables> = OnceLock::new();
    TABLES.get_or_init(build_tables)
}

fn build_tables() -> Tables {
    use Dimension::{Duration, Length, Mass, Temperature};

    let mut units = HashMap::new();
    let mut add = |definition: UnitDefinition, aliases: &[&str]| {
        for alias in aliases {
            units.insert(normalize(alias), definition);
        }
    };

    add(
        unit("m", Length, 1.0),
        &["m", "meter", "meters", "metre", "metres"],
    );
    add(
        unit("km", Length, 1_000.0),
        &["km", "kilometer", "kilometers", "kilometre", "kilometres"],
    );
    add(
        unit("cm", Length, 0.01),
        &[
            "cm",
            "centimeter",
            "centimeters",
            "centimetre",
            "centimetres",
        ],
    );
    add(
        unit("mm", Length, 0.001),
        &[
            "mm",
            "millimeter",
            "millimeters",
            "millimetre",
            "millimetres",
        ],
    );
    add(
        unit("µm", Length, 0.000_001),
        &[
            "um",
            "µm",
            "μm",
            "micrometer",
            "micrometers",
            "micrometre",
            "micrometres",
            "micron",
            "microns",
        ],
    );
    add(
        unit("nm", Length, 0.000_000_001),
        &["nm", "nanometer", "nanometers", "nanometre", "nanometres"],
    );
    add(unit("mi", Length, 1_609.344), &["mi", "mile", "miles"]);
    add(unit("yd", Length, 0.9144), &["yd", "yard", "yards"]);
    add(unit("ft", Length, 0.3048), &["ft", "foot", "feet"]);
    add(
        unit("in", Length, 0.0254),
        &["in", "inch", "inches", "inchs"],
    );

    add(unit("g", Mass, 1.0), &["g", "gram", "grams"]);
    add(unit("kg", Mass, 1_000.0), &["kg", "kilogram", "kilograms"]);
    add(unit("mg", Mass, 0.001), &["mg", "milligram", "milligrams"]);
    add(
        unit("µg", Mass, 0.000_001),
        &["ug", "µg", "μg", "mcg", "microgram", "micrograms"],
    );
    add(
        unit("lb", Mass, 453.592_37),
        &["lb", "lbs", "pound", "pounds"],
    );
    add(unit("oz", Mass, 28.349_523_125), &["oz", "ounce", "ounces"]);
    add(unit("st", Mass, 6_350.293_18), &["st", "stone", "stones"]);

    add(
        unit("ms", Duration, 0.001),
        &["ms", "millisecond", "milliseconds"],
    );
    add(unit("s", Duration, 1.0), &["s", "sec", "second", "seconds"]);
    add(unit("min", Duration, 60.0), &["min", "minute", "minutes"]);
    add(
        unit("h", Duration, 3_600.0),
        &["h", "hr", "hrs", "hour", "hours"],
    );
    add(unit("d", Duration, 86_400.0), &["d", "day", "days"]);
    add(
        unit("wk", Duration, 604_800.0),
        &["wk", "wks", "week", "weeks"],
    );

    add(
        UnitDefinition {
            symbol: "°C",
            dimension: Temperature,
            scale: 1.0,
            offset: 273.15,
        },
        &[
            "c",
            "°c",
            "ºc",
            "celsius",
            "centigrade",
            "degree c",
            "degrees c",
            "degree celsius",
            "degrees celsius",
        ],
    );
    add(
        UnitDefinition {
            symbol: "°F",
            dimension: Temperature,
            scale: 5.0 / 9.0,
            offset: 255.372_222_222_222_2,
        },
        &[
            "f",
            "°f",
            "ºf",
            "fahrenheit",
            "degree f",
            "degrees f",
            "degree fahrenheit",
            "degrees fahrenheit",
        ],
    );
    add(
        unit("K", Temperature, 1.0),
        &["k", "kelvin", "kelvins", "degree k", "degrees k"],
    );

    // (symbol, bytes per unit, full-name aliases)
    #[rustfmt::skip]
    let data: [(&'static str, f64, [&str; 2]); 18] = [
        ("B", 1.0, ["byte", "bytes"]),
        ("kB", 1_000.0, ["kilobyte", "kilobytes"]),
        ("MB", 1_000_000.0, ["megabyte", "megabytes"]),
        ("GB", 1_000_000_000.0, ["gigabyte", "gigabytes"]),
        ("TB", 1_000_000_000_000.0, ["terabyte", "terabytes"]),
        ("KiB", 1_024.0, ["kibibyte", "kibibytes"]),
        ("MiB", 1_048_576.0, ["mebibyte", "mebibytes"]),
        ("GiB", 1_073_741_824.0, ["gibibyte", "gibibytes"]),
        ("TiB", 1_099_511_627_776.0, ["tebibyte", "tebibytes"]),
        ("b", 1.0 / 8.0, ["bit", "bits"]),
        ("kb", 1_000.0 / 8.0, ["kilobit", "kilobits"]),
        ("Mb", 1_000_000.0 / 8.0, ["megabit", "megabits"]),
        ("Gb", 1_000_000_000.0 / 8.0, ["gigabit", "gigabits"]),
        ("Tb", 1_000_000_000_000.0 / 8.0, ["terabit", "terabits"]),
        ("Kib", 1_024.0 / 8.0, ["kibibit", "kibibits"]),
        ("Mib", 1_048_576.0 / 8.0, ["mebibit", "mebibits"]),
        ("Gib", 1_073_741_824.0 / 8.0, ["gibibit", "gibibits"]),
        ("Tib", 1_099_511_627_776.0 / 8.0, ["tebibit", "tebibits"]),
    ];
    let mut data_symbols = HashMap::new();
    let mut data_names = HashMap::new();
    for (symbol, factor, names) in data {
        let definition = unit(symbol, Dimension::Data, factor);
        data_symbols.insert(symbol, definition);
        for name in names {
            data_names.insert(normalize(name), definition);
        }
    }

    Tables {
        units,
        data_symbols,
        data_names,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_formatting_rounds_to_twelve_significant_digits() {
        assert_eq!(format_number(1000.0 / 1609.344, '.'), "0.621371192237");
        assert_eq!(format_number(1.5 / 0.3048, ','), "4,92125984252");
        assert_eq!(format_number(1_048_576.0, '.'), "1048576");
        assert_eq!(format_number(1e20, '.'), "100000000000000000000");
        assert_eq!(format_number(1.5e-9, '.'), "0.0000000015");
        assert_eq!(format_number(-2.5, '.'), "-2.5");
        assert_eq!(format_number(0.9999999999999, '.'), "1");
    }

    #[test]
    fn separators_are_found_case_insensitively() {
        assert_eq!(find_separators("1 KM TO mi"), vec![(4, 7)]);
        assert_eq!(find_separators("1 in to m").len(), 2);
        assert!(find_separators("1 tomato").is_empty());
        assert!(find_separators("1 mi").is_empty());
    }

    #[test]
    fn quantities_parse_like_the_original_pattern() {
        assert_eq!(parse_quantity("1,5 km"), Some((1.5, "km")));
        assert_eq!(parse_quantity("-3e2m"), Some((-300.0, "m")));
        assert_eq!(parse_quantity("5e"), Some((5.0, "e")));
        assert_eq!(parse_quantity(".5 g"), Some((0.5, "g")));
        assert_eq!(parse_quantity("5"), None);
        assert_eq!(parse_quantity("km"), None);
    }
}
