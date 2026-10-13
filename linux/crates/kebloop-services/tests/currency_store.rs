//! Port of the currency tests of `Tests/KebloopTests/CalculationStoreTests.swift`: the bounded
//! cache, offline use of saved rates, the pair endpoint and request coalescing, invalid
//! responses. The HTTP client is injected, so nothing here needs the network.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use jiff::Timestamp;
use kebloop_core::calculator::CalculatorEngine;
use kebloop_core::{
    CalculationOutcome, CurrencyPair, CurrencyRateCachePolicy, CurrencyRateSnapshot,
};
use kebloop_services::currency::{
    CACHE_FILE_NAME, CurrencyRateStore, FetchError, RateFetcher, RateRequest, RateResponse,
    RefreshOutcome, UreqFetcher,
};

const NOW: i64 = 1_791_000_000;
const DAY: i64 = 86_400;

fn at(seconds: i64) -> Timestamp {
    Timestamp::from_second(seconds).expect("valid timestamp")
}

fn pair(base: &str, quote: &str) -> CurrencyPair {
    CurrencyPair::new(base, quote).expect("supported pair")
}

/// A clock the test can move.
#[derive(Clone)]
struct Clock(Arc<AtomicI64>);

impl Clock {
    fn new(seconds: i64) -> Self {
        Self(Arc::new(AtomicI64::new(seconds)))
    }

    fn advance(&self, seconds: i64) {
        self.0.fetch_add(seconds, Ordering::SeqCst);
    }

    fn as_fn(&self) -> Arc<dyn Fn() -> Timestamp + Send + Sync> {
        let seconds = self.0.clone();
        Arc::new(move || at(seconds.load(Ordering::SeqCst)))
    }
}

/// Answers every request with a canned response and records the requests.
struct Mock {
    response: Result<RateResponse, FetchError>,
    delay: Duration,
    requests: Mutex<Vec<RateRequest>>,
}

impl Mock {
    fn json(body: &str) -> Arc<Self> {
        Self::with(Ok(RateResponse {
            status: 200,
            body: body.as_bytes().to_vec(),
            content_length: Some(body.len() as u64),
        }))
    }

    fn with(response: Result<RateResponse, FetchError>) -> Arc<Self> {
        Arc::new(Self {
            response,
            delay: Duration::ZERO,
            requests: Mutex::new(Vec::new()),
        })
    }

