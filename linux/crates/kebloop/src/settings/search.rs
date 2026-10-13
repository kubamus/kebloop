//! Search: categories, searchable folders, exclusions, hidden applications, empty-launcher
//! behaviour and pinned favorites.

use std::path::Path;
use std::rc::Rc;

use adw::prelude::*;
use kebloop_platform::apps::DesktopApp;
use kebloop_services::personalization::PersonalizationSettings;

use super::logic::{self, PageId, add_paths, check_path, display_path};
use super::widgets::{
    ComboControl, DynGroup, Env, Page, SwitchControl, action_row, button_row, choose_folders,
    confirm, error_label, group, icon_button, note, placeholder_row, preferences_page, set_error,
};
use crate::i18n::{tr, trf};
use kebloop_services::l10n::FormatArg;

const EMPTY_MODES: [&str; 3] = ["frequent", "recent", "favorites"];

/// Which list of paths a row edits.
#[derive(Clone, Copy)]
enum PathList {
    Folders,
    Exclusions,
}

fn list_of(settings: &mut PersonalizationSettings, which: PathList) -> &mut Vec<String> {
    match which {
        PathList::Folders => &mut settings.folders,
        PathList::Exclusions => &mut settings.exclusions,
    }
}

/// Build the page.
pub fn build(env: &Rc<Env>) -> Page {
    let ctx = env.ctx.clone();
    let page = preferences_page();
    page.set_title(&PageId::Search.title());
    page.set_description(&PageId::Search.subtitle());

    // Categories.
    let categories = group(&tr("Search categories"), "");
    let switch = |title: &str,
                  read: fn(&PersonalizationSettings) -> bool,
                  write: fn(&mut PersonalizationSettings, bool)| {
        let ctx = ctx.clone();
        let control = Rc::new(SwitchControl::new(
            env,
            title,
            "",
            ctx.with_settings(|s| read(&s.personalization)),
            move |on| ctx.edit_settings(|s| write(&mut s.personalization, on)),
        ));
        categories.add(&control.row);
        (control, read)
    };
    let switches = vec![
        switch(
            &tr("Applications"),
            |p| p.apps_enabled,
            |p, v| p.apps_enabled = v,
        ),
        switch(
            &tr("Files and folders"),
            |p| p.files_enabled,
            |p, v| p.files_enabled = v,
        ),
        switch(
            &tr("Quicklinks and web searches"),
            |p| p.links_enabled,
            |p, v| p.links_enabled = v,
        ),
        switch(
            &tr("Built-in commands"),
            |p| p.commands_enabled,
            |p, v| p.commands_enabled = v,
        ),
    ];
    page.add(&categories);

    // Folders.
    let folders = Rc::new(DynGroup::new(group(
        &tr("Searchable folders"),
        &tr("File results are limited to these folders. You can select more than one."),
    )));
    let folder_error = error_label();
    page.add(&folders.group);
    {
        let env = env.clone();
        let error = folder_error.clone();
        let add = button_row(&tr("Choose Folders…"), "folder-new-symbolic", move || {
            let env2 = env.clone();
            let error = error.clone();
            choose_folders(&env, &tr("Choose Searchable Folders"), true, move |paths| {
                let additions: Vec<String> = paths
                    .iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect();
                let mut added = 0;
                let outcome = env2.ctx.edit_settings(|s| {
                    added = add_paths(&mut s.personalization.folders, &additions);
                });
                match outcome {
                    Ok(()) => {
                        set_error(&error, None);
                        env2.toast(&if added == 0 {
                            tr("No folders were added.")
                        } else {
                            trf("Added %d folder(s).", &[FormatArg::Int(added as i64)])
                        });
                    }
                    Err(message) => set_error(&error, Some(&message)),
                }
            });
        });
        // The add row sits below the dynamic rows: its own group.
        let add_group = group("", "");
        add_group.add(&add);
        add_group.add(&folder_error);
        page.add(&add_group);
    }

    // Exclusions.
    let exclusions = Rc::new(DynGroup::new(group(
        &tr("Exclude paths"),
        &tr("Excluded folders stay out of search even when they sit inside a searchable folder."),
    )));
    page.add(&exclusions.group);
    let exclusion_error = error_label();
    let exclusion_entry = adw::EntryRow::builder()
        .title(tr("/path/to/exclude or ~/Folder"))
        .show_apply_button(true)
        .build();
    {
        let env = env.clone();
        let error = exclusion_error.clone();
        exclusion_entry.connect_apply(move |entry| {
            let home = env.ctx.home.clone();
            let existing = env
                .ctx
                .with_settings(|s| s.personalization.exclusions.clone());
            match check_path(&entry.text(), &existing, &home, &|p: &Path| p.exists()) {
                Ok(path) => match env.ctx.edit_settings(|s| {
                    add_paths(&mut s.personalization.exclusions, &[path]);
                }) {
                    Ok(()) => {
                        entry.set_text("");
                        set_error(&error, None);
                    }
                    Err(message) => set_error(&error, Some(&message)),
                },
                Err(problem) => set_error(&error, Some(&problem.message())),
            }
        });
    }
    {
        let env = env.clone();
        let choose = icon_button("folder-open-symbolic", &tr("Choose a folder to exclude"));
        choose.connect_clicked(move |_| {
            let env2 = env.clone();
            choose_folders(
                &env,
                &tr("Choose a Folder to Exclude"),
                false,
                move |paths| {
                    let additions: Vec<String> = paths
                        .iter()
                        .map(|p| p.to_string_lossy().into_owned())
                        .collect();
                    if let Err(message) = env2.ctx.edit_settings(|s| {
                        add_paths(&mut s.personalization.exclusions, &additions);
                    }) {
                        env2.toast(&message);
                    }
                },
            );
        });
        exclusion_entry.add_suffix(&choose);
    }
    let exclusion_group = group("", "");
    exclusion_group.add(&exclusion_entry);
    exclusion_group.add(&exclusion_error);
    page.add(&exclusion_group);

    // Hidden applications.
    let hidden = Rc::new(DynGroup::new(group(
        &tr("Hidden applications"),
        &tr(
            "Hide selected apps from launcher results. This does not remove app shortcuts or aliases.",
        ),
    )));
    page.add(&hidden.group);
    {
        let env = env.clone();
        let hide_group = group("", "");
        hide_group.add(&button_row(
            &tr("Choose Applications…"),
            "list-add-symbolic",
            move || choose_application(&env),
        ));
        page.add(&hide_group);
    }

    // Empty launcher.
    let empty = group(&tr("Empty launcher"), "");
    let modes = vec![
        tr("Frequent apps"),
        tr("Recent apps"),
        tr("Pinned favorites"),
    ];
    let mode_index = |ctx: &crate::context::AppContext| {
        let mode = ctx.with_settings(|s| s.personalization.empty_mode.clone());
        u32::try_from(EMPTY_MODES.iter().position(|m| *m == mode).unwrap_or(0)).unwrap_or(0)
    };
    let mode_combo = {
        let ctx = ctx.clone();
        Rc::new(ComboControl::new(
            env,
            &tr("Show when search is empty"),
            "",
            &modes,
            mode_index(&ctx),
            {
                let ctx = ctx.clone();
                move |index| {
                    let Some(mode) = EMPTY_MODES.get(index as usize) else {
                        return Ok(());
                    };
                    ctx.edit_settings(|s| s.personalization.empty_mode = (*mode).to_owned())
                }
            },
        ))
    };
    empty.add(&mode_combo.row);
    page.add(&empty);

    // Pins.
    let pins = Rc::new(DynGroup::new(group(
        &tr("Pinned favorites"),
        &tr(
            "Pin supported results with Ctrl+K in the launcher. Favorites appear here and can be shown when search is empty.",
        ),
    )));
    page.add(&pins.group);

    // Reset.
    let reset_group = group("", "");
    {
        let env = env.clone();
        let row = button_row(&tr("Reset Search Settings…"), "edit-undo-symbolic", {
            let env = env.clone();
            move || {
                let env2 = env.clone();
                confirm(
                    &env,
                    &tr("Reset search settings?"),
                    &tr(
                        "Search categories, folders, exclusions, hidden apps, empty-launcher behavior, and pinned favorites will return to their defaults.",
                    ),
                    &tr("Reset Search Settings"),
                    true,
                    move || {
                        if let Err(message) = env2.ctx.edit_settings(reset_search) {
                            env2.toast(&message);
                        }
                    },
                );
            }
        });
        row.add_css_class("destructive-action");
        reset_group.add(&row);
        let _ = env;
    }
    page.add(&reset_group);

    let refresh = {
        let env = env.clone();
        move || {
            let ctx = &env.ctx;
            for (control, read) in &switches {
                control.sync(ctx.with_settings(|s| read(&s.personalization)));
            }
            mode_combo.select(mode_index(ctx));
            let home = ctx.home.clone();
            let personalization = ctx.with_settings(|s| s.personalization.clone());

            path_rows(
                &env,
                &folders,
                PathList::Folders,
                &personalization.folders,
                &home,
                &tr("File search is off until you add a folder."),
            );
            path_rows(
                &env,
                &exclusions,
                PathList::Exclusions,
                &personalization.exclusions,
                &home,
                "",
            );

            let apps = ctx.apps.borrow().clone();
            let hidden_ids = personalization.hidden_apps.clone();
            hidden.update(&hidden_ids.join("|"), || {
                if hidden_ids.is_empty() {
                    return vec![placeholder_row(&tr("No hidden applications"), "")];
                }
                hidden_ids
                    .iter()
                    .map(|id| {
                        let name = apps
                            .iter()
                            .find(|a| a.id == *id || a.path.to_string_lossy() == id.as_str())
                            .map_or_else(|| id.clone(), |a| a.name.clone());
                        let row = action_row(&name, if name == *id { "" } else { id });
                        let remove = icon_button(
                            "user-trash-symbolic",
                            &trf("Remove %@", &[FormatArg::Str(name.clone())]),
                        );
                        let (env, id) = (env.clone(), id.clone());
                        remove.connect_clicked(move |_| {
                            if let Err(m) = env.ctx.edit_settings(|s| {
                                s.personalization.hidden_apps.retain(|h| *h != id)
                            }) {
                                env.toast(&m);
                            }
                        });
                        row.add_suffix(&remove);
                        row.upcast()
                    })
                    .collect()
            });

            let pin_list = ctx.with_settings(|s| s.pins.clone());
            let signature = pin_list
                .iter()
                .map(|p| p.id.as_str())
                .collect::<Vec<_>>()
                .join("|");
            pins.update(&signature, || {
                if pin_list.is_empty() {
                    return vec![placeholder_row(&tr("No pinned favorites yet"), "")];
                }
                pin_list
                    .iter()
                    .map(|pin| {
                        let row = action_row(&pin.name, &pin.path);
                        row.add_prefix(&gtk::Image::from_icon_name(pin_icon(&pin.kind)));
                        let remove = icon_button(
                            "edit-delete-symbolic",
                            &trf(
                                "Remove %@ from pinned favorites",
                                &[FormatArg::Str(pin.name.clone())],
                            ),
                        );
                        let (env, id) = (env.clone(), pin.id.clone());
                        remove.connect_clicked(move |_| {
                            if let Err(m) = env.ctx.edit_settings(|s| s.pins.retain(|p| p.id != id))
                            {
                                env.toast(&m);
                            }
                        });
                        row.add_suffix(&remove);
                        row.upcast()
                    })
                    .collect()
            });
        }
    };
    refresh();
    let _ = note;
    Page::new(&page, refresh)
}

