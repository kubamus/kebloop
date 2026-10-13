//! Cached "does this file still exist" answers for pinned files. The root list is rebuilt on every
//! keystroke on the GTK thread, and a `stat` on a pinned path can block for seconds on a dead
//! network mount, so lookups only read memory: an unknown path is assumed to exist, a worker
//! thread checks it and tells the launcher when an answer differs, and answers older than the
//! time-to-live are re-checked the next time they are used.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

/// How long an answer is trusted before it is checked again.
pub const EXISTS_TTL: Duration = Duration::from_secs(30);

#[derive(Default)]
struct State {
    known: HashMap<String, (bool, Instant)>,
    queued: HashSet<String>,
    queue: Vec<String>,
}

/// Existence answers shared between the launcher (reads) and the checker thread (writes).
pub struct ExistsCache {
    state: Mutex<State>,
    ttl: Duration,
}

impl ExistsCache {
    /// Cache with the given time-to-live.
    pub fn new(ttl: Duration) -> Self {
        Self {
            state: Mutex::new(State::default()),
            ttl,
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The last known answer (`true` while unknown). Never touches the file system; an unknown or
    /// outdated answer is queued for the checker. Returns `(exists, newly_queued)`.
    pub fn lookup(&self, path: &str, now: Instant) -> (bool, bool) {
        let mut state = self.lock();
        let (exists, fresh) = match state.known.get(path) {
            Some((exists, at)) => (*exists, now.saturating_duration_since(*at) < self.ttl),
            None => (true, false),
        };
        let mut queued = false;
        if !fresh && state.queued.insert(path.to_owned()) {
            state.queue.push(path.to_owned());
            queued = true;
        }
        (exists, queued)
    }

    /// Paths waiting for a check.
    pub fn take_queue(&self) -> Vec<String> {
        std::mem::take(&mut self.lock().queue)
    }

    /// Record a checked answer. Returns whether it differs from what lookups returned before.
    pub fn store(&self, path: &str, exists: bool, now: Instant) -> bool {
        let mut state = self.lock();
        state.queued.remove(path);
        let before = state.known.insert(path.to_owned(), (exists, now));
        before.is_none_or(|(old, _)| old) != exists
    }
}

/// Starts the checker thread. `kick` wakes it when new paths were queued; `changed` receives a
/// message whenever an answer flipped so the list can be rebuilt.
pub fn spawn_checker(
    cache: Arc<ExistsCache>,
    kick: async_channel::Receiver<()>,
    changed: async_channel::Sender<()>,
    exists: impl Fn(&Path) -> bool + Send + 'static,
) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name("kebloop-exists".into())
        .spawn(move || {
            while kick.recv_blocking().is_ok() {
                let mut flipped = false;
                for path in cache.take_queue() {
                    let answer = exists(Path::new(&path));
                    flipped |= cache.store(&path, answer, Instant::now());
                }
                if flipped && changed.try_send(()).is_err() && changed.is_closed() {
                    break;
                }
            }
        })
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_paths_are_assumed_to_exist_and_queued_once() {
        let cache = ExistsCache::new(Duration::from_secs(30));
        let now = Instant::now();
        assert_eq!(cache.lookup("/a", now), (true, true));
        assert_eq!(cache.lookup("/a", now), (true, false), "already queued");
        assert_eq!(cache.take_queue(), ["/a"]);
        assert!(cache.take_queue().is_empty());
    }

    #[test]
    fn answers_are_cached_until_they_expire_and_flips_are_reported() {
        let cache = ExistsCache::new(Duration::from_secs(30));
        let t0 = Instant::now();
        cache.lookup("/gone", t0);
        assert!(cache.store("/gone", false, t0), "assumed true, found false");
        assert!(!cache.store("/gone", false, t0), "no change");
        assert_eq!(
            cache.lookup("/gone", t0 + Duration::from_secs(5)),
            (false, false)
        );
        // After the time-to-live the old answer is still returned, but a re-check is queued.
        let later = t0 + Duration::from_secs(31);
        let _ = cache.take_queue();
        assert_eq!(cache.lookup("/gone", later), (false, true));
        assert_eq!(cache.take_queue(), ["/gone"]);
        assert!(cache.store("/gone", true, later), "the file came back");
    }

    #[test]
    fn lookups_never_hit_the_file_system_and_the_checker_reports_changes() {
        let dir = tempfile::tempdir().unwrap();
        let present = dir.path().join("present.txt");
        let missing = dir.path().join("missing.txt");
        std::fs::write(&present, "x").unwrap();
        let cache = Arc::new(ExistsCache::new(Duration::from_secs(30)));
        let (kick_tx, kick_rx) = async_channel::bounded(1);
        let (changed_tx, changed_rx) = async_channel::bounded(1);
        spawn_checker(cache.clone(), kick_rx, changed_tx, |path| path.exists()).unwrap();

        let (present_str, missing_str) = (
            present.to_string_lossy().into_owned(),
            missing.to_string_lossy().into_owned(),
        );
        let now = Instant::now();
        assert!(cache.lookup(&present_str, now).0);
        assert!(cache.lookup(&missing_str, now).0, "assumed until checked");
        kick_tx.try_send(()).unwrap();
        changed_rx.recv_blocking().unwrap();
        assert!(cache.lookup(&present_str, now).0);
        assert!(
            !cache.lookup(&missing_str, now).0,
            "the worker found it missing"
        );
    }
}
