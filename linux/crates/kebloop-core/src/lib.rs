//! Pure, synchronous Kebloop engines shared by every Linux front end.
//!
//! This crate is a behaviour-for-behaviour port of `Sources/KebloopCore`:
//!
//! * [`calculator`] - the launcher calculator (arithmetic, percentages, functions),
//!   which also dispatches to [`units`] and [`dates`];
//! * [`units`] - unit conversions (length, mass, duration, temperature, data) and mixed-unit sums;
//! * [`dates`] - date differences, date arithmetic and relative dates (`now`, `in 2h`);
//! * [`timezones`] - named time-zone lookups and clock conversions;
//! * [`currency`] - currency conversions from saved exchange rates (no I/O);
//! * [`search`] - fuzzy ranking with aliases, diacritic folding and usage boost;
//! * [`emoji`] - the offline Unicode emoji catalog and its search;
//! * [`ai`] - prompt URLs for ChatGPT and Claude;
//! * [`locale`] - the small injectable locale used for number and date formatting.
//!
//! Nothing here performs I/O. The current time and the time zone are injected, so
//! every engine is deterministic under test. No input can make the engines panic.

pub mod ai;
pub mod calculator;
pub mod currency;
pub mod dates;
pub mod emoji;
pub mod locale;
pub mod search;
pub mod timezones;
pub mod units;

mod emoji_data;

pub use ai::{AiPromptUrl, AiProvider};
pub use calculator::{
    CalculationOutcome, CalculationResult, CalculatorEngine, SpecializedCalculation,
};
pub use currency::{CurrencyPair, CurrencyRateCachePolicy, CurrencyRateSnapshot};
pub use dates::DateCalculationEngine;
pub use emoji::{EmojiCatalog, EmojiEntry};
pub use locale::Locale;
pub use search::{AppSearch, AppSearchEntry, Searchable};
pub use units::UnitConversionEngine;
