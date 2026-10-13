//! Appearance: colour scheme, text size, panel width, density, transparency and, on layer-shell
//! sessions, which display the panel appears on.

use std::rc::Rc;

use adw::prelude::*;
use gtk::gdk;
use kebloop_services::personalization::PersonalizationSettings;

use super::logic::PageId;
use super::widgets::{
    ComboControl, Env, Page, SpinControl, SwitchControl, button_row, group, note, preferences_page,
};
use crate::i18n::tr;

const SCHEMES: [&str; 3] = ["system", "light", "dark"];

/// Restore the appearance preferences (not the search ones).
pub fn reset_appearance(personalization: &mut PersonalizationSettings) {
    let defaults = PersonalizationSettings::default();
    personalization.appearance = defaults.appearance;
    personalization.text_size = defaults.text_size;
    personalization.panel_width = defaults.panel_width;
    personalization.compact = defaults.compact;
    personalization.transparency = defaults.transparency;
    personalization.placement = defaults.placement;
    personalization.display_id = defaults.display_id;
}

/// Connectors of the connected displays, in GDK order.
fn connectors() -> Vec<String> {
    let Some(display) = gdk::Display::default() else {
        return Vec::new();
    };
    let monitors = display.monitors();
    (0..monitors.n_items())
        .filter_map(|i| {
            monitors
                .item(i)
                .and_then(|o| o.downcast::<gdk::Monitor>().ok())
        })
        .filter_map(|m| m.connector().map(|c| c.to_string()))
        .collect()
}