    fn requests(&self) -> Vec<RateRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl RateFetcher for Mock {
    fn fetch(&self, request: &RateRequest) -> Result<RateResponse, FetchError> {
        self.requests.lock().unwrap().push(request.clone());
        std::thread::sleep(self.delay);
        self.response.clone()
    }
}

fn open(dir: &std::path::Path, fetcher: Arc<dyn RateFetcher>, clock: &Clock) -> CurrencyRateStore {
    CurrencyRateStore::open(dir, fetcher, clock.as_fn())
}

fn write_cache(dir: &std::path::Path, snapshots: &[CurrencyRateSnapshot]) {
    std::fs::write(
        dir.join(CACHE_FILE_NAME),
        serde_json::to_vec(snapshots).unwrap(),
    )
    .unwrap();
}

const USD_PLN: &str = r#"{"date":"2026-10-02","base":"USD","quote":"PLN","rate":3.75}"#;

#[test]
fn the_cache_is_bounded_and_keeps_recent_rates_for_offline_use() {
    let dir = tempfile::tempdir().unwrap();
    let clock = Clock::new(NOW);
    let codes = [
        "USD", "EUR", "GBP", "JPY", "CNY", "CAD", "AUD", "NZD", "CHF", "PLN", "SEK", "NOK",
    ];
    let mut snapshots = Vec::new();
    for base in codes {
        for quote in codes {
            let skipped = base == quote
                || (base == "USD" && quote == "EUR")
                || (base == "EUR" && quote == "USD")
                || (base == "EUR" && quote == "GBP");
            if skipped {
                continue;
            }
            let offset = i64::try_from(snapshots.len()).unwrap();
            snapshots.push(CurrencyRateSnapshot::new(
                pair(base, quote),
                1.25,
                at(NOW - DAY),
                at(NOW - offset),
            ));
        }
    }
    let stale_pair = pair("USD", "EUR");
    snapshots.truncate(CurrencyRateCachePolicy::MAXIMUM_ENTRIES - 1);
    snapshots.push(CurrencyRateSnapshot::new(
        stale_pair.clone(),
        0.9,
        at(NOW - DAY),
        at(NOW - 2 * DAY),
    ));
    snapshots.push(CurrencyRateSnapshot::new(
        pair("EUR", "GBP"),
        0.8,
        at(NOW - 3 * DAY),
        at(NOW - 3 * DAY),
    ));
    write_cache(dir.path(), &snapshots);

    let store = open(dir.path(), Mock::json(USD_PLN), &clock);
    assert_eq!(
        store.snapshots().len(),
        CurrencyRateCachePolicy::MAXIMUM_ENTRIES
    );
    let stale = store.snapshot(&stale_pair).expect("saved rate");
    assert_eq!(stale.rate, 0.9);
    assert!(stale.is_stale(at(NOW)));
    assert!(
        store
            .snapshots()
            .iter()
            .all(|s| s.pair != pair("EUR", "GBP")),
        "the oldest download is dropped at the limit"
    );
    // The pruned set was written back.
    let reopened = open(dir.path(), Mock::json(USD_PLN), &clock);
    assert_eq!(reopened.snapshots(), store.snapshots());
    assert!(
        store
            .snapshots()
            .windows(2)
            .all(|w| w[0].fetched_at >= w[1].fetched_at),
        "newest download first"
    );
}

#[test]
fn rates_older_than_thirty_days_or_from_the_future_are_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let clock = Clock::new(NOW);
    let expired = CurrencyRateSnapshot::new(
        pair("USD", "EUR"),
        0.9,
        at(NOW - 40 * DAY),
        at(NOW - 40 * DAY),
    );
    let future = CurrencyRateSnapshot::new(pair("USD", "PLN"), 4.0, at(NOW), at(NOW + 100));
    let invalid = CurrencyRateSnapshot::new(pair("USD", "GBP"), 0.0, at(NOW), at(NOW));
    let kept = CurrencyRateSnapshot::new(
        pair("USD", "JPY"),
        150.0,
        at(NOW - 29 * DAY),
        at(NOW - 29 * DAY),
    );
    write_cache(dir.path(), &[expired, future, invalid, kept.clone()]);
    let store = open(dir.path(), Mock::json(USD_PLN), &clock);
    assert_eq!(store.snapshots(), [kept]);
}

#[test]
fn a_missing_oversized_or_corrupt_cache_file_is_empty() {
    let clock = Clock::new(NOW);
    let dir = tempfile::tempdir().unwrap();
    assert!(
        open(dir.path(), Mock::json(USD_PLN), &clock)
            .snapshots()
            .is_empty()
    );
    std::fs::write(dir.path().join(CACHE_FILE_NAME), "not json").unwrap();
    assert!(
        open(dir.path(), Mock::json(USD_PLN), &clock)
            .snapshots()
            .is_empty()
    );
    std::fs::write(dir.path().join(CACHE_FILE_NAME), vec![b' '; 70 * 1024]).unwrap();
    assert!(
        open(dir.path(), Mock::json(USD_PLN), &clock)
            .snapshots()
            .is_empty()
    );
}

