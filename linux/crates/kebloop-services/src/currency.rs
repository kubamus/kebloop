//! Exchange-rate cache and refresh for the calculator's currency conversions.
//!
//! Port of `Sources/Kebloop/CurrencyRateStore.swift`:
//!
//! * the rate source is the [Frankfurter](https://frankfurter.dev/) API, one request per
//!   currency pair (`GET {base}/v2/rate/{from}/{to}`, `Accept: application/json`), which
//!   returns `{"date":"2026-10-02","base":"USD","quote":"PLN","rate":3.75}`;
//! * only a pair that appears in a conversion query is requested, never on every keystroke:
//!   fresh pairs (downloaded less than 24 hours ago) are not requested again, a pair that was
//!   attempted within the last 30 seconds is not retried, and concurrent requests for one
//!   pair are coalesced;
//! * up to 64 pairs are cached in `currency-rates.json` under the injected cache directory
//!   (newest download first); entries older than 30 days are dropped, so a recent rate keeps
//!   working offline and a failed or invalid response never removes it;
//! * responses larger than 32 KiB, non-2xx statuses, non-finite or non-positive rates,
//!   mismatching currencies and observation dates more than a day in the future are ignored.
//!
//! All methods are blocking and meant for a worker thread; reading [`CurrencyRateStore::snapshots`]
//! is cheap and never touches the network or the disk. The HTTP client is injected
//! ([`RateFetcher`]) so everything is testable offline; [`UreqFetcher`] is the production
//! implementation (blocking `ureq` with rustls).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use jiff::Timestamp;
use jiff::civil::Date;
use kebloop_core::calculator::CalculatorEngine;
use kebloop_core::{CurrencyPair, CurrencyRateCachePolicy, CurrencyRateSnapshot};
use serde::Deserialize;

use crate::util::atomic_write;

/// The rate provider.
pub const DEFAULT_API_BASE: &str = "https://api.frankfurter.dev";
/// Name of the cache file inside the cache directory.
pub const CACHE_FILE_NAME: &str = "currency-rates.json";
/// Largest accepted response body (also checked against `Content-Length`).
pub const MAXIMUM_RESPONSE_BYTES: u64 = 32 * 1024;
/// A larger cache file is ignored (and replaced on the next download).
pub const MAXIMUM_CACHE_BYTES: u64 = 64 * 1024;
/// Minimum time between two attempts for one pair.
pub const RETRY_DELAY_SECONDS: i64 = 30;
/// Request timeout.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(8);
/// An observation dated further than this into the future is rejected.
const MAXIMUM_FUTURE_SECONDS: i64 = 24 * 60 * 60;

/// One rate request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RateRequest {
    /// Absolute URL.
    pub url: String,
    /// Value of the `Accept` header.
    pub accept: &'static str,
}

/// A completed HTTP exchange.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RateResponse {
    /// HTTP status code.
    pub status: u16,
    /// Body, truncated by the fetcher at [`MAXIMUM_RESPONSE_BYTES`] + 1 bytes.
    pub body: Vec<u8>,
    /// The announced `Content-Length`, if any.
    pub content_length: Option<u64>,
}

/// Why a request produced no response.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FetchError {
    /// Connection, TLS, timeout or protocol failure.
    #[error("network error: {0}")]
    Network(String),
}

/// Performs one blocking HTTP GET. Implementations must be thread-safe.
pub trait RateFetcher: Send + Sync {
    /// Fetches `request`.
    fn fetch(&self, request: &RateRequest) -> Result<RateResponse, FetchError>;
}

/// The production fetcher: blocking `ureq` over rustls, eight second timeout, no redirects to
/// other schemes, body read limited to 32 KiB.
#[derive(Debug, Clone)]
pub struct UreqFetcher {
    agent: ureq::Agent,
}

impl UreqFetcher {
    /// A fetcher with the standard configuration (HTTPS only).
    pub fn new() -> Self {
        Self::build(true)
    }

    /// Like [`UreqFetcher::new`] but also allows plain `http://` URLs, for tests against a
    /// local server and for a self-hosted mirror on a trusted network.
    pub fn allowing_plain_http() -> Self {
        Self::build(false)
    }