fn pin_icon(kind: &str) -> &'static str {
    match kind {
        "application" => "application-x-executable-symbolic",
        "file" => "text-x-generic-symbolic",
        "quicklink" => "insert-link-symbolic",
        _ => "utilities-terminal-symbolic",
    }
}

fn path_rows(
    env: &Rc<Env>,
    dynamic: &DynGroup,
    which: PathList,
    paths: &[String],
    home: &str,
    empty_text: &str,
) {
    let signature = paths.join("\u{1}");
    dynamic.update(&signature, || {
        if paths.is_empty() {
            return if empty_text.is_empty() {
                Vec::new()
            } else {
                vec![placeholder_row(empty_text, "")]
            };
        }
        paths
            .iter()
            .map(|path| {
                let expanded = kebloop_services::util::expand_tilde(path, home);
                let exists = Path::new(&expanded).exists();
                let row = action_row(&display_path(path, home), "");
                row.add_prefix(&gtk::Image::from_icon_name(if exists {
                    "folder-symbolic"
                } else {
                    "dialog-warning-symbolic"
                }));
                if !exists {
                    row.set_subtitle(&tr("This folder is unavailable."));
                }
                let remove = icon_button(
                    "user-trash-symbolic",
                    &trf("Remove %@", &[FormatArg::Str(path.clone())]),
                );
                let (env, path) = (env.clone(), path.clone());
                remove.connect_clicked(move |_| {
                    if let Err(message) = env.ctx.edit_settings(|s| {
                        list_of(&mut s.personalization, which).retain(|p| *p != path);
                    }) {
                        env.toast(&message);
                    }
                });
                row.add_suffix(&remove);
                row.upcast()
            })
            .collect()
    });
}

