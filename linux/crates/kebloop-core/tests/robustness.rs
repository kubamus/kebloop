//! No input may panic any engine, and evaluation must stay bounded.

use jiff::tz::TimeZone;
use kebloop_core::{
    AppSearch, AppSearchEntry, CalculatorEngine, CurrencyPair, CurrencyRateSnapshot,
    DateCalculationEngine, EmojiCatalog, Locale, UnitConversionEngine,
};

/// A tiny deterministic xorshift generator (no external dependencies).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }
}

const FRAGMENTS: &[&str] = &[
    "1",
    "2",
    "10",
    "0",
    "999999999999",
    "1e",
    "1e308",
    "1e-5",
    ".",
    ",",
    "+",
    "-",
    "−",
    "*",
    "×",
    "/",
    "÷",
    "^",
    "%",
    "(",
    ")",
    " ",
    "  ",
    "\t",
    "\n",
    "pi",
    "e",
    "sqrt",
    "abs",
    "round",
    "of",
    "off",
    "to",
    "in",
    "as",
    "and",
    "km",
    "mi",
    "°C",
    "°F",
    "K",
    "MiB",
    "MB",
    "b",
    "B",
    "µm",
    "feet",
    "kg",
    "lb",
    "today",
    "tomorrow",
    "yesterday",
    "days",
    "day",
    "week",
    "weeks",
    "month",
    "months",
    "year",
    "years",
    "between",
    "now",
    "time",
    "pm",
    "am",
    "9:30",
    "9:30 AM",
    "New York",
    "Tokyo",
    "PST",
    "USD",
    "eur",
    "$",
    "€",
    "zł",
    "dollars",
    "inchs",
    "s",
    "ms",
    "9223372036854775808",
    "date",
    "difference",
    "from",
    "2026-02-30",
    "2026-03-08",
    "2026-03-08T02:30:00",
    "2026-03-08T00:00:00Z",
    "2026-03-08T00:00:00+05:30",
    "01/02/2026",
    "01.02.2026",
    "99/99/9999",
    "0000-01-01",
    "9999-12-31",
    "9999-12-31 23:59",
    "T",
    "Z",
    "é",
    "ł",
    "🦀",
    "👨‍👩‍👧",
    "\u{0301}",
    "\u{202e}",
    "\u{0}",
    "٣",
    "²",
    "½",
];

fn random_input(rng: &mut Rng) -> String {
    let parts = 1 + rng.below(14);
    let mut text = String::new();
    for _ in 0..parts {
        if rng.below(10) == 0 {
            let code = u32::try_from(rng.below(0x11_0000)).unwrap_or(0x41);
            text.push(char::from_u32(code).unwrap_or('?'));
        } else {
            text.push_str(FRAGMENTS[rng.below(FRAGMENTS.len())]);
        }
        if rng.below(3) == 0 {
            text.push(' ');
        }
    }
    text
}

#[test]
fn random_strings_never_panic_any_engine() {
    let now = "2026-10-12T10:00:00Z".parse().expect("timestamp");
    let zones: Vec<TimeZone> = ["UTC", "America/New_York", "Pacific/Auckland"]
        .iter()
        .map(|name| TimeZone::get(name).expect("zone"))
        .collect();
    let locales = [Locale::en_us(), Locale::en_gb(), Locale::pl(), Locale::de()];
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let entries = vec![
        AppSearchEntry::new("a", "Visual Studio Code", "/a").with_aliases(["code"]),
        AppSearchEntry::new("b", "Łódź", "/b"),
    ];
    let search = AppSearch::new();
    let usage = std::collections::HashMap::new();
    let rates: Vec<CurrencyRateSnapshot> = [("USD", "EUR", 0.92), ("PLN", "USD", 0.25)]
        .into_iter()
        .filter_map(|(base, quote, rate)| {
            Some(CurrencyRateSnapshot::new(
                CurrencyPair::new(base, quote)?,
                rate,
                now,
                now,
            ))
        })
        .collect();

    for iteration in 0..30_000 {
        let input = random_input(&mut rng);
        let locale = locales[iteration % locales.len()].clone();
        let zone = zones[iteration % zones.len()].clone();

        let calculator =
            CalculatorEngine::new(locale.clone(), zone.clone()).with_reference_time(now);
        let _ = calculator.evaluate(&input);
        let _ = calculator.evaluate_with_rates(&input, &rates);
        let _ = CalculatorEngine::currency_pair(&input);
        let _ = DateCalculationEngine::new(locale.clone(), zone).evaluate(&input, now);
        let _ = UnitConversionEngine::new(locale).evaluate(&input);
        let _ = search.ranked(&input, &entries, &usage);
        // The catalog has ~4,000 entries; sampling keeps the debug-build run short.
        if iteration % 10 == 0 {
            let _ = EmojiCatalog::search(&input);
        }
    }
}

#[test]
fn pathological_inputs_stay_bounded() {
    let calculator = CalculatorEngine::new(Locale::en_us(), TimeZone::UTC);
    let long = "a ".repeat(100_000);
    let inputs = [
        long.clone(),
        "-".repeat(300),
        "2^".repeat(255) + "2",
        "(".repeat(100_000),
        " to ".repeat(10_000),
        "2026-01-01 + ".to_owned() + &"9".repeat(40) + " days",
        "2026-01-01 + 9999999999999 years".to_owned(),
        "in 99999999999999999999 days".to_owned(),
        "1".repeat(100_000) + " km to m",
        "$".repeat(100_000) + " to usd",
        "9:30 pm in x ".repeat(5_000) + "to y",
        "now + ".repeat(10_000),
    ];
    for input in inputs {
        let _ = calculator.evaluate(&input);
        let _ = DateCalculationEngine::new(Locale::en_us(), TimeZone::UTC)
            .evaluate(&input, jiff::Timestamp::UNIX_EPOCH);
        let _ = UnitConversionEngine::new(Locale::en_us()).evaluate(&input);
    }
}