    fn build(https_only: bool) -> Self {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(REQUEST_TIMEOUT))
            .http_status_as_error(false)
            .https_only(https_only)
            .max_redirects(3)
            .user_agent("Kebloop")
            .build()
            .into();
        Self { agent }
    }
}

impl Default for UreqFetcher {
    fn default() -> Self {
        Self::new()
    }
}

impl RateFetcher for UreqFetcher {
    fn fetch(&self, request: &RateRequest) -> Result<RateResponse, FetchError> {
        let mut response = self
            .agent
            .get(&request.url)
            .header("Accept", request.accept)
            .call()
            .map_err(|error| FetchError::Network(error.to_string()))?;
        let status = response.status().as_u16();
        let content_length = response.body().content_length();
        // A body over the limit is reported as exactly "limit + 1" bytes so it is rejected.
        let body = match response
            .body_mut()
            .with_config()
            .limit(MAXIMUM_RESPONSE_BYTES)
            .read_to_vec()
        {
            Ok(body) => body,
            Err(ureq::Error::BodyExceedsLimit(_)) => {
                vec![0; usize::try_from(MAXIMUM_RESPONSE_BYTES + 1).unwrap_or(usize::MAX)]
            }
            Err(error) => return Err(FetchError::Network(error.to_string())),
        };
        Ok(RateResponse {
            status,
            body,
            content_length,
        })
    }
}

/// What a refresh call did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshOutcome {
    /// The input is not a currency conversion query; nothing happened.
    NotACurrencyQuery,
    /// A saved rate is fresh (or the pair is identical); no request was made.
    Fresh,
    /// The pair was attempted less than 30 seconds ago.
    Throttled,
    /// Another thread is already downloading this pair.
    InFlight,
    /// A new rate was downloaded and saved; re-evaluate the calculator.
    Updated,
    /// The request failed or its response was rejected; saved rates are unchanged.
    Failed,
}

#[derive(Debug, Default)]
struct State {
    snapshots: Vec<CurrencyRateSnapshot>,
    active: HashSet<CurrencyPair>,
    last_attempt: HashMap<CurrencyPair, Timestamp>,
    last_error: Option<String>,
}

type Clock = Arc<dyn Fn() -> Timestamp + Send + Sync>;

/// The rate cache. Cheap to share behind an `Arc`; every method takes `&self`.
pub struct CurrencyRateStore {
    cache_path: PathBuf,
    api_base: String,
    fetcher: Arc<dyn RateFetcher>,
    clock: Clock,
    state: Mutex<State>,
}

