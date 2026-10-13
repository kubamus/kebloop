//! Snippets: list, editor with placeholder hints and the expansion keyword. Keyword expansion
//! uses opt-in accessibility text events in supported applications.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use kebloop_services::snippets::{Snippet, remove_snippet, save_snippet_with_keyword};

use super::logic::PageId;
use super::widgets::{
    DynGroup, Env, Page, SwitchControl, action_row, error_label, group, icon_button, mono_label,
    note, placeholder_row, preferences_page, set_error, text_editor, text_of,
};
use crate::i18n::{tr, trf};
use kebloop_services::l10n::FormatArg;

/// Build the page.
pub fn build(env: &Rc<Env>) -> Page {
    let page = preferences_page();
    page.set_title(&PageId::Snippets.title());
    page.set_description(&PageId::Snippets.subtitle());

    let expansion = Rc::new(SwitchControl::new(
        env,
        &tr("Expand snippet keywords while typing"),
        &tr(
            "Uses accessibility APIs in focused editable text fields. Password fields are skipped. Applications without accessible editable text need the Snippets command.",
        ),
        crate::services::snippets::enabled(&env.ctx),
        {
            let ctx = env.ctx.clone();
            move |on| {
                ctx.config
                    .borrow_mut()
                    .set_snippet_expansion_enabled(on)
                    .map_err(|e| e.to_string())?;
                ctx.emit(crate::context::Change::Settings);
                Ok(())
            }
        },
    ));
    let expansion_status = note("");
    let settings = group("", "");
    settings.add(&expansion.row);
    settings.add(&expansion_status);
    page.add(&settings);

    let list = Rc::new(DynGroup::new(group(&tr("Snippets"), "")));
    page.add(&list.group);

    // Editor.
    let editing: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let editor = group(
        &tr("New Snippet"),
        &tr(
            "Use {name} for prompted values, {name:friend} for a default, and {date}, {time}, or {clipboard} for local values. Select a snippet in the launcher to preview and copy.",
        ),
    );
    let name = adw::EntryRow::builder().title(tr("Name")).build();
    editor.add(&name);
    let keyword = adw::EntryRow::builder()
        .title(tr("Expansion keyword (optional)"))
        .build();
    editor.add(&keyword);
    editor.add(&note(&tr(
        "The keyword expands after a space. Use a distinctive prefix such as ;addr to avoid accidental matches.",
    )));
    let (view, scroller) = text_editor(150, &tr("Snippet text"));
    scroller.set_margin_top(6);
    editor.add(&scroller);
    let placeholders = note("");
    editor.add(&placeholders);
    let error = error_label();
    editor.add(&error);
    {
        let placeholders = placeholders.clone();
        view.buffer().connect_changed(move |buffer| {
            let text = buffer
                .text(&buffer.start_iter(), &buffer.end_iter(), false)
                .to_string();
            let found = Snippet::new("x", text).placeholders();
            placeholders.set_label(&if found.is_empty() {
                String::new()
            } else {
                trf(
                    "Placeholders: %@",
                    &[FormatArg::Str(
                        found
                            .iter()
                            .map(|p| format!("{{{p}}}"))
                            .collect::<Vec<_>>()
                            .join(", "),
                    )],
                )
            });
            placeholders.set_visible(!found.is_empty());
        });
    }
    let buttons = gtk::Box::builder()
        .spacing(8)
        .halign(gtk::Align::End)
        .margin_top(8)
        .build();
    let save = gtk::Button::builder()
        .label(tr("Save"))
        .css_classes(["suggested-action"])
        .build();
    let new = gtk::Button::builder().label(tr("New Snippet")).build();
    buttons.append(&new);
    buttons.append(&save);
    editor.add(&buttons);
    page.add(&editor);
    let reset: Rc<dyn Fn()> = {
        let (editing, name, keyword, view, error, editor) = (
            editing.clone(),
            name.clone(),
            keyword.clone(),
            view.clone(),
            error.clone(),
            editor.clone(),
        );
        Rc::new(move || {
            *editing.borrow_mut() = None;
            name.set_text("");
            keyword.set_text("");
            view.buffer().set_text("");
            set_error(&error, None);
            editor.set_title(&tr("New Snippet"));
        })
    };
    {
        let reset = reset.clone();
        new.connect_clicked(move |_| reset());
    }
    {
        let (env, editing, name, keyword, view, error, reset) = (
            env.clone(),
            editing.clone(),
            name.clone(),
            keyword.clone(),
            view.clone(),
            error.clone(),
            reset.clone(),
        );
        save.connect_clicked(move |_| {
            let id = editing.borrow().clone();
            let (n, t, k) = (name.text(), text_of(&view), keyword.text());
            // Like macOS: a blank keyword field clears the snippet's keyword.
            let outcome = env.ctx.update_settings(|s| {
                save_snippet_with_keyword(&mut s.snippets, id.as_deref(), &n, &t, Some(&k))
            });
            match outcome {
                Ok(_) => reset(),
                Err(e) => set_error(&error, Some(&tr(&e.to_string()))),
            }
        });
    }

    let refresh = {
        let env = env.clone();
        move || {
            let ctx = &env.ctx;
            expansion.sync(crate::services::snippets::enabled(ctx));
            expansion_status.set_text(&ctx.snippet_expansion_status.borrow());
            let snippets = ctx.with_settings(|s| s.snippets.clone());
            let signature = snippets
                .iter()
                .map(|s| {
                    format!(
                        "{}|{}|{}|{}",
                        s.id,
                        s.name,
                        s.template,
                        s.keyword.as_deref().unwrap_or("")
                    )
                })
                .collect::<Vec<_>>()
                .join("\u{1}");
            list.update(&signature, || {
                if snippets.is_empty() {
                    return vec![placeholder_row(&tr("Add snippets in Settings"), "")];
                }
                snippets
                    .iter()
                    .map(|snippet| {
                        let first = snippet.template.lines().next().unwrap_or("");
                        let row = action_row(&snippet.name, first);
                        row.add_prefix(&gtk::Image::from_icon_name("insert-text-symbolic"));
                        if let Some(keyword) = &snippet.keyword {
                            let label = mono_label(keyword);
                            label.set_valign(gtk::Align::Center);
                            label.set_wrap(false);
                            label.add_css_class("dim-label");
                            row.add_suffix(&label);
                        }
                        let edit = icon_button("document-edit-symbolic", &tr("Edit"));
                        {
                            let (editing, name, keyword, view, error, editor, snippet) = (
                                editing.clone(),
                                name.clone(),
                                keyword.clone(),
                                view.clone(),
                                error.clone(),
                                editor.clone(),
                                snippet.clone(),
                            );
                            edit.connect_clicked(move |_| {
                                *editing.borrow_mut() = Some(snippet.id.clone());
                                name.set_text(&snippet.name);
                                keyword.set_text(snippet.keyword.as_deref().unwrap_or(""));
                                view.buffer().set_text(&snippet.template);
                                set_error(&error, None);
                                editor.set_title(&tr("Edit Snippet"));
                            });
                        }
                        let delete = icon_button("user-trash-symbolic", &tr("Delete"));
                        {
                            let (env, id, editing, reset) = (
                                env.clone(),
                                snippet.id.clone(),
                                editing.clone(),
                                reset.clone(),
                            );
                            delete.connect_clicked(move |_| {
                                if editing.borrow().as_deref() == Some(id.as_str()) {
                                    reset();
                                }
                                if let Err(m) = env
                                    .ctx
                                    .edit_settings(|s| remove_snippet(&mut s.snippets, &id))
                                {
                                    env.toast(&m);
                                }
                            });
                        }
                        row.add_suffix(&edit);
                        row.add_suffix(&delete);
                        row.upcast()
                    })
                    .collect()
            });
        }
    };
    refresh();
    Page::new(&page, refresh)
}
