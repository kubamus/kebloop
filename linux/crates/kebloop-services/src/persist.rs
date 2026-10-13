//! Debounced background persistence.
//!
//! Stores that change often (clipboard history, usage counters) hand their bytes to a
//! [`DebouncedWriter`] instead of rewriting and fsyncing the file on every change. Pending writes
//! to the same file are coalesced and written by one worker thread after the changes stop for
//! [`DebouncedWriter::DEFAULT_DELAY`] (but at most [`DebouncedWriter::DEFAULT_MAX_DELAY`] after the
//! first one). [`DebouncedWriter::flush`] writes everything synchronously (used on quit) and
//! dropping the writer flushes too.
//!
//! All file writes go through one lock, and the pending data is taken only after that lock was
//! acquired, so two writers never run at the same time and an older snapshot can never overwrite a
//! newer one.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::error::ConfigError;
use crate::util::atomic_write;

/// Produces the bytes of a pending write; runs on the worker thread (or inside `flush`).
pub type Producer = Box<dyn FnOnce() -> Result<Vec<u8>, ConfigError> + Send>;

struct Pending {
    produce: Producer,
    mode: u32,
}

#[derive(Default)]
struct State {
    pending: BTreeMap<PathBuf, Pending>,
    first_dirty: Option<Instant>,
    last_change: Option<Instant>,
    closed: bool,
    last_error: Option<String>,
    writes: u64,
}

struct Shared {
    state: Mutex<State>,
    wake: Condvar,
    /// Held for the whole duration of every file write.
    write_lock: Mutex<()>,
    delay: Duration,
    max_delay: Duration,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Shared {
    /// Write everything that is pending. Returns the first error.
    fn write_pending(&self) -> Result<(), ConfigError> {
        let _writing = lock(&self.write_lock);
        let batch = {
            let mut state = lock(&self.state);
            state.first_dirty = None;
            state.last_change = None;
            std::mem::take(&mut state.pending)
        };
        let mut first_error = None;
        for (path, pending) in batch {
            let outcome =
                (pending.produce)().and_then(|bytes| atomic_write(&path, &bytes, pending.mode));
            let mut state = lock(&self.state);
            match outcome {
                Ok(()) => {
                    state.writes += 1;
                    state.last_error = None;
                }
                Err(error) => {
                    state.last_error = Some(error.to_string());
                    first_error.get_or_insert(error);
                }
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    fn run(&self) {
        loop {
            let mut state = lock(&self.state);
            loop {
                if state.pending.is_empty() {
                    if state.closed {
                        return;
                    }
                    state = self
                        .wake
                        .wait(state)
                        .unwrap_or_else(PoisonError::into_inner);
                    continue;
                }
                let now = Instant::now();
                let due = match (state.last_change, state.first_dirty) {
                    (Some(last), Some(first)) => (last + self.delay).min(first + self.max_delay),
                    _ => now,
                };
                if state.closed || now >= due {
                    break;
                }
                state = self
                    .wake
                    .wait_timeout(state, due - now)
                    .unwrap_or_else(PoisonError::into_inner)
                    .0;
            }
            drop(state);
            if let Err(error) = self.write_pending() {
                eprintln!("kebloop: background save failed: {error}");
            }
        }
    }
}

/// Coalescing background file writer. See the module documentation.
pub struct DebouncedWriter {
    shared: Arc<Shared>,
    worker: Mutex<Option<JoinHandle<()>>>,
    threaded: bool,
}

impl DebouncedWriter {
    /// Quiet time after the last change before a write starts.
    pub const DEFAULT_DELAY: Duration = Duration::from_millis(500);
    /// Longest a change may wait while more changes keep arriving.
    pub const DEFAULT_MAX_DELAY: Duration = Duration::from_secs(3);

    /// Writer with the default timing.
    pub fn new() -> Self {
        Self::with_timing(Self::DEFAULT_DELAY, Self::DEFAULT_MAX_DELAY)
    }

    /// Writer with explicit timing (tests).
    pub fn with_timing(delay: Duration, max_delay: Duration) -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            wake: Condvar::new(),
            write_lock: Mutex::new(()),
            delay,
            max_delay,
        });
        let worker_shared = shared.clone();
        let spawned = std::thread::Builder::new()
            .name("kebloop-persist".into())
            .spawn(move || worker_shared.run());
        let (worker, threaded) = match spawned {
            Ok(handle) => (Some(handle), true),
            Err(_) => (None, false),
        };
        Self {
            shared,
            worker: Mutex::new(worker),
            threaded,
        }
    }

    /// Queue a write of `path`, replacing a still-pending write of the same file. `produce` runs
    /// on the worker thread.
    pub fn schedule(&self, path: &Path, mode: u32, produce: Producer) {
        {
            let mut state = lock(&self.shared.state);
            let now = Instant::now();
            state
                .pending
                .insert(path.to_path_buf(), Pending { produce, mode });
            state.first_dirty.get_or_insert(now);
            state.last_change = Some(now);
        }
        if self.threaded {
            self.shared.wake.notify_all();
        } else if let Err(error) = self.shared.write_pending() {
            eprintln!("kebloop: save failed: {error}");
        }
    }

    /// Write `bytes` to `path` right now (serialised with background writes) and drop any older
    /// pending write of the same file.
    pub fn write_now(&self, path: &Path, bytes: &[u8], mode: u32) -> Result<(), ConfigError> {
        let _writing = lock(&self.shared.write_lock);
        lock(&self.shared.state).pending.remove(path);
        atomic_write(path, bytes, mode)
    }

    /// Write everything pending, synchronously.
    pub fn flush(&self) -> Result<(), ConfigError> {
        self.shared.write_pending()
    }

    /// Whether a write is waiting for its debounce delay.
    pub fn has_pending(&self) -> bool {
        !lock(&self.shared.state).pending.is_empty()
    }

    /// Number of completed file writes (diagnostics and tests).
    pub fn completed_writes(&self) -> u64 {
        lock(&self.shared.state).writes
    }

    /// Message of the last failed background write, if the latest attempt failed.
    pub fn last_error(&self) -> Option<String> {
        lock(&self.shared.state).last_error.clone()
    }
}

impl Default for DebouncedWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for DebouncedWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DebouncedWriter")
            .field("pending", &self.has_pending())
            .finish()
    }
}

