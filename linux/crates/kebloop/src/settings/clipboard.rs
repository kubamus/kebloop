//! Clipboard: recording switch, limits, pausing, backend status and clearing the history.

use std::rc::Rc;
use std::time::{Duration, SystemTime};

use adw::prelude::*;

use super::logic::PageId;
use super::widgets::{
    Env, Page, SpinControl, SwitchControl, action_row, button_row, confirm, group, note,
    preferences_page, row_button,
};
use crate::context::{AppContext, Change, ClipboardStatus};
use crate::i18n::{tr, trf};
use kebloop_services::l10n::FormatArg;

/// Title and explanation of the clipboard backend state.
pub fn backend_text(status: &ClipboardStatus) -> (String, String) {
    match status {
        ClipboardStatus::Starting => (tr("Starting…"), String::new()),
        ClipboardStatus::Active => (tr("Recording clipboard changes."), String::new()),
        ClipboardStatus::HistoryDisabled(reason) => (
            tr("Clipboard history is turned off in this session."),
            reason.clone(),
        ),
        ClipboardStatus::Unavailable(reason) => (
            tr("Clipboard history is unavailable on this compositor."),
            reason.clone(),
        ),
    }
}

fn pause_text(ctx: &AppContext) -> String {
    let store = ctx.clipboard.borrow();
    let now = SystemTime::now();
    match store.pause_deadline().filter(|_| store.is_paused(now)) {
        Some(deadline) => {
            let formatted = jiff::Timestamp::try_from(deadline)
                .map(|t| {
                    t.to_zoned(ctx.time_zone.clone())
                        .strftime("%H:%M")
                        .to_string()
                })
                .unwrap_or_default();
            format!("{} {formatted}", tr("Paused until"))
        }
        None => String::new(),
    }
}

fn apply<F>(ctx: &AppContext, change: F) -> Result<(), String>
where
    F: FnOnce(
        &mut kebloop_services::clipboard::ClipboardStore,
    ) -> Result<(), kebloop_services::ConfigError>,
{
    let outcome = change(&mut ctx.clipboard.borrow_mut()).map_err(|e| tr(&e.to_string()));
    ctx.emit(Change::Clipboard);
    outcome
}

