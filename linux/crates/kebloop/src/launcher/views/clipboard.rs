//! Clipboard History helpers: state messages, copy/remove/pin operations and lazy thumbnails.

use std::rc::Rc;
use std::time::SystemTime;

use gtk::prelude::*;
use gtk::{gdk, glib};
use kebloop_services::clipboard::ClipboardPayload;

use crate::context::{AppContext, Change, ClipboardStatus};
use crate::i18n::tr;
use crate::services;

/// What to say when the list is empty (or as a note above a non-empty one).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipboardNote {
    /// The compositor offers no clipboard protocol.
    Unavailable(String),
    /// The history store could not be opened; history is off for this run.
    StoreUnavailable(String),
    /// Recording is off in Settings.
    Disabled,
    /// Recording is paused for a while.
    Paused,
    /// Recording, nothing stored yet.
    Empty,
    /// Recording, but the filter matches nothing.
    NoMatch,
}

/// Decide the note from the backend and store state.
pub fn note_for(
    status: &ClipboardStatus,
    enabled: bool,
    paused: bool,
    stored: usize,
    shown: usize,
) -> Option<ClipboardNote> {
    match status {
        ClipboardStatus::HistoryDisabled(reason) => {
            return Some(ClipboardNote::StoreUnavailable(reason.clone()));
        }
        ClipboardStatus::Unavailable(reason) => {
            return Some(ClipboardNote::Unavailable(reason.clone()));
        }
        ClipboardStatus::Starting | ClipboardStatus::Active => {}
    }
    if !enabled {
        return Some(ClipboardNote::Disabled);
    }
    if paused {
        return Some(ClipboardNote::Paused);
    }
    if shown == 0 {
        return Some(if stored == 0 {
            ClipboardNote::Empty
        } else {
            ClipboardNote::NoMatch
        });
    }
    None
}

/// Localized text of a note.
pub fn note_text(note: &ClipboardNote) -> String {
    match note {
        ClipboardNote::Unavailable(_) => tr("Clipboard history is unavailable on this compositor."),
        ClipboardNote::StoreUnavailable(_) => tr(
            "Clipboard history could not be opened and is turned off in this session. See Help for details.",
        ),
        ClipboardNote::Disabled => tr("Clipboard history is turned off in Settings."),
        ClipboardNote::Paused => tr("Clipboard recording is paused."),
        ClipboardNote::Empty => tr("Copy some text to start your history."),
        ClipboardNote::NoMatch => tr("No matching clipboard items"),
    }
}

/// The note for the current state of `ctx` with `shown` rows listed.
pub fn current_note(ctx: &AppContext, shown: usize) -> Option<ClipboardNote> {
    let now = SystemTime::now();
    let store = ctx.clipboard.borrow();
    note_for(
        &ctx.clipboard_status.borrow(),
        store.is_enabled(),
        store.is_paused(now),
        store.items().len(),
        shown,
    )
}

/// Put the entry back on the clipboard. Returns whether it worked.
pub fn copy_entry(ctx: &AppContext, entry_id: &str) -> Result<(), String> {
    let payload: Option<ClipboardPayload> = ctx
        .clipboard
        .borrow_mut()
        .payload_for(entry_id)
        .map_err(|error| error.to_string())?;
    let Some(payload) = payload else {
        return Err(tr("This item no longer exists."));
    };
    if services::clipboard::copy_payload(ctx, &payload, false) {
        Ok(())
    } else {
        Err(tr("Could not write to the clipboard."))
    }
}

/// Delete one entry.
pub fn remove_entry(ctx: &AppContext, entry_id: &str) {
    if let Err(error) = ctx.clipboard.borrow_mut().remove(entry_id) {
        eprintln!("kebloop: could not remove the clipboard item: {error}");
    }
    ctx.emit(Change::Clipboard);
}

/// Pin or unpin. Returns `false` when the pin limit was reached.
pub fn set_pinned(ctx: &AppContext, entry_id: &str, pinned: bool) -> bool {
    let result = ctx
        .clipboard
        .borrow_mut()
        .set_pinned(entry_id, pinned, SystemTime::now());
    ctx.emit(Change::Clipboard);
    result.unwrap_or(false)
}

/// Delete the whole history, pins included.
pub fn clear_all(ctx: &AppContext) {
    if let Err(error) = ctx.clipboard.borrow_mut().clear() {
        eprintln!("kebloop: could not clear the clipboard history: {error}");
    }
    ctx.emit(Change::Clipboard);
}

/// Decoded thumbnail pixels, produced off the main thread.
struct Pixels {
    bytes: Vec<u8>,
    width: i32,
    height: i32,
    stride: usize,
    alpha: bool,
}

/// Largest image side that is decoded for a thumbnail.
pub const MAX_THUMBNAIL_SIDE: i32 = 16_384;
/// Largest pixel count (width x height) that is decoded for a thumbnail.
pub const MAX_THUMBNAIL_PIXELS: i64 = 40_000_000;

