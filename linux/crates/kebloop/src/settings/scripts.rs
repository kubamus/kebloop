//! Script commands: reviewed local executables with literal arguments.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use kebloop_services::automation::{ScriptCommand, remove_script, save_script, set_script_enabled};
use kebloop_services::util::new_id;

use super::logic::{PageId, arguments_from_text, arguments_to_text};
use super::widgets::{
    DynGroup, Env, Page, SpinControl, action_row, choose_file, confirm, error_label, group,
    icon_button, note, placeholder_row, preferences_page, set_error, text_editor, text_of,
};
use crate::i18n::{tr, trf};
use kebloop_services::l10n::FormatArg;

/// One line describing what a script would run.
pub fn command_line(command: &ScriptCommand) -> String {
    std::iter::once(command.executable.as_str())
        .chain(command.arguments.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Build the page.
pub fn build(env: &Rc<Env>) -> Page {
    let page = preferences_page();
    page.set_title(&PageId::Scripts.title());
    page.set_description(&PageId::Scripts.subtitle());
    page.add(&{
        let warning = group("", "");
        warning.add(&note(&tr(
            "Scripts run with your user privileges. Review the executable and arguments before enabling. Arguments and metadata are portable; never enter secrets here.",
        )));
        warning
    });
    let list = Rc::new(DynGroup::new(group(&tr("Script Commands"), "")));
    page.add(&list.group);

    let editing: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let editor = group(&tr("Add disabled script"), "");
    let title = adw::EntryRow::builder().title(tr("Title")).build();
    let description = adw::EntryRow::builder().title(tr("Description")).build();
    let executable = adw::EntryRow::builder()
        .title(tr("Absolute executable or runtime path"))
        .build();
    let choose = icon_button("document-open-symbolic", &tr("Choose…"));
    {
        let (env, executable) = (env.clone(), executable.clone());
        choose.connect_clicked(move |_| {
            let executable = executable.clone();
            choose_file(&env, &tr("Choose…"), move |path| {
                executable.set_text(&path.to_string_lossy());
            });
        });
    }
    executable.add_suffix(&choose);
    editor.add(&title);
    editor.add(&description);
    editor.add(&executable);
    editor.add(&note(&tr(
        "Arguments: one literal argument per line. For an interpreted script, make its absolute path the first argument.",
    )));
    let (arguments, scroller) = text_editor(80, &tr("Script arguments, one per line"));
    editor.add(&scroller);
    let timeout = Rc::new(SpinControl::new(
        env,
        &tr("Timeout (seconds)"),
        "",
        (1.0, 300.0, 1.0),
        10.0,
        |_| Ok(()),
    ));
    editor.add(&timeout.row);
    let error = error_label();
    editor.add(&error);
    let save = gtk::Button::builder()
        .label(tr("Add disabled script"))
        .css_classes(["suggested-action"])
        .halign(gtk::Align::End)
        .margin_top(8)
        .build();
    let cancel = gtk::Button::builder()
        .label(tr("Cancel"))
        .visible(false)
        .build();
    let buttons = gtk::Box::builder()
        .spacing(8)
        .halign(gtk::Align::End)
        .margin_top(8)
        .build();
    buttons.append(&cancel);
    buttons.append(&save);
    save.set_margin_top(0);
    editor.add(&buttons);
    page.add(&editor);

    let reset: Rc<dyn Fn()> = {
        let (
            editing,
            title,
            description,
            executable,
            arguments,
            timeout,
            error,
            save,
            cancel,
            editor,
        ) = (
            editing.clone(),
            title.clone(),
            description.clone(),
            executable.clone(),
            arguments.clone(),
            timeout.clone(),
            error.clone(),
            save.clone(),
            cancel.clone(),
            editor.clone(),
        );
        Rc::new(move || {
            *editing.borrow_mut() = None;
            title.set_text("");
            description.set_text("");
            executable.set_text("");
            arguments.buffer().set_text("");
            timeout.sync(10.0);
            set_error(&error, None);
            save.set_label(&tr("Add disabled script"));
            cancel.set_visible(false);
            editor.set_title(&tr("Add disabled script"));
        })
    };
    {
        let reset = reset.clone();
        cancel.connect_clicked(move |_| reset());
    }
    {
        let (env, editing, title, description, executable, arguments, timeout, error, reset) = (
            env.clone(),
            editing.clone(),
            title.clone(),
            description.clone(),
            executable.clone(),
            arguments.clone(),
            timeout.clone(),
            error.clone(),
            reset.clone(),
        );
        save.connect_clicked(move |_| {
            let id = editing.borrow().clone().unwrap_or_else(new_id);
            let mut command = ScriptCommand::new(
                id,
                title.text().trim(),
                executable.text().trim(),
                arguments_from_text(&text_of(&arguments)),
            );
            command.description = description.text().trim().to_owned();
            command.timeout = timeout.row.value();
            // A saved or edited script always needs a fresh review.
            command.enabled = false;
            let outcome = env
                .ctx
                .update_settings(|s| save_script(&mut s.scripts, command));
            match outcome {
                Ok(()) => reset(),
                Err(e) => set_error(&error, Some(&tr(&e.to_string()))),
            }
        });
    }

    let refresh = {
        let env = env.clone();
        move || {
            let scripts = env.ctx.with_settings(|s| s.scripts.clone());
            let signature = scripts
                .iter()
                .map(|c| {
                    format!(
                        "{}|{}|{}|{}|{:?}|{}",
                        c.id, c.title, c.enabled, c.executable, c.arguments, c.timeout
                    )
                })
                .collect::<Vec<_>>()
                .join("\u{1}");
            list.update(&signature, || {
                if scripts.is_empty() {
                    return vec![placeholder_row(&tr("Add script commands in Settings"), "")];
                }
                scripts
                    .iter()
                    .map(|command| {
                        script_row(
                            &env,
                            command,
                            &editing,
                            &title,
                            &description,
                            &executable,
                            &arguments,
                            &timeout,
                            &error,
                            &save,
                            &cancel,
                            &editor,
                        )
                    })
                    .collect()
            });
        }
    };
    refresh();
    Page::new(&page, refresh)
}

#[allow(clippy::too_many_arguments)]
fn script_row(
    env: &Rc<Env>,
    command: &ScriptCommand,
    editing: &Rc<RefCell<Option<String>>>,
    title: &adw::EntryRow,
    description: &adw::EntryRow,
    executable: &adw::EntryRow,
    arguments: &gtk::TextView,
    timeout: &Rc<SpinControl>,
    error: &gtk::Label,
    save: &gtk::Button,
    cancel: &gtk::Button,
    editor: &adw::PreferencesGroup,
) -> gtk::Widget {
    let row = action_row(&command.title, &command_line(command));
    row.set_subtitle_lines(2);
    // Enabling goes through a review dialog instead of flipping the switch directly.
    let toggle = gtk::Switch::builder()
        .valign(gtk::Align::Center)
        .active(command.enabled)
        .build();
    {
        let (env, command) = (env.clone(), command.clone());
        let reverting = Rc::new(std::cell::Cell::new(false));
        toggle.connect_state_set(move |toggle, wanted| {
            if reverting.get() {
                return gtk::glib::Propagation::Proceed;
            }
            let id = command.id.clone();
            if !wanted {
                let outcome = env
                    .ctx
                    .edit_settings(|s| {
                        set_script_enabled(&mut s.scripts, &id, false);
                    });
                if let Err(message) = outcome {
                    env.toast(&message);
                    return gtk::glib::Propagation::Stop;
                }
                return gtk::glib::Propagation::Proceed;
            }
            // Enabling needs a review: undo the visual change until confirmed.
            reverting.set(true);
            toggle.set_active(false);
            reverting.set(false);
            let env2 = env.clone();
            confirm(
                &env,
                &trf("Enable %@?", &[FormatArg::Str(command.title.clone())]),
                &format!(
                    "{}\n\n{}\n{}",
                    tr("Scripts run with your user privileges. Review the executable and arguments before enabling."),
                    command_line(&command),
                    trf("Timeout (seconds): %d", &[FormatArg::Int(command.timeout as i64)]),
                ),
                &tr("Enable"),
                false,
                move || {
                    if let Err(message) = env2.ctx.edit_settings(|s| {
                        set_script_enabled(&mut s.scripts, &id, true);
                    }) {
                        env2.toast(&message);
                    }
                },
            );
            gtk::glib::Propagation::Stop
        });
    }
    row.add_suffix(&toggle);
    let edit = icon_button("document-edit-symbolic", &tr("Edit"));
    {
        let (
            editing,
            title,
            description,
            executable,
            arguments,
            timeout,
            error,
            save,
            cancel,
            editor,
            command,
        ) = (
            editing.clone(),
            title.clone(),
            description.clone(),
            executable.clone(),
            arguments.clone(),
            timeout.clone(),
            error.clone(),
            save.clone(),
            cancel.clone(),
            editor.clone(),
            command.clone(),
        );
        edit.connect_clicked(move |_| {
            *editing.borrow_mut() = Some(command.id.clone());
            title.set_text(&command.title);
            description.set_text(&command.description);
            executable.set_text(&command.executable);
            arguments
                .buffer()
                .set_text(&arguments_to_text(&command.arguments));
            timeout.sync(command.timeout);
            set_error(&error, None);
            save.set_label(&tr("Save script"));
            cancel.set_visible(true);
            editor.set_title(&tr("Edit script"));
        });
    }
    let remove = icon_button("user-trash-symbolic", &tr("Remove"));
    {
        let (env, id) = (env.clone(), command.id.clone());
        remove.connect_clicked(move |_| {
            if let Err(m) = env
                .ctx
                .edit_settings(|s| remove_script(&mut s.scripts, &id))
            {
                env.toast(&m);
            }
        });
    }
    row.add_suffix(&edit);
    row.add_suffix(&remove);
    row.upcast()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_line_lists_executable_then_arguments() {
        let command = ScriptCommand::new(
            "a",
            "T",
            "/usr/bin/env",
            vec!["python3".into(), "/x.py".into()],
        );
        assert_eq!(command_line(&command), "/usr/bin/env python3 /x.py");
    }
}