#[test]
fn a_fetch_uses_the_pair_endpoint_and_saves_the_rate() {
    let dir = tempfile::tempdir().unwrap();
    let clock = Clock::new(NOW);
    let mock = Mock::json(USD_PLN);
    let store = open(dir.path(), mock.clone(), &clock);

    assert_eq!(
        store.refresh_if_needed("100 USD to PLN"),
        RefreshOutcome::Updated
    );
    assert_eq!(
        mock.requests(),
        [RateRequest {
            url: "https://api.frankfurter.dev/v2/rate/usd/pln".into(),
            accept: "application/json",
        }]
    );
    let saved = store.snapshot(&pair("USD", "PLN")).expect("saved");
    assert_eq!(saved.rate, 3.75);
    assert_eq!(saved.rate_date, at(1_790_899_200)); // 2026-10-02T00:00:00Z
    assert_eq!(saved.fetched_at, at(NOW));
    assert!(CalculatorEngine::currency_pair(&"x".repeat(513)).is_none());

    // Persisted: a new store (offline) knows the rate and converts with it.
    let offline = open(
        dir.path(),
        Mock::with(Err(FetchError::Network("offline".into()))),
        &clock,
    );
    let calculator = CalculatorEngine::default().with_reference_time(at(NOW));
    let CalculationOutcome::Result(result) =
        calculator.evaluate_with_rates("100 USD to PLN", &offline.snapshots())
    else {
        panic!("a saved rate converts offline");
    };
    assert_eq!(result.display, "375.00 PLN (rate dated Oct 2, 2026)");
    assert_eq!(result.clipboard, "375.00 PLN");
}

#[test]
fn custom_provider_base_urls_are_used_without_a_trailing_slash() {
    let dir = tempfile::tempdir().unwrap();
    let mock = Mock::json(USD_PLN);
    let store =
        open(dir.path(), mock.clone(), &Clock::new(NOW)).with_api_base("http://mirror.test/");
    store.refresh_pair(&pair("USD", "PLN"));
    assert_eq!(mock.requests()[0].url, "http://mirror.test/v2/rate/usd/pln");
}

#[test]
fn fresh_pairs_identical_pairs_and_ordinary_queries_make_no_request() {
    let dir = tempfile::tempdir().unwrap();
    let clock = Clock::new(NOW);
    let fresh = CurrencyRateSnapshot::new(pair("PLN", "USD"), 0.25, at(NOW), at(NOW - 3_600));
    write_cache(dir.path(), &[fresh]);
    let mock = Mock::json(USD_PLN);
    let store = open(dir.path(), mock.clone(), &clock);

    // The inverse of a fresh rate is fresh too.
    assert_eq!(
        store.refresh_if_needed("100 USD to PLN"),
        RefreshOutcome::Fresh
    );
    assert_eq!(
        store.refresh_if_needed("10 GBP to GBP"),
        RefreshOutcome::Fresh
    );
    assert_eq!(
        store.refresh_if_needed("search for USD"),
        RefreshOutcome::NotACurrencyQuery
    );
    assert_eq!(
        store.refresh_if_needed("100 USD to"),
        RefreshOutcome::NotACurrencyQuery
    );
    assert!(mock.requests().is_empty());

    // After 24 hours the rate is stale and is downloaded again.
    clock.advance(DAY + 1);
    assert_eq!(
        store.refresh_if_needed("100 USD to PLN"),
        RefreshOutcome::Updated
    );
    assert_eq!(mock.requests().len(), 1);
}

#[test]
fn a_failed_attempt_is_not_repeated_for_thirty_seconds() {
    let dir = tempfile::tempdir().unwrap();
    let clock = Clock::new(NOW);
    let mock = Mock::with(Err(FetchError::Network("offline".into())));
    let store = open(dir.path(), mock.clone(), &clock);
    assert_eq!(
        store.refresh_if_needed("100 USD to PLN"),
        RefreshOutcome::Failed
    );
    assert_eq!(
        store.refresh_if_needed("100 USD to PLN"),
        RefreshOutcome::Throttled
    );
    clock.advance(29);
    assert_eq!(
        store.refresh_if_needed("100 USD to PLN"),
        RefreshOutcome::Throttled
    );
    clock.advance(1);
    assert_eq!(
        store.refresh_if_needed("100 USD to PLN"),
        RefreshOutcome::Failed
    );
    assert_eq!(mock.requests().len(), 2);
    assert!(store.snapshots().is_empty());
    assert!(!store.is_refreshing(&pair("USD", "PLN")));
}