impl std::fmt::Debug for CurrencyRateStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CurrencyRateStore")
            .field("cache_path", &self.cache_path)
            .field("api_base", &self.api_base)
            .finish_non_exhaustive()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl CurrencyRateStore {
    /// Opens the cache in `cache_dir` (`$XDG_CACHE_HOME/kebloop`). A missing, oversized or
    /// unreadable cache file is treated as empty; expired and future-dated entries are
    /// dropped and the file is rewritten when anything changed.
    pub fn open(cache_dir: &Path, fetcher: Arc<dyn RateFetcher>, clock: Clock) -> Self {
        let cache_path = cache_dir.join(CACHE_FILE_NAME);
        let cached = read_cache(&cache_path);
        let snapshots = retained(&cached, clock());
        let store = Self {
            cache_path,
            api_base: DEFAULT_API_BASE.to_owned(),
            fetcher,
            clock,
            state: Mutex::new(State {
                snapshots,
                ..State::default()
            }),
        };
        if lock(&store.state).snapshots != cached {
            store.persist();
        }
        store
    }

    /// Opens the cache with the real clock and the `ureq` fetcher.
    pub fn with_system_defaults(cache_dir: &Path) -> Self {
        Self::open(
            cache_dir,
            Arc::new(UreqFetcher::new()),
            Arc::new(Timestamp::now),
        )
    }

    /// Replaces the provider base URL (tests, mirrors).
    #[must_use]
    pub fn with_api_base(mut self, base: &str) -> Self {
        self.api_base = base.trim_end_matches('/').to_owned();
        self
    }

    /// Path of the cache file.
    pub fn cache_path(&self) -> &Path {
        &self.cache_path
    }

    /// All saved rates, newest download first. Pass these to
    /// [`CalculatorEngine::evaluate_with_rates`].
    pub fn snapshots(&self) -> Vec<CurrencyRateSnapshot> {
        lock(&self.state).snapshots.clone()
    }

    /// The newest saved rate usable for `pair` (in either direction).
    pub fn snapshot(&self, pair: &CurrencyPair) -> Option<CurrencyRateSnapshot> {
        lock(&self.state)
            .snapshots
            .iter()
            .filter(|s| s.pair == *pair || is_inverse(&s.pair, pair))
            .max_by_key(|s| s.fetched_at)
            .cloned()
    }

    /// Whether a download of `pair` is in flight.
    pub fn is_refreshing(&self, pair: &CurrencyPair) -> bool {
        lock(&self.state).active.contains(pair)
    }

    /// The reason the cache file could not be written the last time, if it could not.
    pub fn last_error(&self) -> Option<String> {
        lock(&self.state).last_error.clone()
    }

    /// Safe to call as the query changes: ordinary queries are ignored, fresh pairs are not
    /// fetched again and repeated input for an in-flight pair is coalesced. Blocking.
    pub fn refresh_if_needed(&self, input: &str) -> RefreshOutcome {
        match CalculatorEngine::currency_pair(input) {
            Some(pair) => self.refresh_pair(&pair),
            None => RefreshOutcome::NotACurrencyQuery,
        }
    }

    /// Downloads `pair` unless a fresh rate is saved, it was attempted recently or another
    /// thread is already on it. Blocking.
    pub fn refresh_pair(&self, pair: &CurrencyPair) -> RefreshOutcome {
        if pair.base() == pair.quote() {
            return RefreshOutcome::Fresh;
        }
        let current = (self.clock)();
        {
            let mut state = lock(&self.state);
            let fresh = state
                .snapshots
                .iter()
                .filter(|s| s.pair == *pair || is_inverse(&s.pair, pair))
                .max_by_key(|s| s.fetched_at)
                .is_some_and(|cached| {
                    current.as_second() - cached.fetched_at.as_second()
                        < CurrencyRateCachePolicy::FRESHNESS_SECONDS
                });
            if fresh {
                return RefreshOutcome::Fresh;
            }
            if let Some(last) = state.last_attempt.get(pair)
                && current.as_second() - last.as_second() < RETRY_DELAY_SECONDS
            {
                return RefreshOutcome::Throttled;
            }
            if !state.active.insert(pair.clone()) {
                return RefreshOutcome::InFlight;
            }
            state.last_attempt.insert(pair.clone(), current);
        }

        let downloaded = self.fetch(pair);

        let mut state = lock(&self.state);
        state.active.remove(pair);
        let Some(downloaded) = downloaded else {
            return RefreshOutcome::Failed;
        };
        let mut combined: Vec<CurrencyRateSnapshot> = state
            .snapshots
            .iter()
            .filter(|s| s.pair != *pair)
            .cloned()
            .collect();
        combined.push(downloaded);
        state.snapshots = retained(&combined, (self.clock)());
        drop(state);
        self.persist();
        RefreshOutcome::Updated
    }

    fn fetch(&self, pair: &CurrencyPair) -> Option<CurrencyRateSnapshot> {
        let request = RateRequest {
            url: format!(
                "{}/v2/rate/{}/{}",
                self.api_base,
                pair.base().to_lowercase(),
                pair.quote().to_lowercase()
            ),
            accept: "application/json",
        };
        let response = self.fetcher.fetch(&request).ok()?;
        if !(200..300).contains(&response.status)
            || u64::try_from(response.body.len()).ok()? > MAXIMUM_RESPONSE_BYTES
            || response
                .content_length
                .is_some_and(|length| length > MAXIMUM_RESPONSE_BYTES)
        {
            return None;
        }
        let payload: ProviderRate = serde_json::from_slice(&response.body).ok()?;
        let rate_date = parse_provider_date(&payload.date)?;
        let now = (self.clock)();
        if payload.base.to_uppercase() != pair.base()
            || payload.quote.to_uppercase() != pair.quote()
            || !payload.rate.is_finite()
            || payload.rate <= 0.0
            || rate_date.as_second() > now.as_second().saturating_add(MAXIMUM_FUTURE_SECONDS)
        {
            return None;
        }
        Some(CurrencyRateSnapshot::new(
            pair.clone(),
            payload.rate,
            rate_date,
            now,
        ))
    }

    fn persist(&self) {
        let snapshots = lock(&self.state).snapshots.clone();
        let result = serde_json::to_vec(&snapshots)
            .map_err(crate::ConfigError::from)
            .and_then(|bytes| {
                if u64::try_from(bytes.len()).is_ok_and(|len| len <= MAXIMUM_CACHE_BYTES) {
                    atomic_write(&self.cache_path, &bytes, 0o600)
                } else {
                    Ok(())
                }
            });
        lock(&self.state).last_error = result.err().map(|error| error.to_string());
    }
}