/// Restore the search-related preferences, as "Reset Search Settings" does on macOS.
pub fn reset_search(settings: &mut kebloop_services::config::PortableSettings) {
    let defaults = PersonalizationSettings::default();
    let p = &mut settings.personalization;
    p.apps_enabled = defaults.apps_enabled;
    p.files_enabled = defaults.files_enabled;
    p.links_enabled = defaults.links_enabled;
    p.commands_enabled = defaults.commands_enabled;
    p.folders = defaults.folders;
    p.exclusions = defaults.exclusions;
    p.hidden_apps = defaults.hidden_apps;
    p.empty_mode = defaults.empty_mode;
    settings.pins.clear();
}

/// A searchable list of installed applications; choosing one hides it from the launcher.
fn choose_application(env: &Rc<Env>) {
    let dialog = adw::Dialog::builder()
        .title(tr("Choose Applications to Hide"))
        .content_width(440)
        .content_height(520)
        .build();
    let search = gtk::SearchEntry::builder()
        .placeholder_text(tr("Search applications"))
        .margin_start(12)
        .margin_end(12)
        .margin_bottom(8)
        .build();
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .margin_start(12)
        .margin_end(12)
        .margin_bottom(12)
        .build();
    let scroller = gtk::ScrolledWindow::builder()
        .child(&list)
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();
    let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    body.append(&search);
    body.append(&scroller);
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(&body));
    dialog.set_child(Some(&toolbar));

    let apps: Vec<DesktopApp> = env.ctx.apps.borrow().clone();
    let fill = {
        let list = list.clone();
        let env = env.clone();
        let dialog = dialog.clone();
        Rc::new(move |query: &str| {
            while let Some(child) = list.first_child() {
                list.remove(&child);
            }
            let hidden = env
                .ctx
                .with_settings(|s| s.personalization.hidden_apps.clone());
            let (shown, _) = logic::filter_apps(&apps, query, 60);
            for app in shown.into_iter().filter(|a| !hidden.contains(&a.id)) {
                let row = action_row(&app.name, "");
                let image = gtk::Image::new();
                crate::icons::apply(
                    &image,
                    &crate::launcher::model::IconSpec::App(app.icon.clone()),
                    24,
                );
                row.add_prefix(&image);
                row.set_activatable(true);
                let (env, id, dialog) = (env.clone(), app.id.clone(), dialog.clone());
                row.connect_activated(move |_| {
                    match env.ctx.edit_settings(|s| {
                        if !s.personalization.hidden_apps.contains(&id) {
                            s.personalization.hidden_apps.push(id.clone());
                        }
                    }) {
                        Ok(()) => {
                            dialog.close();
                        }
                        Err(message) => env.toast(&message),
                    }
                });
                list.append(&row);
            }
        })
    };
    fill("");
    {
        let fill = fill.clone();
        search.connect_search_changed(move |entry| fill(&entry.text()));
    }
    env.present(&dialog);
    search.grab_focus();
}

