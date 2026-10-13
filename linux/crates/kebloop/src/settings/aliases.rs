//! Application aliases: extra names to search for an application.

use std::rc::Rc;

use adw::prelude::*;
use kebloop_services::aliases::save_alias;

use super::logic::{PageId, filter_apps};
use super::widgets::{DynGroup, Env, Page, group, icon_button, placeholder_row, preferences_page};
use crate::i18n::{tr, trf};
use kebloop_services::l10n::FormatArg;

/// How many applications are listed at once.
const ROWS: usize = 50;

/// Build the page.
pub fn build(env: &Rc<Env>) -> Page {
    let page = preferences_page();
    page.set_title(&PageId::Aliases.title());
    page.set_description(&PageId::Aliases.subtitle());
    let head = group(&tr("Application aliases"), "");
    let search = gtk::SearchEntry::builder()
        .placeholder_text(tr("Search applications"))
        .margin_bottom(6)
        .build();
    search.update_property(&[gtk::accessible::Property::Label(&tr(
        "Search applications for aliases",
    ))]);
    head.add(&search);
    head.add(&super::widgets::note(&tr(
        "Give an app a name to search for, such as “terminal” for Ghostty.",
    )));
    page.add(&head);
    let list = Rc::new(DynGroup::new(group("", "")));
    page.add(&list.group);

    let refresh: Rc<dyn Fn()> = {
        let env = env.clone();
        let search = search.clone();
        Rc::new(move || {
            let query = search.text().to_string();
            let apps = env.ctx.apps.borrow().clone();
            let aliases = env.ctx.with_settings(|s| s.aliases.clone());
            // Apps that have an alias come first, so they are easy to find again.
            let (shown, more) = {
                let (mut all, _) = filter_apps(&apps, &query, apps.len());
                all.sort_by_key(|app| !aliases.contains_key(&app.id));
                let more = all.len().saturating_sub(ROWS);
                all.truncate(ROWS);
                (all, more)
            };
            let signature = format!(
                "{query}|{}|{more}",
                shown
                    .iter()
                    .map(|a| format!("{}={}", a.id, aliases.get(&a.id).map_or("", String::as_str)))
                    .collect::<Vec<_>>()
                    .join("\u{1}")
            );
            list.update(&signature, || {
                let mut rows: Vec<gtk::Widget> = shown
                    .iter()
                    .map(|app| {
                        alias_row(
                            &env,
                            &app.id,
                            &app.name,
                            app.icon.clone(),
                            aliases.get(&app.id),
                        )
                    })
                    .collect();
                if rows.is_empty() {
                    rows.push(placeholder_row(
                        &tr("No matching applications"),
                        &tr("Try another search term."),
                    ));
                }
                if more > 0 {
                    rows.push(placeholder_row(
                        &trf(
                            "%d more applications. Refine the search to see them.",
                            &[FormatArg::Int(i64::try_from(more).unwrap_or(0))],
                        ),
                        "",
                    ));
                }
                rows
            });
        })
    };
    {
        let refresh = refresh.clone();
        search.connect_search_changed(move |_| refresh());
    }
    refresh();
    Page::new(&page, move || refresh())
}

fn alias_row(
    env: &Rc<Env>,
    id: &str,
    name: &str,
    icon: Option<String>,
    alias: Option<&String>,
) -> gtk::Widget {
    let row = adw::EntryRow::builder()
        .title(gtk::glib::markup_escape_text(name))
        .text(alias.map_or("", String::as_str))
        .show_apply_button(true)
        .build();
    row.set_tooltip_text(Some(&trf(
        "Alias for %@",
        &[FormatArg::Str(name.to_owned())],
    )));
    let image = gtk::Image::new();
    crate::icons::apply(&image, &crate::launcher::model::IconSpec::App(icon), 24);
    row.add_prefix(&image);
    {
        let (env, id) = (env.clone(), id.to_owned());
        row.connect_apply(move |entry| {
            let text = entry.text();
            let outcome = env
                .ctx
                .update_settings(|s| save_alias(&mut s.aliases, &id, &text));
            match outcome {
                Ok(()) => {
                    entry.remove_css_class("error");
                    env.toast(&if text.trim().is_empty() {
                        tr("Alias removed.")
                    } else {
                        tr("Alias saved.")
                    });
                }
                Err(error) => {
                    entry.add_css_class("error");
                    env.toast(&tr(&error.to_string()));
                }
            }
        });
    }
    if alias.is_some() {
        let clear = icon_button("edit-clear-symbolic", &tr("Remove"));
        let (env, id) = (env.clone(), id.to_owned());
        clear.connect_clicked(move |_| {
            if let Err(message) = env.ctx.edit_settings(|s| {
                s.aliases.remove(&id);
            }) {
                env.toast(&message);
            }
        });
        row.add_suffix(&clear);
    }
    row.upcast()
}