impl Drop for DebouncedWriter {
    fn drop(&mut self) {
        lock(&self.shared.state).closed = true;
        self.shared.wake.notify_all();
        if let Some(handle) = lock(&self.worker).take() {
            let _ = handle.join();
        }
        let _ = self.shared.write_pending();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn bytes(text: &'static str) -> Producer {
        Box::new(move || Ok(text.as_bytes().to_vec()))
    }

    fn wait_for(condition: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !condition() {
            assert!(Instant::now() < deadline, "condition not reached");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn rapid_changes_coalesce_into_one_write_after_the_delay() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("history.json");
        let writer =
            DebouncedWriter::with_timing(Duration::from_millis(80), Duration::from_secs(5));
        let produced = Arc::new(AtomicUsize::new(0));
        for round in 0..20 {
            let produced = produced.clone();
            writer.schedule(
                &file,
                0o600,
                Box::new(move || {
                    produced.fetch_add(1, Ordering::SeqCst);
                    Ok(format!("v{round}").into_bytes())
                }),
            );
        }
        assert!(!file.exists(), "nothing is written before the delay");
        wait_for(|| file.exists());
        wait_for(|| !writer.has_pending());
        assert_eq!(fs::read(&file).unwrap(), b"v19");
        assert_eq!(writer.completed_writes(), 1);
        assert_eq!(
            produced.load(Ordering::SeqCst),
            1,
            "replaced snapshots are never serialised"
        );
    }

    #[test]
    fn flush_writes_synchronously_and_dropping_flushes() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a.json"), dir.path().join("b.json"));
        let writer = DebouncedWriter::with_timing(Duration::from_secs(60), Duration::from_secs(60));
        writer.schedule(&a, 0o600, bytes("one"));
        assert!(!a.exists());
        writer.flush().unwrap();
        assert_eq!(fs::read(&a).unwrap(), b"one");
        assert!(!writer.has_pending());
        writer.schedule(&b, 0o600, bytes("two"));
        drop(writer);
        assert_eq!(fs::read(&b).unwrap(), b"two");
    }

    #[test]
    fn write_now_discards_older_pending_data_and_never_regresses() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("config.json");
        let writer =
            DebouncedWriter::with_timing(Duration::from_millis(30), Duration::from_secs(5));
        writer.schedule(&file, 0o600, bytes("old"));
        writer.write_now(&file, b"new", 0o600).unwrap();
        std::thread::sleep(Duration::from_millis(150));
        writer.flush().unwrap();
        assert_eq!(fs::read(&file).unwrap(), b"new");
    }

    #[test]
    fn writers_are_serialised_under_contention() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("race.json");
        let writer = Arc::new(DebouncedWriter::with_timing(
            Duration::from_millis(1),
            Duration::from_millis(5),
        ));
        let running = Arc::new(AtomicUsize::new(0));
        let overlap = Arc::new(AtomicUsize::new(0));
        let threads: Vec<_> = (0..4)
            .map(|id| {
                let (writer, file) = (writer.clone(), file.clone());
                let (running, overlap) = (running.clone(), overlap.clone());
                std::thread::spawn(move || {
                    for round in 0..50 {
                        let (running, overlap) = (running.clone(), overlap.clone());
                        writer.schedule(
                            &file,
                            0o600,
                            Box::new(move || {
                                if running.fetch_add(1, Ordering::SeqCst) != 0 {
                                    overlap.fetch_add(1, Ordering::SeqCst);
                                }
                                std::thread::sleep(Duration::from_micros(200));
                                running.fetch_sub(1, Ordering::SeqCst);
                                Ok(format!("{id}-{round}").into_bytes())
                            }),
                        );
                        if round % 10 == 0 {
                            let _ = writer.flush();
                        }
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        writer.flush().unwrap();
        assert_eq!(overlap.load(Ordering::SeqCst), 0, "two writers overlapped");
        assert!(fs::read(&file).unwrap().contains(&b'-'));
        assert_eq!(
            fs::read_dir(dir.path()).unwrap().count(),
            1,
            "no temporary files left"
        );
    }

    #[test]
    fn failed_writes_are_reported_by_flush() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("file");
        fs::write(&blocker, "x").unwrap();
        let writer = DebouncedWriter::with_timing(Duration::from_secs(60), Duration::from_secs(60));
        writer.schedule(&blocker.join("child.json"), 0o600, bytes("data"));
        assert!(writer.flush().is_err());
        assert!(writer.last_error().is_some());
    }
}
