//! Clipboard wiring: the Wayland watcher feeds the history store; copying goes through the
//! watcher (works without focus) and falls back to the GDK clipboard.

use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, SystemTime};

use gtk::glib;
use gtk::prelude::*;
use kebloop_platform::clipboard::{ClipboardConfig, ClipboardEvent, ClipboardWatcher};
use kebloop_services::clipboard::{ClipboardPayload, RecordOutcome};

use crate::context::{AppContext, Change, ClipboardStatus};

type Started = Result<
    (
        Arc<ClipboardWatcher>,
        async_channel::Receiver<ClipboardEvent>,
    ),
    String,
>;

/// Whether history is recording right now; the value shared with the watcher so that it never
/// reads clipboard content while history is off or paused.
fn sync_recording(ctx: &AppContext) -> bool {
    let recording = ctx.clipboard.borrow().is_recording(SystemTime::now());
    ctx.clipboard_recording.swap(recording, Ordering::Relaxed) != recording
}

/// Keep the watcher's recording flag in step with the store: on every clipboard-related change
/// and when a pause runs out.
fn keep_recording_flag_current(ctx: &Rc<AppContext>) {
    sync_recording(ctx);
    let weak = Rc::downgrade(ctx);
    ctx.subscribe(Rc::new(move |change| {
        if change != Change::Clipboard {
            return;
        }
        let Some(ctx) = weak.upgrade() else {
            return;
        };
        sync_recording(&ctx);
        schedule_pause_expiry(&ctx);
    }));
    schedule_pause_expiry(ctx);
}

/// When recording is paused, re-evaluate the flag the moment the pause ends.
fn schedule_pause_expiry(ctx: &Rc<AppContext>) {
    let now = SystemTime::now();
    let Some(deadline) = ctx
        .clipboard
        .borrow()
        .pause_deadline()
        .filter(|deadline| *deadline > now)
    else {
        return;
    };
    let wait = deadline
        .duration_since(now)
        .unwrap_or_default()
        .saturating_add(Duration::from_millis(100));
    let weak = Rc::downgrade(ctx);
    glib::timeout_add_local_once(wait, move || {
        if let Some(ctx) = weak.upgrade()
            && sync_recording(&ctx)
        {
            ctx.emit(Change::Clipboard);
        }
    });
}

/// Start the watcher on a worker thread; history is fed from the main loop.
pub fn init(ctx: &Rc<AppContext>) {
    if ctx.clipboard.borrow().unavailable_reason().is_some() {
        // The store could not be opened: nothing to feed, and nothing may be read.
        return;
    }
    keep_recording_flag_current(ctx);
    let (sender, receiver) = async_channel::bounded::<Started>(1);
    let config = ClipboardConfig {
        recording: ctx.clipboard_recording.clone(),
        revision: ctx.clipboard_revision.clone(),
        ..ClipboardConfig::default()
    };
    let spawned = std::thread::Builder::new()
        .name("kebloop-clipboard-start".into())
        .spawn(move || {
            let started = ClipboardWatcher::start(config)
                .map(|(watcher, events)| (Arc::new(watcher), events))
                .map_err(|error| error.to_string());
            let _ = sender.send_blocking(started);
        });
    if let Err(error) = spawned {
        *ctx.clipboard_status.borrow_mut() = ClipboardStatus::Unavailable(error.to_string());
        return;
    }
    let ctx = ctx.clone();
    glib::spawn_future_local(async move {
        match receiver.recv().await {
            Ok(Ok((watcher, events))) => {
                *ctx.clipboard_watcher.borrow_mut() = Some(watcher);
                *ctx.clipboard_status.borrow_mut() = ClipboardStatus::Active;
                ctx.emit(Change::Clipboard);
                while let Ok(event) = events.recv().await {
                    record(&ctx, event);
                }
                *ctx.clipboard_status.borrow_mut() =
                    ClipboardStatus::Unavailable("the clipboard watcher stopped".into());
                ctx.emit(Change::Clipboard);
            }
            Ok(Err(reason)) => {
                *ctx.clipboard_status.borrow_mut() = ClipboardStatus::Unavailable(reason);
                ctx.emit(Change::Clipboard);
            }
            Err(_) => {}
        }
    });
}