/// Build the page.
pub fn build(env: &Rc<Env>) -> Page {
    let ctx = env.ctx.clone();
    let page = preferences_page();
    page.set_title(&PageId::Appearance.title());
    page.set_description(&PageId::Appearance.subtitle());
    let read = |ctx: &crate::context::AppContext| ctx.with_settings(|s| s.personalization.clone());
    let p = read(&ctx);

    let look = group(&tr("Appearance"), "");
    let scheme_combo = {
        let ctx = ctx.clone();
        Rc::new(ComboControl::new(
            env,
            &tr("Color scheme"),
            "",
            &[tr("System"), tr("Light"), tr("Dark")],
            u32::try_from(SCHEMES.iter().position(|s| *s == p.appearance).unwrap_or(0))
                .unwrap_or(0),
            move |index| {
                let Some(scheme) = SCHEMES.get(index as usize) else {
                    return Ok(());
                };
                ctx.edit_settings(|s| s.personalization.appearance = (*scheme).to_owned())
            },
        ))
    };
    look.add(&scheme_combo.row);
    let text_spin = {
        let ctx = ctx.clone();
        Rc::new(SpinControl::new(
            env,
            &tr("Text size"),
            "",
            (11.0, 24.0, 1.0),
            p.text_size,
            move |value| ctx.edit_settings(|s| s.personalization.text_size = value.round()),
        ))
    };
    look.add(&text_spin.row);
    let width_spin = {
        let ctx = ctx.clone();
        Rc::new(SpinControl::new(
            env,
            &tr("Panel width"),
            &tr("Between 420 and 1000 pixels."),
            (420.0, 1000.0, 20.0),
            p.panel_width,
            move |value| ctx.edit_settings(|s| s.personalization.panel_width = value.round()),
        ))
    };
    look.add(&width_spin.row);
    let compact = {
        let ctx = ctx.clone();
        Rc::new(SwitchControl::new(
            env,
            &tr("Compact rows"),
            "",
            p.compact,
            move |on| ctx.edit_settings(|s| s.personalization.compact = on),
        ))
    };
    look.add(&compact.row);
    let transparency = {
        let ctx = ctx.clone();
        Rc::new(SwitchControl::new(
            env,
            &tr("Use translucent panel"),
            &tr("Blur and see-through effects depend on your compositor."),
            p.transparency,
            move |on| ctx.edit_settings(|s| s.personalization.transparency = on),
        ))
    };
    look.add(&transparency.row);
    page.add(&look);

    // Placement: only meaningful with layer-shell, where the compositor otherwise picks the
    // focused output.
    let placement = group(&tr("Panel placement"), "");
    let layer_shell = crate::launcher::layer_shell_supported();
    let displays = Rc::new(std::cell::RefCell::new(connectors()));
    let mode = {
        let ctx = ctx.clone();
        Rc::new(ComboControl::new(
            env,
            &tr("Place launcher on"),
            "",
            &[tr("Focused display"), tr("Selected display")],
            u32::from(p.placement == "display"),
            move |index| {
                let selected = index == 1;
                let first = connectors().into_iter().next();
                ctx.edit_settings(|s| {
                    if selected {
                        s.personalization.placement = "display".into();
                        if s.personalization.display_id.is_empty() {
                            s.personalization.display_id = first.clone().unwrap_or_default();
                        }
                    } else {
                        s.personalization.placement = "pointer".into();
                    }
                })
            },
        ))
    };
    let initial_displays = displays.borrow().clone();
    let display = {
        let ctx = ctx.clone();
        let displays = displays.clone();
        Rc::new(ComboControl::new(
            env,
            &tr("Display"),
            "",
            &initial_displays,
            0,
            move |index| {
                let Some(connector) = displays.borrow().get(index as usize).cloned() else {
                    return Ok(());
                };
                ctx.edit_settings(|s| s.personalization.display_id = connector)
            },
        ))
    };
    let placement_note = note("");
    if layer_shell {
        placement.add(&mode.row);
        placement.add(&display.row);
        placement.add(&placement_note);
    } else {
        placement.add(&note(&tr(
            "This desktop does not support layer-shell panels, so the compositor decides where the launcher appears.",
        )));
    }
    page.add(&placement);

    let reset = group("", "");
    {
        let ctx = ctx.clone();
        let env = env.clone();
        reset.add(&button_row(
            &tr("Reset Appearance to Defaults"),
            "edit-undo-symbolic",
            move || {
                if let Err(message) =
                    ctx.edit_settings(|s| reset_appearance(&mut s.personalization))
                {
                    env.toast(&message);
                }
            },
        ));
    }
    page.add(&reset);

    let refresh = {
        let ctx = ctx.clone();
        move || {
            let p = read(&ctx);
            scheme_combo.select(
                u32::try_from(SCHEMES.iter().position(|s| *s == p.appearance).unwrap_or(0))
                    .unwrap_or(0),
            );
            text_spin.sync(p.text_size);
            width_spin.sync(p.panel_width);
            compact.sync(p.compact);
            transparency.sync(p.transparency);
            let connected = connectors();
            let known = *displays.borrow() == connected;
            if !known {
                *displays.borrow_mut() = connected.clone();
            }
            let selected = connected
                .iter()
                .position(|c| *c == p.display_id)
                .and_then(|i| u32::try_from(i).ok());
            if !known {
                display.sync(&connected, selected.unwrap_or(0));
            } else if let Some(index) = selected {
                display.select(index);
            }
            mode.select(u32::from(p.placement == "display"));
            display
                .row
                .set_sensitive(p.placement == "display" && !connected.is_empty());
            placement_note.set_label(&if connected.is_empty() {
                tr("No display information is available.")
            } else if p.placement == "display" && selected.is_none() {
                tr("The selected display is unavailable. Kebloop will use the focused display until it returns.")
            } else {
                String::new()
            });
            placement_note.set_visible(!placement_note.label().is_empty());
        }
    };
    refresh();
    Page::new(&page, refresh)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_restores_appearance_only() {
        let mut p = PersonalizationSettings {
            appearance: "dark".into(),
            text_size: 20.0,
            panel_width: 900.0,
            compact: true,
            transparency: false,
            placement: "display".into(),
            display_id: "DP-1".into(),
            files_enabled: false,
            ..PersonalizationSettings::default()
        };
        reset_appearance(&mut p);
        let defaults = PersonalizationSettings::default();
        assert_eq!(p.appearance, defaults.appearance);
        assert_eq!(p.text_size, defaults.text_size);
        assert_eq!(p.panel_width, defaults.panel_width);
        assert!(!p.compact && p.transparency);
        assert_eq!(
            (p.placement.as_str(), p.display_id.as_str()),
            ("pointer", "")
        );
        assert!(!p.files_enabled);
        p.validate().unwrap();
    }
}