#[derive(Deserialize)]
struct ProviderRate {
    date: String,
    base: String,
    quote: String,
    rate: f64,
}

fn is_inverse(candidate: &CurrencyPair, pair: &CurrencyPair) -> bool {
    candidate.base() == pair.quote() && candidate.quote() == pair.base()
}

fn read_cache(path: &Path) -> Vec<CurrencyRateSnapshot> {
    let Ok(metadata) = std::fs::metadata(path) else {
        return Vec::new();
    };
    if metadata.len() > MAXIMUM_CACHE_BYTES {
        return Vec::new();
    }
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// Keeps the newest valid snapshot of each pair (finite positive rate, downloaded no more than
/// 30 days ago and not in the future), newest first, at most 64.
fn retained(values: &[CurrencyRateSnapshot], now: Timestamp) -> Vec<CurrencyRateSnapshot> {
    let mut latest: HashMap<&CurrencyPair, &CurrencyRateSnapshot> = HashMap::new();
    for snapshot in values {
        if !snapshot.rate.is_finite() || snapshot.rate <= 0.0 {
            continue;
        }
        let age = now.as_second() - snapshot.fetched_at.as_second();
        if !(0..=CurrencyRateCachePolicy::MAXIMUM_AGE_SECONDS).contains(&age) {
            continue;
        }
        if latest
            .get(&snapshot.pair)
            .is_some_and(|previous| previous.fetched_at >= snapshot.fetched_at)
        {
            continue;
        }
        latest.insert(&snapshot.pair, snapshot);
    }
    let mut kept: Vec<CurrencyRateSnapshot> = latest.into_values().cloned().collect();
    kept.sort_by(|a, b| {
        b.fetched_at
            .cmp(&a.fetched_at)
            .then_with(|| a.pair.base().cmp(b.pair.base()))
            .then_with(|| a.pair.quote().cmp(b.pair.quote()))
    });
    kept.truncate(CurrencyRateCachePolicy::MAXIMUM_ENTRIES);
    kept
}

/// Strict `yyyy-MM-dd`, interpreted as midnight UTC.
fn parse_provider_date(text: &str) -> Option<Timestamp> {
    let bytes = text.as_bytes();
    let digits = |range: std::ops::Range<usize>| -> Option<i16> {
        let part = text.get(range)?;
        if part.bytes().all(|b| b.is_ascii_digit()) {
            part.parse().ok()
        } else {
            None
        }
    };
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return None;
    }
    let year = digits(0..4)?;
    let month = i8::try_from(digits(5..7)?).ok()?;
    let day = i8::try_from(digits(8..10)?).ok()?;
    if year < 1 {
        return None;
    }
    let date = Date::new(year, month, day).ok()?;
    date.to_zoned(jiff::tz::TimeZone::UTC)
        .ok()
        .map(|zoned| zoned.timestamp())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_dates_are_strict() {
        assert_eq!(
            parse_provider_date("2026-10-02").map(Timestamp::as_second),
            Some(1_790_899_200)
        );
        for bad in [
            "2026-1-02",
            "2026-02-30",
            "20261002",
            "2026-10-02T00:00:00Z",
            "0000-01-01",
            "",
            "２０２６-10-02",
        ] {
            assert_eq!(parse_provider_date(bad), None, "{bad}");
        }
    }
}
