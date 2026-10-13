//! Profiles: create, rename, switch, duplicate, export and delete (one always remains).

use std::rc::Rc;

use adw::prelude::*;
use kebloop_services::ConfigError;
use kebloop_services::config::ConfigStore;

use super::configuration::export_to_file;
use super::logic::PageId;
use super::widgets::{
    DynGroup, Env, Page, confirm, error_label, group, icon_button, note, preferences_page,
    row_button, set_error,
};
use crate::context::Change;
use crate::i18n::{tr, trf};
use kebloop_services::l10n::FormatArg;

/// Name given to a duplicated profile.
pub fn copy_name(name: &str) -> String {
    format!("{name} Copy")
}

/// Run a profile operation, tell the listeners, and report a failure in `error`.
fn run(
    env: &Rc<Env>,
    error: &gtk::Label,
    change: Change,
    operation: impl FnOnce(&mut ConfigStore) -> Result<(), ConfigError>,
) -> bool {
    let outcome = operation(&mut env.ctx.config.borrow_mut());
    match outcome {
        Ok(()) => {
            set_error(error, None);
            env.ctx.emit(change);
            true
        }
        Err(e) => {
            set_error(error, Some(&tr(&e.to_string())));
            false
        }
    }
}

/// Build the page.
pub fn build(env: &Rc<Env>) -> Page {
    let page = preferences_page();
    page.set_title(&PageId::Profiles.title());
    page.set_description(&PageId::Profiles.subtitle());
    let intro = group("", "");
    intro.add(&note(&tr(
        "Profiles keep links, aliases, shortcuts, pins, and search and appearance preferences together. Launch at login and clipboard privacy stay global.",
    )));
    page.add(&intro);
    let list = Rc::new(DynGroup::new(group(&tr("Profiles"), "")));
    page.add(&list.group);
    let error = error_label();

    let create = group(&tr("Create a profile"), "");
    let name = adw::EntryRow::builder()
        .title(tr("New profile name"))
        .show_apply_button(true)
        .build();
    {
        let (env, error) = (env.clone(), error.clone());
        name.connect_apply(move |entry| {
            let text = entry.text().to_string();
            if run(&env, &error, Change::Settings, |c| {
                c.create_profile(&text).map(|_| ())
            }) {
                entry.set_text("");
            }
        });
    }
    create.add(&name);
    create.add(&error);
    page.add(&create);

    let refresh = {
        let env = env.clone();
        move || {
            let (profiles, active) = {
                let config = env.ctx.config.borrow();
                (
                    config
                        .profiles()
                        .iter()
                        .map(|p| (p.id.clone(), p.name.clone()))
                        .collect::<Vec<_>>(),
                    config.active_profile_id().to_owned(),
                )
            };
            let signature = format!("{profiles:?}|{active}");
            list.update(&signature, || {
                let only_one = profiles.len() <= 1;
                profiles
                    .iter()
                    .map(|(id, profile_name)| {
                        profile_row(&env, id, profile_name, *id == active, only_one, &error)
                    })
                    .collect()
            });
        }
    };
    refresh();
    Page::new(&page, refresh)
}

fn profile_row(
    env: &Rc<Env>,
    id: &str,
    name: &str,
    active: bool,
    only_one: bool,
    error: &gtk::Label,
) -> gtk::Widget {
    let row = adw::EntryRow::builder()
        .title(tr("Profile name"))
        .text(name)
        .show_apply_button(true)
        .build();
    {
        let (env, id, error) = (env.clone(), id.to_owned(), error.clone());
        row.connect_apply(move |entry| {
            let text = entry.text().to_string();
            run(&env, &error, Change::Settings, |c| {
                c.rename_profile(&id, &text)
            });
        });
    }
    if active {
        let badge = gtk::Image::from_icon_name("object-select-symbolic");
        badge.set_tooltip_text(Some(&tr("Active")));
        row.add_prefix(&badge);
    } else {
        let switch = row_button(&tr("Switch"));
        let (env, id, error) = (env.clone(), id.to_owned(), error.clone());
        switch.connect_clicked(move |_| {
            run(&env, &error, Change::Profile, |c| c.switch_profile(&id));
        });
        row.add_suffix(&switch);
    }
    let duplicate = icon_button(
        "edit-copy-symbolic",
        &trf("Duplicate profile %@", &[FormatArg::Str(name.to_owned())]),
    );
    {
        let (env, id, name, error) = (env.clone(), id.to_owned(), name.to_owned(), error.clone());
        duplicate.connect_clicked(move |_| {
            run(&env, &error, Change::Settings, |c| {
                c.duplicate_profile(&id, &copy_name(&name)).map(|_| ())
            });
        });
    }
    let export = icon_button(
        "document-save-symbolic",
        &trf("Export profile %@", &[FormatArg::Str(name.to_owned())]),
    );
    {
        let (env, id) = (env.clone(), id.to_owned());
        export.connect_clicked(move |_| export_to_file(&env, Some(id.clone())));
    }
    let delete = icon_button(
        "user-trash-symbolic",
        &trf("Delete profile %@", &[FormatArg::Str(name.to_owned())]),
    );
    delete.set_sensitive(!only_one);
    {
        let (env, id, name, error) = (env.clone(), id.to_owned(), name.to_owned(), error.clone());
        delete.connect_clicked(move |_| {
            let (env2, id, error) = (env.clone(), id.clone(), error.clone());
            confirm(
                &env,
                &trf("Delete profile %@?", &[FormatArg::Str(name.clone())]),
                &tr("Its links, aliases, shortcuts, pins, and preferences will be removed."),
                &tr("Delete"),
                true,
                move || {
                    run(&env2, &error, Change::Profile, |c| c.delete_profile(&id));
                },
            );
        });
    }
    row.add_suffix(&duplicate);
    row.add_suffix(&export);
    row.add_suffix(&delete);
    row.upcast()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicates_get_a_copy_suffix() {
        assert_eq!(copy_name("Work"), "Work Copy");
    }
}