/// Build the page.
pub fn build(env: &Rc<Env>) -> Page {
    let ctx = env.ctx.clone();
    let page = preferences_page();
    page.set_title(&PageId::Clipboard.title());
    page.set_description(&PageId::Clipboard.subtitle());

    let backend = group(&tr("Backend"), "");
    let backend_row = action_row("", "");
    backend_row.set_subtitle_lines(0);
    backend_row.add_prefix(&gtk::Image::from_icon_name("edit-paste-symbolic"));
    backend.add(&backend_row);
    page.add(&backend);

    let history = group(&tr("Clipboard history"), "");
    let (is_on, max_items, max_days) = {
        let store = ctx.clipboard.borrow();
        (
            store.is_enabled(),
            u32::try_from(store.maximum_item_count()).unwrap_or(200),
            store.maximum_age_days(),
        )
    };
    let enabled = {
        let ctx = ctx.clone();
        Rc::new(SwitchControl::new(
            env,
            &tr("Enable clipboard history"),
            &tr(
                "Copied text is stored locally on this computer. Disable history to stop collecting new items.",
            ),
            is_on,
            move |on| apply(&ctx, |store| store.set_enabled(on)),
        ))
    };
    history.add(&enabled.row);
    let limit = {
        let ctx = ctx.clone();
        Rc::new(SpinControl::new(
            env,
            &tr("Unpinned item limit"),
            "",
            (1.0, 200.0, 1.0),
            f64::from(max_items),
            move |value| {
                apply(&ctx, |store| {
                    store.set_maximum_item_count(value as usize, SystemTime::now())
                })
            },
        ))
    };
    history.add(&limit.row);
    let retention = {
        let ctx = ctx.clone();
        Rc::new(SpinControl::new(
            env,
            &tr("Retention days (0 = forever)"),
            "",
            (0.0, 3650.0, 1.0),
            f64::from(max_days),
            move |value| {
                apply(&ctx, |store| {
                    store.set_maximum_age_days(value as u32, SystemTime::now())
                })
            },
        ))
    };
    history.add(&retention.row);
    history.add(&note(&tr(
        "Pinned items do not expire and are exempt from the count limit (up to 50 pins). Clear History includes pinned items. Attachments are limited to 5 MB per entry and 50 MB total.",
    )));
    page.add(&history);

    let pause = group(&tr("Pause recording"), "");
    let pause_row = action_row(&tr("Pause recording"), "");
    let resume = row_button(&tr("Resume Recording"));
    let quarter = row_button(&tr("Pause for 15 minutes"));
    let hour = row_button(&tr("Pause for 1 hour"));
    for (button, seconds) in [(&quarter, 15 * 60), (&hour, 60 * 60)] {
        let ctx = ctx.clone();
        button.connect_clicked(move |_| {
            let _ = apply(&ctx, |store| {
                store.pause_for(Duration::from_secs(seconds), SystemTime::now())
            });
        });
    }
    {
        let ctx = ctx.clone();
        resume.connect_clicked(move |_| {
            let _ = apply(&ctx, |store| store.resume());
        });
    }
    let buttons = gtk::Box::builder()
        .spacing(6)
        .valign(gtk::Align::Center)
        .build();
    buttons.append(&quarter);
    buttons.append(&hour);
    buttons.append(&resume);
    pause_row.add_suffix(&buttons);
    pause.add(&pause_row);
    page.add(&pause);

    let privacy = group(&tr("Privacy"), "");
    privacy.add(&note(&tr(
        "Kebloop skips items that mark themselves as passwords or concealed.",
    )));
    privacy.add(&note(&tr(
        "Source exclusions use X11 clipboard-owner identities. Native Wayland offers no reliable source identity, so exclusions cannot be applied there.",
    )));
    let excluded = adw::EntryRow::builder()
        .title(tr("Excluded source applications (comma-separated IDs)"))
        .show_apply_button(true)
        .build();
    let last_sources = std::cell::RefCell::new(None::<String>);
    {
        let ctx = ctx.clone();
        excluded.connect_apply(move |row| {
            let sources = row
                .text()
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .collect();
            let _ = apply(&ctx, |store| store.set_excluded_sources(sources));
        });
    }
    privacy.add(&excluded);
    page.add(&privacy);

    let data = group("", "");
    let count_row = action_row("", "");
    count_row.add_prefix(&gtk::Image::from_icon_name("edit-paste-symbolic"));
    data.add(&count_row);
    let clear = button_row(&tr("Clear Clipboard History"), "user-trash-symbolic", {
        let env = env.clone();
        move || {
            let env2 = env.clone();
            confirm(
                &env,
                &tr("Clear clipboard history?"),
                &tr(
                    "All saved clipboard items, including pinned items and attachments, will be removed.",
                ),
                &tr("Clear History"),
                true,
                move || {
                    if let Err(message) = apply(&env2.ctx, |store| store.clear()) {
                        env2.toast(&message);
                    }
                },
            );
        }
    });
    clear.add_css_class("destructive-action");
    data.add(&clear);
    page.add(&data);

    let refresh = {
        let ctx = ctx.clone();
        move || {
            let (title, detail) = backend_text(&ctx.clipboard_status.borrow());
            backend_row.set_title(&gtk::glib::markup_escape_text(&title));
            backend_row.set_subtitle(&gtk::glib::markup_escape_text(&detail));
            let (is_enabled, count, max, days, paused) = {
                let store = ctx.clipboard.borrow();
                (
                    store.is_enabled(),
                    store.items().len(),
                    store.maximum_item_count(),
                    store.maximum_age_days(),
                    store.is_paused(SystemTime::now()),
                )
            };
            enabled.sync(is_enabled);
            let sources = ctx
                .clipboard
                .borrow()
                .excluded_sources()
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ");
            if last_sources.borrow().as_ref() != Some(&sources) {
                excluded.set_text(&sources);
                *last_sources.borrow_mut() = Some(sources);
            }
            limit.sync(f64::from(u32::try_from(max).unwrap_or(200)));
            retention.sync(f64::from(days));
            resume.set_sensitive(paused);
            pause_row.set_subtitle(&pause_text(&ctx));
            count_row.set_title(&trf(
                "%d saved items",
                &[FormatArg::Int(i64::try_from(count).unwrap_or(0))],
            ));
            clear.set_sensitive(count > 0);
        }
    };
    refresh();
    Page::new(&page, refresh)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_states_are_described() {
        assert_eq!(
            backend_text(&ClipboardStatus::Unavailable("no data-control".into())).1,
            "no data-control"
        );
        assert!(!backend_text(&ClipboardStatus::Active).0.is_empty());
        assert!(backend_text(&ClipboardStatus::Active).1.is_empty());
    }
}