#[cfg(test)]
mod tests {
    use super::*;
    use kebloop_services::config::PortableSettings;
    use kebloop_services::personalization::PinnedResult;

    #[test]
    fn reset_restores_search_defaults_and_clears_pins_but_keeps_appearance() {
        let mut settings = PortableSettings::default();
        settings.personalization.files_enabled = false;
        settings.personalization.folders = vec!["/srv".into()];
        settings.personalization.exclusions = vec!["/srv/x".into()];
        settings.personalization.hidden_apps = vec!["a.desktop".into()];
        settings.personalization.empty_mode = "recent".into();
        settings.personalization.appearance = "dark".into();
        settings.pins.push(PinnedResult {
            id: "file:/x".into(),
            name: "x".into(),
            path: "/x".into(),
            kind: "file".into(),
        });
        reset_search(&mut settings);
        let defaults = PersonalizationSettings::default();
        assert_eq!(settings.personalization.folders, defaults.folders);
        assert!(settings.personalization.files_enabled);
        assert!(settings.personalization.exclusions.is_empty());
        assert!(settings.personalization.hidden_apps.is_empty());
        assert_eq!(settings.personalization.empty_mode, "frequent");
        assert_eq!(settings.personalization.appearance, "dark");
        assert!(settings.pins.is_empty());
        settings.validate().unwrap();
    }
}
