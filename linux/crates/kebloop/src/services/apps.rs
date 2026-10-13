//! Application index: cached list first, then background rescans (startup, each launcher
//! opening when stale, every five minutes, inotify changes, manual refresh).

use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::glib;
use kebloop_platform::apps::{
    self, AppWatcher, DesktopApp, REFRESH_INTERVAL, ScanConfig, application_dirs,
};
use kebloop_platform::paths::Env;

use crate::context::{AppContext, Change};

/// Bookkeeping for the single-flight rescans.
#[derive(Default)]
pub struct ScanState {
    running: bool,
    pending: bool,
    last_completed: Option<Instant>,
    sender: Option<async_channel::Sender<Vec<DesktopApp>>>,
}

/// Load the cache and start the background machinery.
pub fn init(ctx: &Rc<AppContext>) {
    let cached = apps::load_cache(&ctx.xdg.application_index_cache());
    if !cached.is_empty() {
        *ctx.apps.borrow_mut() = cached;
    }
    let (sender, receiver) = async_channel::unbounded::<Vec<DesktopApp>>();
    ctx.scan.borrow_mut().sender = Some(sender);

    let receiving = ctx.clone();
    glib::spawn_future_local(async move {
        while let Ok(scanned) = receiver.recv().await {
            let rescan = {
                let mut scan = receiving.scan.borrow_mut();
                scan.running = false;
                scan.last_completed = Some(Instant::now());
                std::mem::take(&mut scan.pending)
            };
            let changed = *receiving.apps.borrow() != scanned;
            if changed {
                *receiving.apps.borrow_mut() = scanned;
                receiving.emit(Change::Apps);
            }
            if rescan {
                refresh(&receiving);
            }
        }
    });

    match AppWatcher::start(&application_dirs(&ctx.xdg), Duration::from_millis(600)) {
        Ok((watcher, changes)) => {
            ctx.keep_alive(watcher);
            let watching = ctx.clone();
            glib::spawn_future_local(async move {
                while changes.recv().await.is_ok() {
                    refresh(&watching);
                }
            });
        }
        Err(error) => eprintln!("kebloop: application watcher unavailable: {error}"),
    }

    let periodic = ctx.clone();
    glib::timeout_add_seconds_local(
        u32::try_from(REFRESH_INTERVAL.as_secs()).unwrap_or(300),
        move || {
            refresh(&periodic);
            glib::ControlFlow::Continue
        },
    );
    refresh(ctx);
}

/// Rescan on a worker thread. Overlapping requests collapse into one follow-up scan.
pub fn refresh(ctx: &Rc<AppContext>) {
    let sender = {
        let mut scan = ctx.scan.borrow_mut();
        if scan.running {
            scan.pending = true;
            return;
        }
        let Some(sender) = scan.sender.clone() else {
            return;
        };
        scan.running = true;
        sender
    };
    let cache_path = ctx.xdg.application_index_cache();
    let spawned = std::thread::Builder::new()
        .name("kebloop-app-scan".into())
        .spawn(move || {
            let config = ScanConfig::from_env(&Env::from_process());
            let scanned = apps::scan_applications(&config);
            if let Err(error) = apps::save_cache(&cache_path, &scanned) {
                eprintln!("kebloop: could not write the application cache: {error}");
            }
            let _ = sender.send_blocking(scanned);
        });
    if let Err(error) = spawned {
        eprintln!("kebloop: could not start the application scan: {error}");
        ctx.scan.borrow_mut().running = false;
    }
}

/// Rescan unless one finished less than `max_age` ago (called when the launcher opens).
pub fn refresh_if_stale(ctx: &Rc<AppContext>, max_age: Duration) {
    let stale = ctx
        .scan
        .borrow()
        .last_completed
        .is_none_or(|finished| finished.elapsed() >= max_age);
    if stale {
        refresh(ctx);
    }
}
