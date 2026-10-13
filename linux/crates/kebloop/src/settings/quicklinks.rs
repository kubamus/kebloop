//! Quicklinks and web-search templates, with inline validation and a live URL preview.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use kebloop_services::quicklinks::{
    Destination, destination, remove_quicklink, remove_web_search, save_quicklink, save_web_search,
};

use super::logic::PageId;
use super::widgets::{
    DynGroup, Env, Page, action_row, button_row, error_label, group, icon_button, note,
    placeholder_row, preferences_page, set_error,
};
use crate::i18n::{tr, trf};
use kebloop_services::l10n::FormatArg;

/// Preview of a `{query}` target for the sample text `argument`, or the reason it is invalid.
pub fn preview_text(target: &str, argument: &str, home: &str) -> String {
    match destination(target, argument, home) {
        Some(Destination::Url(url)) => url.to_string(),
        Some(Destination::File(path)) => path.to_string_lossy().into_owned(),
        None => tr("This template is invalid."),
    }
}

/// Build the page.
pub fn build(env: &Rc<Env>) -> Page {
    let ctx = env.ctx.clone();
    let page = preferences_page();
    page.set_title(&PageId::Quicklinks.title());
    page.set_description(&PageId::Quicklinks.subtitle());

    // ----- quicklinks -----
    let links = Rc::new(DynGroup::new(group(
        &tr("Quicklinks"),
        &tr(
            "Open websites, files, and folders by name. A keyword can also run a link with an argument, such as “gh kebloop”.",
        ),
    )));
    page.add(&links.group);
    let editing: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let name = adw::EntryRow::builder().title(tr("Name")).build();
    let keyword = adw::EntryRow::builder()
        .title(tr("Keyword (optional)"))
        .build();
    let target = adw::EntryRow::builder()
        .title(tr("Website URL, file path, or folder path"))
        .build();
    let preview = note("");
    preview.set_selectable(true);
    preview.set_visible(false);
    let link_error = error_label();
    let editor_title = group(&tr("Add a quicklink"), "");
    editor_title.add(&name);
    editor_title.add(&keyword);
    editor_title.add(&target);
    editor_title.add(&preview);
    editor_title.add(&link_error);
    let save_row = button_row(&tr("Add Quicklink"), "list-add-symbolic", || {});
    let cancel_row = button_row(&tr("Cancel"), "window-close-symbolic", || {});
    cancel_row.set_visible(false);
    let reset_form: Rc<dyn Fn()> = {
        let (editing, name, keyword, target, error) = (
            editing.clone(),
            name.clone(),
            keyword.clone(),
            target.clone(),
            link_error.clone(),
        );
        let (save_row, cancel_row, editor_title) =
            (save_row.clone(), cancel_row.clone(), editor_title.clone());
        Rc::new(move || {
            *editing.borrow_mut() = None;
            name.set_text("");
            keyword.set_text("");
            target.set_text("");
            set_error(&error, None);
            save_row.set_title(&tr("Add Quicklink"));
            cancel_row.set_visible(false);
            editor_title.set_title(&tr("Add a quicklink"));
        })
    };
    {
        let (preview, home) = (preview.clone(), ctx.home.clone());
        target.connect_changed(move |entry| {
            let text = entry.text();
            let templated = text.contains("{query}");
            preview.set_visible(templated);
            if templated {
                preview.set_label(&trf(
                    "Preview: %@",
                    &[FormatArg::Str(preview_text(&text, "A space & tea", &home))],
                ));
            }
        });
    }
    {
        let (env, editing, name, keyword, target, error, reset) = (
            env.clone(),
            editing.clone(),
            name.clone(),
            keyword.clone(),
            target.clone(),
            link_error.clone(),
            reset_form.clone(),
        );
        save_row.connect_activated(move |_| {
            let home = env.ctx.home.clone();
            let id = editing.borrow().clone();
            let (n, t, k) = (name.text(), target.text(), keyword.text());
            let outcome = env.ctx.update_settings(|s| {
                let s = &mut *s;
                save_quicklink(
                    &mut s.quicklinks,
                    &s.web_searches,
                    id.as_deref(),
                    &n,
                    &t,
                    &k,
                    &home,
                )
            });
            match outcome {
                Ok(_) => reset(),
                Err(error_value) => set_error(&error, Some(&tr(&error_value.to_string()))),
            }
        });
    }
    {
        let reset = reset_form.clone();
        cancel_row.connect_activated(move |_| reset());
    }
    editor_title.add(&save_row);
    editor_title.add(&cancel_row);
    page.add(&editor_title);

    // ----- web searches -----
    let searches = Rc::new(DynGroup::new(group(
        &tr("Web search templates"),
        &tr(
            "Use {query} where the encoded search text should go. For example, type “web cats” in the launcher.",
        ),
    )));
    page.add(&searches.group);
    let editing_search: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let search_name = adw::EntryRow::builder()
        .title(tr("Name"))
        .text("Web Search")
        .build();
    let search_keyword = adw::EntryRow::builder()
        .title(tr("Keyword"))
        .text("web")
        .build();
    let search_target = adw::EntryRow::builder()
        .title(tr("https://example.com/?q={query}"))
        .text("https://duckduckgo.com/?q={query}")
        .build();
    let search_preview = note("");
    search_preview.set_selectable(true);
    let search_error = error_label();
    let search_editor = group(&tr("Add a web search"), "");
    search_editor.add(&search_name);
    search_editor.add(&search_keyword);
    search_editor.add(&search_target);
    search_editor.add(&search_preview);
    search_editor.add(&search_error);
    let search_save = button_row(&tr("Add Web Search"), "list-add-symbolic", || {});
    let search_cancel = button_row(&tr("Cancel"), "window-close-symbolic", || {});
    search_cancel.set_visible(false);
    let update_preview = {
        let (preview, home) = (search_preview.clone(), ctx.home.clone());
        Rc::new(move |text: &str| {
            preview.set_visible(!text.trim().is_empty());
            let shown = match destination(text, "Warsaw cafés & tea", &home) {
                Some(Destination::Url(url)) if text.contains("{query}") => url.to_string(),
                _ => tr("Use a valid HTTP or HTTPS URL with {query}."),
            };
            preview.set_label(&trf("Preview: %@", &[FormatArg::Str(shown)]));
        })
    };
    update_preview(&search_target.text());
    {
        let update_preview = update_preview.clone();
        search_target.connect_changed(move |entry| update_preview(&entry.text()));
    }
    let reset_search_form: Rc<dyn Fn()> = {
        let (editing, n, k, t, e) = (
            editing_search.clone(),
            search_name.clone(),
            search_keyword.clone(),
            search_target.clone(),
            search_error.clone(),
        );
        let (save, cancel, title) = (
            search_save.clone(),
            search_cancel.clone(),
            search_editor.clone(),
        );
        Rc::new(move || {
            *editing.borrow_mut() = None;
            n.set_text("Web Search");
            k.set_text("web");
            t.set_text("https://duckduckgo.com/?q={query}");
            set_error(&e, None);
            save.set_title(&tr("Add Web Search"));
            cancel.set_visible(false);
            title.set_title(&tr("Add a web search"));
        })
    };
    {
        let (env, editing, n, k, t, e, reset) = (
            env.clone(),
            editing_search.clone(),
            search_name.clone(),
            search_keyword.clone(),
            search_target.clone(),
            search_error.clone(),
            reset_search_form.clone(),
        );
        search_save.connect_activated(move |_| {
            let home = env.ctx.home.clone();
            let id = editing.borrow().clone();
            let (name, keyword, target) = (n.text(), k.text(), t.text());
            let outcome = env.ctx.update_settings(|s| {
                let s = &mut *s;
                save_web_search(
                    &s.quicklinks,
                    &mut s.web_searches,
                    id.as_deref(),
                    &name,
                    &keyword,
                    &target,
                    &home,
                )
            });
            match outcome {
                Ok(_) => reset(),
                Err(error_value) => set_error(&e, Some(&tr(&error_value.to_string()))),
            }
        });
    }
    {
        let reset = reset_search_form.clone();
        search_cancel.connect_activated(move |_| reset());
    }
    search_editor.add(&search_save);
    search_editor.add(&search_cancel);
    page.add(&search_editor);

    let refresh = {
        let env = env.clone();
        move || {
            let (link_list, search_list) = env
                .ctx
                .with_settings(|s| (s.quicklinks.clone(), s.web_searches.clone()));
            let signature = link_list
                .iter()
                .map(|l| format!("{}|{}|{}|{}", l.id, l.name, l.keyword, l.target))
                .collect::<Vec<_>>()
                .join("\n");
            links.update(&signature, || {
                if link_list.is_empty() {
                    return vec![placeholder_row(
                        &tr("No quicklinks yet"),
                        &tr("Add a website, file, or folder below."),
                    )];
                }
                link_list
                    .iter()
                    .map(|link| {
                        let detail = if link.keyword.is_empty() {
                            link.target.clone()
                        } else {
                            format!("{} …  ·  {}", link.keyword, link.target)
                        };
                        let row = action_row(&link.name, &detail);
                        row.add_prefix(&gtk::Image::from_icon_name("insert-link-symbolic"));
                        let edit = icon_button("document-edit-symbolic", &tr("Edit"));
                        {
                            let (editing, name, keyword, target, error, save, cancel, title, link) = (
                                editing.clone(),
                                name.clone(),
                                keyword.clone(),
                                target.clone(),
                                link_error.clone(),
                                save_row.clone(),
                                cancel_row.clone(),
                                editor_title.clone(),
                                link.clone(),
                            );
                            edit.connect_clicked(move |_| {
                                *editing.borrow_mut() = Some(link.id.clone());
                                name.set_text(&link.name);
                                keyword.set_text(&link.keyword);
                                target.set_text(&link.target);
                                set_error(&error, None);
                                save.set_title(&tr("Save Quicklink"));
                                cancel.set_visible(true);
                                title.set_title(&tr("Edit quicklink"));
                            });
                        }
                        let remove = icon_button(
                            "user-trash-symbolic",
                            &trf("Remove quicklink %@", &[FormatArg::Str(link.name.clone())]),
                        );
                        {
                            let (env, id, editing, reset) =
                                (env.clone(), link.id.clone(), editing.clone(), reset_form.clone());
                            remove.connect_clicked(move |_| {
                                if editing.borrow().as_deref() == Some(id.as_str()) {
                                    reset();
                                }
                                if let Err(m) = env.ctx.edit_settings(|s| remove_quicklink(&mut s.quicklinks, &id)) {
                                    env.toast(&m);
                                }
                            });
                        }
                        row.add_suffix(&edit);
                        row.add_suffix(&remove);
                        row.upcast()
                    })
                    .collect()
            });
            let signature = search_list
                .iter()
                .map(|s| format!("{}|{}|{}|{}", s.id, s.name, s.keyword, s.target))
                .collect::<Vec<_>>()
                .join("\n");
            searches.update(&signature, || {
                if search_list.is_empty() {
                    return vec![placeholder_row(
                        &tr("No web searches yet"),
                        &tr("Add a search template with {query} in its URL."),
                    )];
                }
                search_list
                    .iter()
                    .map(|search| {
                        let row = action_row(
                            &format!("{}  ·  {}", search.name, search.keyword),
                            &search.target,
                        );
                        row.add_prefix(&gtk::Image::from_icon_name("system-search-symbolic"));
                        let edit = icon_button("document-edit-symbolic", &tr("Edit"));
                        {
                            let (editing, n, k, t, e, save, cancel, title, search) = (
                                editing_search.clone(),
                                search_name.clone(),
                                search_keyword.clone(),
                                search_target.clone(),
                                search_error.clone(),
                                search_save.clone(),
                                search_cancel.clone(),
                                search_editor.clone(),
                                search.clone(),
                            );
                            edit.connect_clicked(move |_| {
                                *editing.borrow_mut() = Some(search.id.clone());
                                n.set_text(&search.name);
                                k.set_text(&search.keyword);
                                t.set_text(&search.target);
                                set_error(&e, None);
                                save.set_title(&tr("Save Web Search"));
                                cancel.set_visible(true);
                                title.set_title(&tr("Edit web search"));
                            });
                        }
                        let remove = icon_button(
                            "user-trash-symbolic",
                            &trf(
                                "Remove web search %@",
                                &[FormatArg::Str(search.name.clone())],
                            ),
                        );
                        {
                            let (env, id) = (env.clone(), search.id.clone());
                            remove.connect_clicked(move |_| {
                                if let Err(m) = env
                                    .ctx
                                    .edit_settings(|s| remove_web_search(&mut s.web_searches, &id))
                                {
                                    env.toast(&m);
                                }
                            });
                        }
                        row.add_suffix(&edit);
                        row.add_suffix(&remove);
                        row.upcast()
                    })
                    .collect()
            });
        }
    };
    refresh();
    Page::new(&page, refresh)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_encodes_the_sample_and_flags_invalid_templates() {
        assert_eq!(
            preview_text("https://example.com/?q={query}", "A space & tea", "/home/u"),
            "https://example.com/?q=A%20space%20%26%20tea"
        );
        assert_eq!(
            preview_text("https://{query}.example.com", "x", "/home/u"),
            "This template is invalid."
        );
    }
}