/// Whether an image of these dimensions may be decoded: a few KB of PNG can claim gigapixels
/// ("image bomb"), so the header is checked before any pixel is decoded.
pub fn thumbnail_dimensions_ok(width: i32, height: i32) -> bool {
    width > 0
        && height > 0
        && width <= MAX_THUMBNAIL_SIDE
        && height <= MAX_THUMBNAIL_SIDE
        && i64::from(width) * i64::from(height) <= MAX_THUMBNAIL_PIXELS
}

/// Decode a thumbnail of the entry's image on a worker thread and hand the texture to `done`.
/// `key` is a [`thumbnail_key`](crate::launcher::model::thumbnail_key); a key whose attachment is
/// no longer the entry's current image is ignored. The header dimensions are checked before
/// decoding (see [`thumbnail_dimensions_ok`]).
pub fn load_thumbnail(
    ctx: &Rc<AppContext>,
    key: &str,
    size: i32,
    done: impl Fn(&str, gdk::Texture) + 'static,
) {
    let Some((entry_id, file_name)) = crate::launcher::model::split_thumbnail_key(key) else {
        return;
    };
    let Some(path) = ctx.clipboard.borrow().image_path(entry_id) else {
        return;
    };
    if path.file_name().and_then(|n| n.to_str()) != Some(file_name) {
        return;
    }
    let (sender, receiver) = async_channel::bounded::<Option<Pixels>>(1);
    let spawned = std::thread::Builder::new()
        .name("kebloop-thumbnail".into())
        .spawn(move || {
            let within_limits = gtk::gdk_pixbuf::Pixbuf::file_info(&path)
                .is_some_and(|(_, width, height)| thumbnail_dimensions_ok(width, height));
            let pixels = within_limits
                .then(|| {
                    gtk::gdk_pixbuf::Pixbuf::from_file_at_scale(&path, size * 2, size * 2, true)
                        .ok()
                })
                .flatten()
                .map(|pixbuf| Pixels {
                    bytes: pixbuf.read_pixel_bytes().to_vec(),
                    width: pixbuf.width(),
                    height: pixbuf.height(),
                    stride: usize::try_from(pixbuf.rowstride()).unwrap_or(0),
                    alpha: pixbuf.has_alpha(),
                });
            let _ = sender.send_blocking(pixels);
        });
    if spawned.is_err() {
        return;
    }
    let id = key.to_owned();
    glib::spawn_future_local(async move {
        if let Ok(Some(pixels)) = receiver.recv().await {
            let format = if pixels.alpha {
                gdk::MemoryFormat::R8g8b8a8
            } else {
                gdk::MemoryFormat::R8g8b8
            };
            let texture = gdk::MemoryTexture::new(
                pixels.width,
                pixels.height,
                format,
                &glib::Bytes::from_owned(pixels.bytes),
                pixels.stride,
            );
            done(&id, texture.upcast());
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_bombs_are_refused_before_decoding() {
        assert!(thumbnail_dimensions_ok(1, 1));
        assert!(thumbnail_dimensions_ok(1920, 1080));
        assert!(thumbnail_dimensions_ok(16_384, 2_000));
        assert!(thumbnail_dimensions_ok(6_324, 6_324));
        assert!(!thumbnail_dimensions_ok(16_385, 1));
        assert!(!thumbnail_dimensions_ok(1, 16_385));
        assert!(!thumbnail_dimensions_ok(16_384, 16_384), "268 MP");
        assert!(!thumbnail_dimensions_ok(7_000, 7_000), "49 MP");
        assert!(!thumbnail_dimensions_ok(0, 100));
        assert!(!thumbnail_dimensions_ok(-5, 100));
    }

    #[test]
    fn unavailable_backend_wins_over_everything() {
        let status = ClipboardStatus::Unavailable("no protocol".into());
        assert_eq!(
            note_for(&status, true, false, 5, 5),
            Some(ClipboardNote::Unavailable("no protocol".into()))
        );
    }

    #[test]
    fn disabled_and_paused_states() {
        let active = ClipboardStatus::Active;
        assert_eq!(
            note_for(&active, false, false, 0, 0),
            Some(ClipboardNote::Disabled)
        );
        assert_eq!(
            note_for(&active, true, true, 3, 3),
            Some(ClipboardNote::Paused)
        );
    }

    #[test]
    fn empty_and_filtered_states() {
        let active = ClipboardStatus::Active;
        assert_eq!(
            note_for(&active, true, false, 0, 0),
            Some(ClipboardNote::Empty)
        );
        assert_eq!(
            note_for(&active, true, false, 4, 0),
            Some(ClipboardNote::NoMatch)
        );
        assert_eq!(note_for(&active, true, false, 4, 2), None);
    }
}
