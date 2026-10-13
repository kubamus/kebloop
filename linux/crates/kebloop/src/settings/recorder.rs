//! The shortcut recorder: a dialog that captures a key chord (or accepts a typed GTK accelerator)
//! and hands it to the caller, which can refuse it (conflicts) with a message shown inline.

use std::rc::Rc;

use adw::prelude::*;
use gtk::gdk;
use kebloop_services::accelerator::{ALT, Accelerator, CONTROL, SHIFT, SUPER};

use super::logic::{Recorded, accelerator_label, modifiers_label, record_key};
use super::widgets::{Env, error_label, group, note, set_error};
use crate::i18n::tr;

/// Open the recorder. `accept` receives the GTK accelerator string and a display label; return
/// `Err(message)` to keep the dialog open and show the message.
pub fn record_shortcut(
    env: &Rc<Env>,
    title: &str,
    accept: impl Fn(&str, &str) -> Result<(), String> + 'static,
) {
    let dialog = adw::Dialog::builder()
        .title(tr("Shortcut recorder"))
        .content_width(420)
        .build();
    let heading = gtk::Label::builder()
        .label(title)
        .wrap(true)
        .css_classes(["title-3"])
        .build();
    let live = gtk::Label::builder()
        .label("…")
        .css_classes(["title-1"])
        .build();
    let capture = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .halign(gtk::Align::Fill)
        .css_classes(["card"])
        .focusable(true)
        .can_focus(true)
        .build();
    live.set_margin_top(24);
    live.set_margin_bottom(24);
    capture.append(&live);
    capture.update_property(&[gtk::accessible::Property::Label(&tr("Shortcut recorder"))]);
    let hint = note(&tr(
        "Press Control, Alt, or Super with a key. Press Escape to cancel.",
    ));
    let errors = error_label();

    let typed = adw::EntryRow::builder()
        .title(tr("Shortcut text input"))
        .show_apply_button(true)
        .build();
    let typed_group = group(
        "",
        &tr("Or type a GTK accelerator such as <Control><Alt>t."),
    );
    typed_group.add(&typed);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.set_margin_top(18);
    content.set_margin_bottom(18);
    content.set_margin_start(18);
    content.set_margin_end(18);
    content.append(&heading);
    content.append(&capture);
    content.append(&hint);
    content.append(&errors);
    content.append(&typed_group);
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(&content));
    dialog.set_child(Some(&toolbar));
    dialog.set_focus(Some(&capture));

    let accept = Rc::new(accept);
    let submit = {
        let dialog = dialog.clone();
        let errors = errors.clone();
        move |accelerator: &Accelerator| {
            let text = accelerator.to_string();
            let label = accelerator_label(accelerator);
            match accept(&text, &label) {
                Ok(()) => {
                    dialog.close();
                }
                Err(message) => set_error(&errors, Some(&message)),
            }
        }
    };
    let submit = Rc::new(submit);

    {
        let submit = submit.clone();
        let errors = errors.clone();
        typed.connect_apply(move |row| {
            match Accelerator::parse(row.text().trim()) {
                Ok(accelerator) if accelerator.has_command_modifier() => submit(&accelerator),
                Ok(_) => set_error(&errors, Some(&tr("Include Control, Alt, or Super."))),
                Err(error) => set_error(&errors, Some(&tr(&error.to_string()))),
            };
        });
    }

    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    {
        let live = live.clone();
        let errors = errors.clone();
        let typed = typed.clone();
        let dialog_weak = dialog.downgrade();
        keys.connect_key_pressed(move |_, keyval, keycode, state| {
            let modifiers = modifier_bits(state);
            let in_entry = dialog_weak
                .upgrade()
                .and_then(|d| d.focus())
                .is_some_and(|focus| {
                    focus.is_ancestor(&typed) || focus.eq(typed.upcast_ref::<gtk::Widget>())
                });
            let is_modifier_key = is_modifier(keyval);
            // Typing into the text field stays typing unless a command modifier is held.
            if in_entry && modifiers & (CONTROL | ALT | SUPER) == 0 {
                return gtk::glib::Propagation::Proceed;
            }
            if keyval == gdk::Key::Escape && modifiers == 0 {
                return gtk::glib::Propagation::Proceed;
            }
            let name = if is_modifier_key {
                None
            } else {
                Some(unshifted_key_name(keyval, keycode))
            };
            match record_key(modifiers, name.as_deref()) {
                Recorded::Pending(held) => {
                    live.set_label(&modifiers_label(held));
                    set_error(&errors, None);
                }
                Recorded::Chord(accelerator) => {
                    live.set_label(&accelerator_label(&accelerator));
                    set_error(&errors, None);
                    submit(&accelerator);
                }
                Recorded::Rejected(message) => {
                    live.set_label(&modifiers_label(modifiers));
                    set_error(&errors, Some(&message));
                }
            }
            gtk::glib::Propagation::Stop
        });
    }
    {
        let live = live.clone();
        keys.connect_key_released(move |_, _, _, state| {
            let held = modifier_bits(state);
            if live.label().ends_with('…') {
                live.set_label(&modifiers_label(held));
            }
        });
    }
    content.add_controller(keys);
    env.present(&dialog);
    capture.grab_focus();
}

fn modifier_bits(state: gdk::ModifierType) -> u8 {
    let mut bits = 0;
    if state.contains(gdk::ModifierType::CONTROL_MASK) {
        bits |= CONTROL;
    }
    if state.contains(gdk::ModifierType::ALT_MASK) {
        bits |= ALT;
    }
    if state.contains(gdk::ModifierType::SHIFT_MASK) {
        bits |= SHIFT;
    }
    if state.contains(gdk::ModifierType::SUPER_MASK) {
        bits |= SUPER;
    }
    bits
}

fn is_modifier(key: gdk::Key) -> bool {
    matches!(
        key,
        gdk::Key::Control_L
            | gdk::Key::Control_R
            | gdk::Key::Alt_L
            | gdk::Key::Alt_R
            | gdk::Key::Shift_L
            | gdk::Key::Shift_R
            | gdk::Key::Super_L
            | gdk::Key::Super_R
            | gdk::Key::Meta_L
            | gdk::Key::Meta_R
            | gdk::Key::ISO_Level3_Shift
            | gdk::Key::Caps_Lock
            | gdk::Key::Num_Lock
            | gdk::Key::Hyper_L
            | gdk::Key::Hyper_R
    )
}

/// The GDK name of the key without the effect of Shift (`Shift+1` is the `1` key, not `!`).
fn unshifted_key_name(keyval: gdk::Key, keycode: u32) -> String {
    let base = gdk::Display::default()
        .and_then(|display| display.map_keycode(keycode))
        .and_then(|entries| {
            entries
                .into_iter()
                .find(|(keymap_key, _)| keymap_key.level() == 0 && keymap_key.group() == 0)
                .map(|(_, key)| key)
        })
        .unwrap_or_else(|| keyval.to_lower());
    base.name().map(|name| name.to_string()).unwrap_or_default()
}
