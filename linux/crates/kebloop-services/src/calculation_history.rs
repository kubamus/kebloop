//! The calculator history: results the user explicitly accepted (copied). Port of
//! `Sources/Kebloop/CalculationHistoryStore.swift`.
//!
//! Typing never records anything. The history is local: it lives in its own file
//! (`$XDG_DATA_HOME/kebloop/calculation-history.json`) and is not part of the portable
//! configuration, so it is excluded from profiles and from export/import.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use jiff::Timestamp;
use kebloop_core::CalculationResult;
use serde::{Deserialize, Serialize};

use crate::error::ConfigError;
use crate::util::{atomic_write, new_id};

/// File name inside the data directory.
pub const FILE_NAME: &str = "calculation-history.json";
/// Most entries kept (newest first).
pub const MAXIMUM_ENTRIES: usize = 100;
/// Larger files are ignored and replaced.
pub const MAXIMUM_ENCODED_BYTES: usize = 128 * 1024;
/// Longest accepted expression, display or clipboard text, in UTF-8 bytes.
pub const MAXIMUM_FIELD_BYTES: usize = 2 * 1024;
/// Prefix of launcher result ids for history entries.
pub const RESULT_ID_PREFIX: &str = "calculation-history:";

/// One accepted calculation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalculationHistoryEntry {
    /// Unique identifier.
    pub id: String,
    /// The expression the user typed.
    pub expression: String,
    /// The answer shown.
    pub display: String,
    /// The text that was copied, when it differed from `display`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clipboard: Option<String>,
    /// When it was accepted (Unix seconds in the file).
    #[serde(with = "unix_seconds")]
    pub accepted_at: Timestamp,
}

mod unix_seconds {
    use jiff::Timestamp;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(value: &Timestamp, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_i64(value.as_second())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Timestamp, D::Error> {
        Timestamp::from_second(i64::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

impl CalculationHistoryEntry {
    /// The calculation as the engine returned it.
    pub fn result(&self) -> CalculationResult {
        CalculationResult {
            expression: self.expression.clone(),
            display: self.display.clone(),
            clipboard: self
                .clipboard
                .clone()
                .unwrap_or_else(|| self.display.clone()),
        }
    }

    /// The launcher result id (`calculation-history:<id>`).
    pub fn result_id(&self) -> String {
        format!("{RESULT_ID_PREFIX}{}", self.id)
    }

    fn is_valid(&self) -> bool {
        !self.id.is_empty()
            && !self.expression.is_empty()
            && !self.display.is_empty()
            && self.expression.len() <= MAXIMUM_FIELD_BYTES
            && self.display.len() <= MAXIMUM_FIELD_BYTES
            && self
                .clipboard
                .as_deref()
                .is_none_or(|c| c.len() <= MAXIMUM_FIELD_BYTES)
    }

    /// Case-insensitive substring match on the expression or the answer; every entry matches
    /// an empty query.
    pub fn matches(&self, query: &str) -> bool {
        if query.is_empty() {
            return true;
        }
        let needle = query.to_lowercase();
        self.expression.to_lowercase().contains(&needle)
            || self.display.to_lowercase().contains(&needle)
    }
}

type Clock = Arc<dyn Fn() -> Timestamp + Send + Sync>;

/// The stored history.
pub struct CalculationHistoryStore {
    path: Option<PathBuf>,
    entries: Vec<CalculationHistoryEntry>,
    clock: Clock,
    last_error: Option<String>,
}

impl std::fmt::Debug for CalculationHistoryStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CalculationHistoryStore")
            .field("path", &self.path)
            .field("entries", &self.entries.len())
            .finish_non_exhaustive()
    }
}

impl CalculationHistoryStore {
    /// Opens the history in `data_dir`. A missing, oversized or unreadable file is empty;
    /// invalid entries are skipped and at most 100 are kept.
    pub fn open(data_dir: &Path, clock: Clock) -> Self {
        let path = data_dir.join(FILE_NAME);
        let entries = read(&path);
        Self {
            path: Some(path),
            entries,
            clock,
            last_error: None,
        }
    }

    /// A history that never touches the disk (when the data directory is unusable).
    pub fn in_memory(clock: Clock) -> Self {
        Self {
            path: None,
            entries: Vec::new(),
            clock,
            last_error: None,
        }
    }

    /// Opens the history with the real clock.
    pub fn with_system_clock(data_dir: &Path) -> Self {
        Self::open(data_dir, Arc::new(Timestamp::now))
    }

    /// Entries, newest first.
    pub fn entries(&self) -> &[CalculationHistoryEntry] {
        &self.entries
    }

    /// Entries matching `query` (see [`CalculationHistoryEntry::matches`]), newest first.
    pub fn filtered(&self, query: &str) -> Vec<&CalculationHistoryEntry> {
        self.entries.iter().filter(|e| e.matches(query)).collect()
    }

    /// The reason the last save failed, if it did.
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    /// Records a result the user accepted. Returns `false` when the result is not storable
    /// (empty, or a field over 2 KiB). The same calculation accepted twice is two entries.
    pub fn record_accepted(&mut self, result: &CalculationResult) -> bool {
        let truncate = |text: &str| -> String { text.chars().take(MAXIMUM_FIELD_BYTES).collect() };
        let expression = truncate(&result.expression);
        let display = truncate(&result.display);
        let clipboard = truncate(&result.clipboard);
        if expression.is_empty()
            || display.is_empty()
            || expression.len() > MAXIMUM_FIELD_BYTES
            || display.len() > MAXIMUM_FIELD_BYTES
            || clipboard.len() > MAXIMUM_FIELD_BYTES
        {
            return false;
        }
        self.entries.insert(
            0,
            CalculationHistoryEntry {
                id: new_id(),
                expression,
                display,
                clipboard: Some(clipboard),
                accepted_at: (self.clock)(),
            },
        );
        self.entries.truncate(MAXIMUM_ENTRIES);
        self.persist();
        true
    }

    /// Removes every entry (and the file).
    pub fn clear(&mut self) {
        self.entries.clear();
        if let Some(path) = &self.path {
            self.last_error = match std::fs::remove_file(path) {
                Ok(()) => None,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => Some(ConfigError::io(path, error).to_string()),
            };
        }
    }

    fn persist(&mut self) {
        let Some(path) = &self.path else { return };
        let result = serde_json::to_vec(&self.entries)
            .map_err(ConfigError::from)
            .and_then(|bytes| {
                if bytes.len() <= MAXIMUM_ENCODED_BYTES {
                    atomic_write(path, &bytes, 0o600)
                } else {
                    Ok(())
                }
            });
        self.last_error = result.err().map(|error| error.to_string());
    }
}

fn read(path: &Path) -> Vec<CalculationHistoryEntry> {
    let Ok(metadata) = std::fs::metadata(path) else {
        return Vec::new();
    };
    if usize::try_from(metadata.len()).is_ok_and(|len| len > MAXIMUM_ENCODED_BYTES) {
        return Vec::new();
    }
    let decoded: Vec<CalculationHistoryEntry> = std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    decoded
        .into_iter()
        .take(MAXIMUM_ENTRIES)
        .filter(CalculationHistoryEntry::is_valid)
        .collect()
}