/// Blocks inside `fetch` until released, so a second caller can arrive meanwhile.
struct Gate {
    started: Mutex<Sender<()>>,
    release: Mutex<Receiver<()>>,
    requests: Mutex<usize>,
}

impl RateFetcher for Gate {
    fn fetch(&self, _request: &RateRequest) -> Result<RateResponse, FetchError> {
        *self.requests.lock().unwrap() += 1;
        self.started.lock().unwrap().send(()).unwrap();
        self.release.lock().unwrap().recv().unwrap();
        Ok(RateResponse {
            status: 200,
            body: USD_PLN.as_bytes().to_vec(),
            content_length: None,
        })
    }
}

#[test]
fn concurrent_requests_for_one_pair_are_coalesced() {
    let dir = tempfile::tempdir().unwrap();
    let clock = Clock::new(NOW);
    let (started_tx, started_rx) = channel();
    let (release_tx, release_rx) = channel();
    let gate = Arc::new(Gate {
        started: Mutex::new(started_tx),
        release: Mutex::new(release_rx),
        requests: Mutex::new(0),
    });
    let store = Arc::new(open(dir.path(), gate.clone(), &clock));

    let worker = {
        let store = store.clone();
        std::thread::spawn(move || store.refresh_if_needed("100 USD to PLN"))
    };
    started_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(store.is_refreshing(&pair("USD", "PLN")));
    // Same instant: throttled. Later than the retry delay: still in flight, never a second request.
    assert_eq!(
        store.refresh_if_needed("100 USD to PLN"),
        RefreshOutcome::Throttled
    );
    clock.advance(31);
    assert_eq!(
        store.refresh_if_needed("100 USD to PLN"),
        RefreshOutcome::InFlight
    );
    release_tx.send(()).unwrap();
    assert_eq!(worker.join().unwrap(), RefreshOutcome::Updated);
    assert_eq!(*gate.requests.lock().unwrap(), 1);
    assert!(!store.is_refreshing(&pair("USD", "PLN")));
    assert_eq!(
        store.snapshot(&pair("USD", "PLN")).map(|s| s.rate),
        Some(3.75)
    );
}

