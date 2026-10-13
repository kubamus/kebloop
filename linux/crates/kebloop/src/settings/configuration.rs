//! Configuration: export to a JSON file and import with a reviewable preview.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use adw::prelude::*;
use kebloop_services::config::{
    ConfigurationPreview, ImportMode, MAX_IMPORT_BYTES, UnavailableItem, UnavailableKind,
};

use super::logic::PageId;
use super::widgets::{
    Env, Page, action_row, button_row, choose_file, choose_save_path, confirm, error_label, group,
    note, preferences_page, set_error,
};
use crate::context::Change;
use crate::i18n::{tr, trf};
use kebloop_services::l10n::FormatArg;

/// Export the configuration (one profile, or all when `profile` is `None`) after asking where.
pub fn export_to_file(env: &Rc<Env>, profile: Option<String>) {
    let data = env.ctx.config.borrow().export_data(profile.as_deref());
    let data = match data {
        Ok(data) => data,
        Err(error) => {
            env.toast(&tr(&error.to_string()));
            return;
        }
    };
    let name = if profile.is_some() {
        "Kebloop-profile.json"
    } else {
        "Kebloop-configuration.json"
    };
    let env2 = env.clone();
    choose_save_path(
        env,
        &tr("Export Configuration…"),
        name,
        move |path| match std::fs::write(&path, &data) {
            Ok(()) => env2.toast(&trf(
                "Configuration exported to %@.",
                &[FormatArg::Str(file_name(&path))],
            )),
            Err(error) => env2.toast(&format!("{} {error}", tr("Could not write the file."))),
        },
    );
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.to_string_lossy().into_owned(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// One localized line for an unavailable imported item.
pub fn unavailable_line(item: &UnavailableItem) -> String {
    let kind = match item.kind {
        UnavailableKind::MissingPath => tr("Missing path"),
        UnavailableKind::RestrictedPath => tr("Path is not readable"),
        UnavailableKind::AppleShortcut => tr("Apple Shortcut (macOS only)"),
        UnavailableKind::Shortcut => tr("Shortcut without a Linux equivalent"),
    };
    format!("{kind}: {}", item.subject)
}

/// Read an import file, refusing oversized ones before loading them.
pub fn read_import(path: &Path) -> Result<Vec<u8>, String> {
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if meta.len() > MAX_IMPORT_BYTES as u64 {
        return Err(tr("Configuration file is too large."));
    }
    std::fs::read(path).map_err(|e| e.to_string())
}

/// Build the page.
pub fn build(env: &Rc<Env>) -> Page {
    let page = preferences_page();
    page.set_title(&PageId::Configuration.title());
    page.set_description(&PageId::Configuration.subtitle());
    let files = group(&tr("Configuration"), "");
    files.add(&note(&tr(
        "Export aliases, quicklinks, shortcuts, pins, search and appearance settings, and profiles. Clipboard contents, credentials, caches, and usage history are not included.",
    )));
    {
        let env = env.clone();
        files.add(&button_row(
            &tr("Export Configuration…"),
            "document-save-symbolic",
            move || export_to_file(&env, None),
        ));
    }
    let feedback = error_label();
    let preview_group = group(&tr("Import preview"), "");
    preview_group.set_visible(false);
    let pending: Rc<RefCell<Option<ConfigurationPreview>>> = Rc::new(RefCell::new(None));
    let rows: Rc<RefCell<Vec<gtk::Widget>>> = Rc::new(RefCell::new(Vec::new()));
    let clear: Rc<dyn Fn()> = {
        let (pending, rows, preview_group) = (pending.clone(), rows.clone(), preview_group.clone());
        Rc::new(move || {
            *pending.borrow_mut() = None;
            for row in rows.borrow_mut().drain(..) {
                preview_group.remove(&row);
            }
            preview_group.set_visible(false);
        })
    };
    let show_preview: Rc<dyn Fn(Option<ConfigurationPreview>)> = {
        let (env, pending, rows, preview_group, clear) = (
            env.clone(),
            pending.clone(),
            rows.clone(),
            preview_group.clone(),
            clear.clone(),
        );
        let feedback = feedback.clone();
        Rc::new(move |preview| {
            clear();
            let Some(preview) = preview else {
                return;
            };
            preview_group.set_visible(true);
            let mut added: Vec<gtk::Widget> = Vec::new();
            for line in &preview.summary {
                let row = action_row(line, "");
                row.add_prefix(&gtk::Image::from_icon_name("object-select-symbolic"));
                added.push(row.upcast());
            }
            if !preview.warnings.is_empty() || !preview.unavailable.is_empty() {
                let heading = gtk::Label::builder()
                    .label(tr("Review before applying"))
                    .xalign(0.0)
                    .css_classes(["heading"])
                    .margin_top(8)
                    .build();
                added.push(heading.upcast());
            }
            for warning in &preview.warnings {
                added.push(note(&format!("• {warning}")).upcast());
            }
            for item in &preview.unavailable {
                added.push(note(&format!("• {}", unavailable_line(item))).upcast());
            }
            let merge = button_row(&tr("Merge"), "list-add-symbolic", {
                let (env, pending, feedback, clear) = (
                    env.clone(),
                    pending.clone(),
                    feedback.clone(),
                    clear.clone(),
                );
                move || apply(&env, &pending, ImportMode::Merge, &feedback, &clear)
            });
            merge.add_css_class("suggested-action");
            let replace = button_row(&tr("Replace Existing Settings…"), "edit-undo-symbolic", {
                let (env, pending, feedback, clear) = (
                    env.clone(),
                    pending.clone(),
                    feedback.clone(),
                    clear.clone(),
                );
                move || {
                    let (env2, pending, feedback, clear) = (
                        env.clone(),
                        pending.clone(),
                        feedback.clone(),
                        clear.clone(),
                    );
                    confirm(
                        &env,
                        &tr("Replace existing settings?"),
                        &tr(
                            "Your current configuration is saved to a backup file first, then replaced by the imported one.",
                        ),
                        &tr("Replace"),
                        true,
                        move || apply(&env2, &pending, ImportMode::Replace, &feedback, &clear),
                    );
                }
            });
            replace.add_css_class("destructive-action");
            let cancel = button_row(&tr("Cancel"), "window-close-symbolic", {
                let (clear, feedback) = (clear.clone(), feedback.clone());
                move || {
                    clear();
                    set_error(&feedback, None);
                }
            });
            added.push(merge.upcast());
            added.push(replace.upcast());
            added.push(cancel.upcast());
            added.push(
                note(&tr(
                    "Merge adds items with new IDs and keeps existing values on conflicts. Replace saves a backup before applying imported settings.",
                ))
                .upcast(),
            );
            for row in &added {
                preview_group.add(row);
            }
            *rows.borrow_mut() = added;
            *pending.borrow_mut() = Some(preview);
        })
    };
    {
        let (env, feedback, show) = (env.clone(), feedback.clone(), show_preview.clone());
        files.add(&button_row(
            &tr("Import Configuration…"),
            "document-open-symbolic",
            move || {
                let (env, feedback, show) = (env.clone(), feedback.clone(), show.clone());
                choose_file(
                    &env.clone(),
                    &tr("Import Kebloop Configuration"),
                    move |path| {
                        let preview = read_import(&path).and_then(|bytes| {
                            env.ctx
                                .config
                                .borrow()
                                .preview(&bytes)
                                .map_err(|e| tr(&e.to_string()))
                        });
                        match preview {
                            Ok(preview) => {
                                set_error(&feedback, None);
                                show(Some(preview));
                            }
                            Err(message) => {
                                show(None);
                                set_error(&feedback, Some(&message));
                            }
                        }
                    },
                );
            },
        ));
    }
    files.add(&feedback);
    page.add(&files);
    page.add(&preview_group);
    Page::fixed(&page)
}

fn apply(
    env: &Rc<Env>,
    pending: &Rc<RefCell<Option<ConfigurationPreview>>>,
    mode: ImportMode,
    feedback: &gtk::Label,
    clear: &Rc<dyn Fn()>,
) {
    let Some(preview) = pending.borrow().clone() else {
        return;
    };
    let outcome = env.ctx.config.borrow_mut().apply(&preview, mode, None);
    match outcome {
        Ok(backup) => {
            clear();
            set_error(feedback, None);
            env.ctx.emit(Change::Profile);
            env.ctx.emit(Change::Settings);
            env.toast(&match backup {
                Some(path) => trf(
                    "Configuration replaced. Backup saved to %@.",
                    &[FormatArg::Str(path.to_string_lossy().into_owned())],
                ),
                None => tr("Configuration merged."),
            });
        }
        Err(error) => set_error(feedback, Some(&tr(&error.to_string()))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_items_name_their_kind_and_subject() {
        let item = UnavailableItem {
            kind: UnavailableKind::MissingPath,
            subject: "/gone".into(),
        };
        assert_eq!(unavailable_line(&item), "Missing path: /gone");
    }

    #[test]
    fn oversized_import_files_are_refused_without_reading_them() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.json");
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(MAX_IMPORT_BYTES as u64 + 1).unwrap();
        assert!(read_import(&path).is_err());
        std::fs::write(&path, b"{}").unwrap();
        assert_eq!(read_import(&path).unwrap(), b"{}");
        assert!(read_import(&dir.path().join("missing.json")).is_err());
    }
}