/// Expire pauses and apply retention once a minute.
pub fn start_maintenance(ctx: &Rc<AppContext>) {
    let ctx = ctx.clone();
    glib::timeout_add_seconds_local(60, move || {
        let paused_before = ctx.clipboard.borrow().is_paused(SystemTime::now());
        if let Err(error) = ctx.clipboard.borrow_mut().tick(SystemTime::now()) {
            eprintln!("kebloop: clipboard maintenance failed: {error}");
        }
        let flag_changed = sync_recording(&ctx);
        if flag_changed || paused_before != ctx.clipboard.borrow().is_paused(SystemTime::now()) {
            ctx.emit(Change::Clipboard);
        }
        glib::ControlFlow::Continue
    });
}

fn record(ctx: &AppContext, event: ClipboardEvent) {
    if event.sensitive {
        return;
    }
    let source = event.source_app.as_deref().map(|source| {
        let apps = ctx.apps.borrow();
        let mut matches = apps
            .iter()
            .filter(|app| crate::platform::identity_of(app).matches_app_id(source));
        match (matches.next(), matches.next()) {
            (Some(app), None) => app.id.clone(),
            _ => source.to_owned(),
        }
    });
    // Accept both raw owner identities and their unambiguous desktop-file ids.
    if event
        .source_app
        .as_ref()
        .is_some_and(|source| ctx.clipboard.borrow().excluded_sources().contains(source))
    {
        return;
    }
    let outcome =
        ctx.clipboard
            .borrow_mut()
            .record(&event.data, source.as_deref(), SystemTime::now());
    match outcome {
        Ok(RecordOutcome::Recorded(_)) => ctx.emit(Change::Clipboard),
        Ok(RecordOutcome::Ignored(_)) => {}
        Err(error) => eprintln!("kebloop: could not save clipboard history: {error}"),
    }
}

/// Put plain text on the clipboard. `transient` asks other managers (and our own history) to
/// skip it. Returns whether some backend accepted it.
pub fn copy_text(ctx: &AppContext, text: &str, transient: bool) -> bool {
    let mut formats = BTreeMap::new();
    formats.insert(
        kebloop_services::clipboard::TEXT_OUTPUT_TYPE.to_owned(),
        text.as_bytes().to_vec(),
    );
    if set_through_watcher(ctx, formats, transient) {
        return true;
    }
    set_text_through_gdk(text)
}

/// Put a stored history entry back on the clipboard with all its formats.
pub fn copy_payload(ctx: &AppContext, payload: &ClipboardPayload, transient: bool) -> bool {
    let formats: BTreeMap<String, Vec<u8>> = payload.formats.iter().cloned().collect();
    if formats.is_empty() {
        return false;
    }
    if set_through_watcher(ctx, formats.clone(), transient) {
        return true;
    }
    let text = formats
        .get(kebloop_services::clipboard::TEXT_OUTPUT_TYPE)
        .map(|bytes| String::from_utf8_lossy(bytes).into_owned());
    match text {
        Some(text) => set_text_through_gdk(&text),
        None => false,
    }
}

/// Hand the formats to the watcher thread without waiting: the Wayland thread owns the
/// connection and may be busy, and the UI must never block on it. Returns whether the command was
/// queued; if the thread later reports a failure the text (when there is some) is put on the GDK
/// clipboard instead.
fn set_through_watcher(
    ctx: &AppContext,
    formats: BTreeMap<String, Vec<u8>>,
    transient: bool,
) -> bool {
    let Some(watcher) = ctx.clipboard_watcher.borrow().clone() else {
        return false;
    };
    let fallback_text = formats
        .get(kebloop_services::clipboard::TEXT_OUTPUT_TYPE)
        .map(|bytes| String::from_utf8_lossy(bytes).into_owned());
    let (done, outcome) = async_channel::bounded::<Result<(), String>>(1);
    let queued = watcher.set_clipboard_async(formats, transient, move |result| {
        let _ = done.send_blocking(result.map_err(|error| error.to_string()));
    });
    if let Err(error) = queued {
        eprintln!("kebloop: clipboard write failed: {error}");
        return false;
    }
    glib::spawn_future_local(async move {
        let failure = match outcome.recv().await {
            Ok(Ok(())) => return,
            Ok(Err(error)) => error,
            Err(_) => "the clipboard thread stopped".to_owned(),
        };
        eprintln!("kebloop: clipboard write failed: {failure}");
        if let Some(text) = fallback_text {
            set_text_through_gdk(&text);
        }
    });
    true
}

fn set_text_through_gdk(text: &str) -> bool {
    match gtk::gdk::Display::default() {
        Some(display) => {
            display.clipboard().set_text(text);
            true
        }
        None => false,
    }
}