#[test]
fn invalid_responses_keep_the_stale_cache_available_offline() {
    let pln = pair("USD", "PLN");
    let cached = CurrencyRateSnapshot::new(pln.clone(), 3.5, at(NOW - 2 * DAY), at(NOW - 2 * DAY));
    let big = "x".repeat(32 * 1024 + 1);
    let bad_responses: Vec<(&str, Result<RateResponse, FetchError>)> = vec![
        (
            "rate zero",
            Mock::json(r#"{"date":"2026-10-02","base":"USD","quote":"PLN","rate":0}"#)
                .response
                .clone(),
        ),
        (
            "negative rate",
            Mock::json(r#"{"date":"2026-10-02","base":"USD","quote":"PLN","rate":-1}"#)
                .response
                .clone(),
        ),
        (
            "wrong pair",
            Mock::json(r#"{"date":"2026-10-02","base":"USD","quote":"EUR","rate":3.7}"#)
                .response
                .clone(),
        ),
        (
            "bad date",
            Mock::json(r#"{"date":"2026-13-02","base":"USD","quote":"PLN","rate":3.7}"#)
                .response
                .clone(),
        ),
        (
            "date more than a day ahead",
            Mock::json(r#"{"date":"2026-10-20","base":"USD","quote":"PLN","rate":3.7}"#)
                .response
                .clone(),
        ),
        ("not json", Mock::json("<html>").response.clone()),
        (
            "server error",
            Ok(RateResponse {
                status: 500,
                body: USD_PLN.as_bytes().to_vec(),
                content_length: None,
            }),
        ),
        (
            "oversized body",
            Ok(RateResponse {
                status: 200,
                body: big.clone().into_bytes(),
                content_length: None,
            }),
        ),
        (
            "oversized announced length",
            Ok(RateResponse {
                status: 200,
                body: USD_PLN.as_bytes().to_vec(),
                content_length: Some(32 * 1024 + 1),
            }),
        ),
        ("network error", Err(FetchError::Network("down".into()))),
    ];
    for (name, response) in bad_responses {
        let dir = tempfile::tempdir().unwrap();
        let clock = Clock::new(NOW);
        write_cache(dir.path(), std::slice::from_ref(&cached));
        let store = open(dir.path(), Mock::with(response), &clock);
        assert_eq!(store.refresh_pair(&pln), RefreshOutcome::Failed, "{name}");
        assert_eq!(store.snapshots(), std::slice::from_ref(&cached), "{name}");

        let calculator = CalculatorEngine::default().with_reference_time(at(NOW));
        let CalculationOutcome::Result(result) =
            calculator.evaluate_with_rates("100 USD to PLN", &store.snapshots())
        else {
            panic!("{name}: the cached rate stays available");
        };
        assert!(
            result.display.contains("cached"),
            "{name}: {}",
            result.display
        );
    }

    // A response too large for an empty cache leaves it empty.
    let dir = tempfile::tempdir().unwrap();
    let store = open(
        dir.path(),
        Mock::with(Ok(RateResponse {
            status: 200,
            body: big.into_bytes(),
            content_length: None,
        })),
        &Clock::new(NOW),
    );
    store.refresh_pair(&pln);
    assert!(store.snapshots().is_empty());
}

// ----- the real HTTP client against a local server -----

/// Serves one canned HTTP response per connection; returns the base URL and the received
/// request lines.
fn serve(responses: Vec<String>) -> (String, std::thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let handle = std::thread::spawn(move || {
        let mut received = Vec::new();
        for response in responses {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0u8; 4096];
            let read = stream.read(&mut buffer).unwrap();
            received.push(String::from_utf8_lossy(&buffer[..read]).into_owned());
            stream.write_all(response.as_bytes()).unwrap();
        }
        received
    });
    (base, handle)
}

fn http(status: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

#[test]
fn the_ureq_fetcher_sends_the_accept_header_and_limits_the_body() {
    let big = "x".repeat(40 * 1024);
    let (base, server) = serve(vec![
        http("200 OK", USD_PLN),
        http("500 Internal Server Error", "{}"),
        http("200 OK", &big),
    ]);
    let fetcher = UreqFetcher::allowing_plain_http();
    let request = |path: &str| RateRequest {
        url: format!("{base}{path}"),
        accept: "application/json",
    };

    let ok = fetcher.fetch(&request("/v2/rate/usd/pln")).unwrap();
    assert_eq!((ok.status, ok.body.as_slice()), (200, USD_PLN.as_bytes()));
    assert_eq!(ok.content_length, Some(USD_PLN.len() as u64));

    let failed = fetcher.fetch(&request("/v2/rate/usd/eur")).unwrap();
    assert_eq!(failed.status, 500);

    let oversized = fetcher.fetch(&request("/v2/rate/usd/gbp")).unwrap();
    assert_eq!(
        oversized.body.len(),
        32 * 1024 + 1,
        "reported as just over the limit"
    );

    let received = server.join().unwrap();
    assert!(
        received[0].starts_with("GET /v2/rate/usd/pln HTTP/1.1"),
        "{}",
        received[0]
    );
    assert!(
        received[0]
            .to_lowercase()
            .contains("accept: application/json"),
        "{}",
        received[0]
    );

    // A closed port is a network error, not a panic or a hang.
    let closed = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/", closed.local_addr().unwrap());
    drop(closed);
    assert!(matches!(
        fetcher.fetch(&RateRequest {
            url,
            accept: "application/json"
        }),
        Err(FetchError::Network(_))
    ));
    // The production fetcher refuses plain HTTP.
    let (base, _server) = serve(vec![]);
    assert!(
        UreqFetcher::new()
            .fetch(&RateRequest {
                url: format!("{base}/"),
                accept: "application/json"
            })
            .is_err()
    );
}
