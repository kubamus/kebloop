//! Selective Raycast export migration and reviewed Script Command / JavaScript import UI.

use std::cell::{Cell, RefCell};
use std::fs;
use std::path::PathBuf;
use std::rc::{Rc, Weak};

use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use kebloop_services::automation::{
    AutomationError, AutomationRunner, ScriptCommand, import_scripts,
};
use kebloop_services::raycast::{
    RaycastImportItem, RaycastSdkCommand, RaycastSdkEffect, RaycastSdkPreference, execute_sdk,
    import_selected, inspect_script, preview_export,
};
use kebloop_services::util::atomic_write;

use crate::context::AppContext;
use crate::i18n::tr;

/// GTK page for choosing supported records and preparing disabled Raycast-compatible commands.
#[derive(Clone)]
pub struct RaycastPage {
    inner: Rc<Inner>,
}

struct Inner {
    ctx: Weak<AppContext>,
    root: gtk::Box,
    status: gtk::Label,
    rows: RefCell<Vec<(RaycastImportItem, gtk::CheckButton)>>,
    rows_box: gtk::Box,
    rows_scroll: gtk::ScrolledWindow,
    import_rows: gtk::Button,
    script_candidate: RefCell<Option<(ScriptCommand, PathBuf)>>,
    script_summary: gtk::Label,
    script_review: gtk::Button,
    script_import: gtk::Button,
    sdk_manifest: gtk::Entry,
    sdk_command_name: gtk::Entry,
    sdk_entry: gtk::Entry,
    sdk_runtime: gtk::Entry,
    sdk_preferences: gtk::Box,
    sdk_preference_entries: RefCell<Vec<(RaycastSdkPreference, gtk::Entry)>>,
    sdk_summary: gtk::Label,
    sdk_review: gtk::Button,
    sdk_import: gtk::Button,
    sdk_installed: gtk::Box,
    sdk_store_path: PathBuf,
    sdk_commands: RefCell<Vec<RaycastSdkCommand>>,
    sdk_candidate: RefCell<Option<RaycastSdkCommand>>,
}

impl RaycastPage {
    /// Create the page and load local SDK compatibility commands from the XDG data directory.
    pub fn new(ctx: &Rc<AppContext>) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 10);
        content.set_margin_start(16);
        content.set_margin_end(16);
        content.set_margin_top(12);
        content.set_margin_bottom(12);

        let title = gtk::Label::builder()
            .label(tr("Raycast Migration and Compatibility"))
            .xalign(0.0)
            .css_classes(["kb-page-title"])
            .build();
        let intro = gtk::Label::builder()
            .label(tr("Preview supported Raycast exports and review commands before importing. All imported commands start disabled."))
            .xalign(0.0)
            .wrap(true)
            .build();
        content.append(&title);
        content.append(&intro);

        let status = gtk::Label::builder()
            .xalign(0.0)
            .wrap(true)
            .selectable(true)
            .hexpand(true)
            .visible(false)
            .build();
        status.add_css_class("dim-label");

        let migration_heading = section_heading("Import Raycast Export");
        let migration_hint = helper_text(
            "Choose a snippets or quicklinks JSON export. Select individual valid rows; existing Kebloop items are preserved.",
        );
        let choose_export = gtk::Button::with_label(&tr("Choose Raycast Export…"));
        choose_export.set_halign(gtk::Align::Start);
        let rows_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let rows_scroll = gtk::ScrolledWindow::builder()
            .child(&rows_box)
            .max_content_height(190)
            .min_content_height(20)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .visible(false)
            .build();
        rows_scroll.add_css_class("card");
        let import_rows = gtk::Button::with_label(&tr("Import Selected Items"));
        import_rows.set_halign(gtk::Align::End);
        import_rows.set_sensitive(false);
        content.append(&migration_heading);
        content.append(&migration_hint);
        content.append(&choose_export);
        content.append(&rows_scroll);
        content.append(&import_rows);
        content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));

        let script_heading = section_heading("Raycast Script Commands");
        let script_hint = helper_text(
            "Read @raycast metadata without running the script. Review its source, then import it disabled into Script Commands.",
        );
        let choose_script = gtk::Button::with_label(&tr("Choose Raycast Script…"));
        choose_script.set_halign(gtk::Align::Start);
        let script_summary = gtk::Label::builder()
            .xalign(0.0)
            .wrap(true)
            .selectable(true)
            .visible(false)
            .build();
        let script_actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let script_review = gtk::Button::with_label(&tr("Review Source"));
        script_review.set_sensitive(false);
        let script_import = gtk::Button::with_label(&tr("Import Disabled Script"));
        script_import.set_sensitive(false);
        script_actions.append(&script_review);
        script_actions.append(&script_import);
        content.append(&script_heading);
        content.append(&script_hint);
        content.append(&choose_script);
        content.append(&script_summary);
        content.append(&script_actions);
        content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));

        let sdk_heading = section_heading("Raycast JavaScript Extensions");
        let sdk_hint = helper_text(
            "Import compiled standalone .js or .mjs no-view commands. The compatibility runtime supports plain-text clipboard copy, static feedback, text preferences and HTTP(S) open.",
        );
        content.append(&sdk_heading);
        content.append(&sdk_hint);
        content.append(&helper_text(
            "Text preferences are stored locally as plain text; do not enter secrets.",
        ));

        let (sdk_manifest, sdk_manifest_picker) =
            labeled_path_row("Raycast package.json", "Choose…", &content);
        let sdk_command_name = gtk::Entry::builder()
            .placeholder_text(tr("Command name from package.json"))
            .hexpand(true)
            .build();
        sdk_command_name.update_property(&[gtk::accessible::Property::Label(&tr(
            "Raycast command name",
        ))]);
        let command_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        command_row.append(
            &gtk::Label::builder()
                .label(tr("Command name"))
                .xalign(0.0)
                .width_chars(19)
                .build(),
        );
        command_row.append(&sdk_command_name);
        content.append(&command_row);
        let (sdk_entry, sdk_entry_picker) =
            labeled_path_row("Compiled JavaScript entry", "Choose…", &content);
        let (sdk_runtime, sdk_runtime_picker) =
            labeled_path_row("Installed node or bun executable", "Choose…", &content);
        let sdk_preview = gtk::Button::with_label(&tr("Preview JavaScript Command"));
        sdk_preview.set_halign(gtk::Align::Start);
        content.append(&sdk_preview);
        let sdk_preferences = gtk::Box::new(gtk::Orientation::Vertical, 4);
        content.append(&sdk_preferences);
        let sdk_summary = gtk::Label::builder()
            .xalign(0.0)
            .wrap(true)
            .selectable(true)
            .visible(false)
            .build();
        let sdk_candidate_actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let sdk_review = gtk::Button::with_label(&tr("Review Source"));
        sdk_review.set_sensitive(false);
        let sdk_import = gtk::Button::with_label(&tr("Import Disabled JavaScript Command"));
        sdk_import.set_halign(gtk::Align::End);
        sdk_import.set_sensitive(false);
        sdk_candidate_actions.append(&sdk_review);
        sdk_candidate_actions.append(&sdk_import);
        let sdk_installed_heading = section_heading("Installed Raycast JavaScript Commands");
        let sdk_installed = gtk::Box::new(gtk::Orientation::Vertical, 6);
        content.append(&sdk_summary);
        content.append(&sdk_candidate_actions);
        content.append(&sdk_installed_heading);
        content.append(&sdk_installed);
        content.append(&status);
        let page_scroll = gtk::ScrolledWindow::builder()
            .child(&content)
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();
        root.append(&page_scroll);

        let sdk_store_path = ctx.xdg.app_data_dir().join("raycast-sdk-commands.json");
        let (sdk_commands, load_error) = load_sdk_commands(&sdk_store_path);

        let inner = Rc::new(Inner {
            ctx: Rc::downgrade(ctx),
            root,
            status,
            rows: RefCell::new(Vec::new()),
            rows_box,
            rows_scroll,
            import_rows,
            script_candidate: RefCell::new(None),
            script_summary,
            script_review,
            script_import,
            sdk_manifest: sdk_manifest.clone(),
            sdk_command_name,
            sdk_entry: sdk_entry.clone(),
            sdk_runtime: sdk_runtime.clone(),
            sdk_preferences,
            sdk_preference_entries: RefCell::new(Vec::new()),
            sdk_summary,
            sdk_review,
            sdk_import,
            sdk_installed,
            sdk_store_path,
            sdk_commands: RefCell::new(sdk_commands),
            sdk_candidate: RefCell::new(None),
        });

        connect_export_picker(&inner, &choose_export);
        connect_import_rows(&inner);
        connect_script_picker(&inner, &choose_script);
        connect_script_review(&inner);
        connect_script_import(&inner);
        connect_path_picker(
            &inner,
            &sdk_manifest,
            &sdk_manifest_picker,
            "Choose Raycast package.json",
        );
        connect_path_picker(
            &inner,
            &sdk_entry,
            &sdk_entry_picker,
            "Choose compiled JavaScript command",
        );
        connect_path_picker(
            &inner,
            &sdk_runtime,
            &sdk_runtime_picker,
            "Choose installed node or bun executable",
        );
        connect_sdk_preview(&inner, &sdk_preview);
        connect_sdk_import(&inner);
        connect_sdk_review(&inner);
        render_sdk_commands(&inner);
        if let Some(error) = load_error {
            show_status(&inner, &error);
        }

        Self { inner }
    }

    /// Page widget for the launcher's stack.
    pub fn widget(&self) -> &gtk::Box {
        &self.inner.root
    }
}

fn section_heading(key: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(tr(key))
        .xalign(0.0)
        .css_classes(["heading"])
        .build()
}

fn helper_text(key: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(tr(key))
        .xalign(0.0)
        .wrap(true)
        .css_classes(["dim-label"])
        .build()
}

fn labeled_path_row(key: &str, button_label: &str, parent: &gtk::Box) -> (gtk::Entry, gtk::Button) {
    let entry = gtk::Entry::builder().hexpand(true).build();
    entry.update_property(&[gtk::accessible::Property::Label(&tr(key))]);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.append(
        &gtk::Label::builder()
            .label(tr(key))
            .xalign(0.0)
            .width_chars(19)
            .build(),
    );
    row.append(&entry);
    let button = gtk::Button::with_label(&tr(button_label));
    row.append(&button);
    parent.append(&row);
    (entry, button)
}

fn connect_path_picker(inner: &Rc<Inner>, entry: &gtk::Entry, button: &gtk::Button, title: &str) {
    let weak = Rc::downgrade(inner);
    let entry = entry.clone();
    let title = title.to_owned();
    button.connect_clicked(move |_| {
        let Some(inner) = weak.upgrade() else { return };
        let entry = entry.clone();
        let parent = parent_window(&inner.root);
        let dialog = gtk::FileDialog::builder().title(&title).modal(true).build();
        dialog.open(parent.as_ref(), gio::Cancellable::NONE, move |result| {
            if let Some(path) = result.ok().and_then(|file| file.path()) {
                entry.set_text(&path.to_string_lossy());
            }
        });
    });
}

fn connect_export_picker(inner: &Rc<Inner>, button: &gtk::Button) {
    let weak = Rc::downgrade(inner);
    button.connect_clicked(move |_| {
        let Some(inner) = weak.upgrade() else { return };
        let dialog = gtk::FileDialog::builder()
            .title(tr("Choose Raycast Export JSON"))
            .modal(true)
            .build();
        let parent = parent_window(&inner.root);
        let weak = Rc::downgrade(&inner);
        dialog.open(parent.as_ref(), gio::Cancellable::NONE, move |result| {
            let Some(path) = result.ok().and_then(|file| file.path()) else {
                return;
            };
            let Some(inner) = weak.upgrade() else { return };
            match fs::read(&path)
                .map_err(|error| error.to_string())
                .and_then(|data| preview_export(&data).map_err(|error| error.to_string()))
            {
                Ok(items) => show_preview(&inner, items),
                Err(error) => {
                    clear_preview(&inner);
                    show_status(&inner, &error);
                }
            }
        });
    });
}

fn show_preview(inner: &Rc<Inner>, items: Vec<RaycastImportItem>) {
    clear_preview(inner);
    for item in items {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let check = gtk::CheckButton::new();
        let kind = match item.kind {
            kebloop_services::raycast::RaycastImportKind::Snippet => tr("Snippet"),
            kebloop_services::raycast::RaycastImportKind::Quicklink => tr("Quicklink"),
        };
        let primary = gtk::Label::builder()
            .label(format!("{kind} · {}", item.name))
            .xalign(0.0)
            .wrap(true)
            .hexpand(true)
            .build();
        let value = gtk::Label::builder()
            .label(&item.value)
            .xalign(0.0)
            .wrap(true)
            .selectable(true)
            .build();
        let details = gtk::Box::new(gtk::Orientation::Vertical, 2);
        details.append(&primary);
        details.append(&value);
        if let Some(issue) = &item.issue {
            let problem = helper_text(issue);
            problem.add_css_class("error");
            details.append(&problem);
            check.set_sensitive(false);
        } else {
            check.set_active(true);
        }
        check.update_property(&[gtk::accessible::Property::Label(&format!(
            "Import {kind}: {}",
            item.name
        ))]);
        row.append(&check);
        row.append(&details);
        row.set_margin_start(8);
        row.set_margin_end(8);
        row.set_margin_top(4);
        row.set_margin_bottom(4);
        inner.rows_box.append(&row);
        inner.rows.borrow_mut().push((item, check));
    }
    inner
        .rows_scroll
        .set_visible(!inner.rows.borrow().is_empty());
    inner.import_rows.set_sensitive(
        inner
            .rows
            .borrow()
            .iter()
            .any(|(item, check)| item.issue.is_none() && check.is_active()),
    );
    show_status(
        inner,
        &format!(
            "{} export items previewed. Select the rows to add.",
            inner.rows.borrow().len()
        ),
    );
}

fn clear_preview(inner: &Rc<Inner>) {
    while let Some(child) = inner.rows_box.first_child() {
        inner.rows_box.remove(&child);
    }
    inner.rows.borrow_mut().clear();
    inner.rows_scroll.set_visible(false);
    inner.import_rows.set_sensitive(false);
}

fn connect_import_rows(inner: &Rc<Inner>) {
    let weak = Rc::downgrade(inner);
    inner.import_rows.connect_clicked(move |_| {
        let Some(inner) = weak.upgrade() else { return };
        let selected: Vec<_> = inner
            .rows
            .borrow()
            .iter()
            .filter(|(item, check)| item.issue.is_none() && check.is_active())
            .map(|(item, _)| item.clone())
            .collect();
        if selected.is_empty() {
            show_status(&inner, &tr("Select at least one supported item."));
            return;
        }
        let Some(ctx) = inner.ctx.upgrade() else { return };
        let selected_ids = selected.iter().map(|item| item.id.clone()).collect();
        let home = ctx.home.clone();
        let outcome = ctx.update_settings(|settings| {
            let searches = settings.web_searches.clone();
            import_selected(
                &selected,
                &selected_ids,
                &mut settings.snippets,
                &mut settings.quicklinks,
                &searches,
                &home,
            )
        });
        match outcome {
            Ok((imported, skipped)) => show_status(
                &inner,
                &format!(
                    "Imported {imported} item(s); preserved {skipped} existing or unsupported item(s)."
                ),
            ),
            Err(error) => show_status(&inner, &error.to_string()),
        }
    });
}

fn connect_script_picker(inner: &Rc<Inner>, button: &gtk::Button) {
    let weak = Rc::downgrade(inner);
    button.connect_clicked(move |_| {
        let Some(inner) = weak.upgrade() else { return };
        let dialog = gtk::FileDialog::builder()
            .title(tr("Choose Raycast Script Command"))
            .modal(true)
            .build();
        let parent = parent_window(&inner.root);
        let weak = Rc::downgrade(&inner);
        dialog.open(parent.as_ref(), gio::Cancellable::NONE, move |result| {
            let Some(path) = result.ok().and_then(|file| file.path()) else {
                return;
            };
            let Some(inner) = weak.upgrade() else { return };
            match inspect_script(&path) {
                Ok(command) => {
                    inner.script_summary.set_text(&format!(
                        "{} · {}\n{}",
                        command.title,
                        command.description,
                        path.display()
                    ));
                    inner.script_summary.set_visible(true);
                    inner.script_review.set_sensitive(true);
                    inner.script_import.set_sensitive(true);
                    *inner.script_candidate.borrow_mut() = Some((command, path));
                    show_status(
                        &inner,
                        &tr("Script metadata is valid. Review its source before importing."),
                    );
                }
                Err(error) => {
                    inner.script_summary.set_text(&error.to_string());
                    inner.script_summary.set_visible(true);
                    inner.script_review.set_sensitive(false);
                    inner.script_import.set_sensitive(false);
                    inner.script_candidate.borrow_mut().take();
                }
            }
        });
    });
}

fn connect_script_review(inner: &Rc<Inner>) {
    let weak = Rc::downgrade(inner);
    inner.script_review.connect_clicked(move |_| {
        let Some(inner) = weak.upgrade() else { return };
        let Some((command, path)) = inner.script_candidate.borrow().clone() else {
            return;
        };
        let source = match fs::read_to_string(path) {
            Ok(source) => source,
            Err(error) => {
                show_status(&inner, &error.to_string());
                return;
            }
        };
        let editor = gtk::TextView::builder()
            .editable(false)
            .cursor_visible(false)
            .monospace(true)
            .wrap_mode(gtk::WrapMode::None)
            .left_margin(10)
            .right_margin(10)
            .top_margin(8)
            .bottom_margin(8)
            .build();
        editor.buffer().set_text(&source);
        editor.update_property(&[gtk::accessible::Property::Label(&tr(
            "Raycast script source",
        ))]);
        let scroll = gtk::ScrolledWindow::builder()
            .child(&editor)
            .hexpand(true)
            .vexpand(true)
            .build();
        let window = gtk::Window::builder()
            .title(format!("{} — {}", tr("Review Source"), command.title))
            .default_width(720)
            .default_height(520)
            .child(&scroll)
            .build();
        if let Some(parent) = parent_window(&inner.root) {
            window.set_transient_for(Some(&parent));
        }
        window.present();
    });
}

fn connect_script_import(inner: &Rc<Inner>) {
    let weak = Rc::downgrade(inner);
    inner.script_import.connect_clicked(move |_| {
        let Some(inner) = weak.upgrade() else { return };
        let Some((command, _)) = inner.script_candidate.borrow().clone() else {
            return;
        };
        let Some(ctx) = inner.ctx.upgrade() else {
            return;
        };
        let outcome =
            ctx.update_settings(|settings| import_scripts(&mut settings.scripts, vec![command]));
        match outcome {
            Ok(()) => show_status(
                &inner,
                &tr("Imported disabled. Review and enable it in Script Commands."),
            ),
            Err(error) => show_status(&inner, &error.to_string()),
        }
    });
}

fn connect_sdk_preview(inner: &Rc<Inner>, button: &gtk::Button) {
    let weak = Rc::downgrade(inner);
    button.connect_clicked(move |_| {
        let Some(inner) = weak.upgrade() else { return };
        let manifest = inner.sdk_manifest.text().to_string();
        let command_name = inner.sdk_command_name.text().to_string();
        let entry = inner.sdk_entry.text().to_string();
        let runtime = inner.sdk_runtime.text().to_string();
        match RaycastSdkCommand::inspect(manifest, &command_name, entry, runtime) {
            Ok(command) => {
                while let Some(child) = inner.sdk_preferences.first_child() {
                    inner.sdk_preferences.remove(&child);
                }
                inner.sdk_preference_entries.borrow_mut().clear();
                for preference in &command.preferences {
                    let label = preference
                        .title
                        .as_deref()
                        .filter(|title| !title.trim().is_empty())
                        .unwrap_or(&preference.name);
                    let entry = gtk::Entry::builder()
                        .placeholder_text(label)
                        .text(preference.default_value.as_deref().unwrap_or_default())
                        .hexpand(true)
                        .build();
                    let required = if preference.required == Some(true) {
                        tr("Required")
                    } else {
                        String::new()
                    };
                    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                    row.append(&gtk::Label::builder().label(format!("{label} {required}")).xalign(0.0).width_chars(19).build());
                    row.append(&entry);
                    inner.sdk_preferences.append(&row);
                    inner
                        .sdk_preference_entries
                        .borrow_mut()
                        .push((preference.clone(), entry));
                }
                inner.sdk_summary.set_text(&format!(
                    "{} · {} preference(s) · standalone no-view command · imported disabled",
                    command.title,
                    command.preferences.len()
                ));
                inner.sdk_summary.set_visible(true);
                inner.sdk_review.set_sensitive(true);
                inner.sdk_import.set_sensitive(true);
                *inner.sdk_candidate.borrow_mut() = Some(command);
                show_status(&inner, &tr("Command manifest and source are valid. Review preferences, then import disabled."));
            }
            Err(error) => {
                inner.sdk_candidate.borrow_mut().take();
                inner.sdk_summary.set_text(&error.to_string());
                inner.sdk_summary.set_visible(true);
                inner.sdk_review.set_sensitive(false);
                inner.sdk_import.set_sensitive(false);
                while let Some(child) = inner.sdk_preferences.first_child() {
                    inner.sdk_preferences.remove(&child);
                }
                inner.sdk_preference_entries.borrow_mut().clear();
                show_status(&inner, &error.to_string());
            }
        }
    });
}

fn connect_sdk_import(inner: &Rc<Inner>) {
    let weak = Rc::downgrade(inner);
    inner.sdk_import.connect_clicked(move |_| {
        let Some(inner) = weak.upgrade() else { return };
        let Some(mut command) = inner.sdk_candidate.borrow().clone() else {
            return;
        };
        for (preference, entry) in inner.sdk_preference_entries.borrow().iter() {
            let value = entry.text().to_string();
            if !value.is_empty() || preference.default_value.is_none() {
                command.values.insert(preference.name.clone(), value);
            }
        }
        let mut commands = inner.sdk_commands.borrow().clone();
        command.enabled = false;
        commands.push(command);
        match persist_sdk_commands(&inner.sdk_store_path, &commands) {
            Ok(()) => {
                *inner.sdk_commands.borrow_mut() = commands;
                inner.sdk_candidate.borrow_mut().take();
                inner.sdk_import.set_sensitive(false);
                inner.sdk_review.set_sensitive(false);
                render_sdk_commands(&inner);
                show_status(
                    &inner,
                    &tr("Imported disabled. Review the source, set preferences and enable it before running."),
                );
            }
            Err(error) => show_status(&inner, &error),
        }
    });
}

fn connect_sdk_review(inner: &Rc<Inner>) {
    let weak = Rc::downgrade(inner);
    inner.sdk_review.connect_clicked(move |_| {
        let Some(inner) = weak.upgrade() else { return };
        let Some(command) = inner.sdk_candidate.borrow().clone() else {
            return;
        };
        show_source_window(&inner, &command.source, &command.title);
    });
}

fn load_sdk_commands(path: &std::path::Path) -> (Vec<RaycastSdkCommand>, Option<String>) {
    match fs::read(path) {
        Ok(data) => match serde_json::from_slice(&data) {
            Ok(commands) => (commands, None),
            Err(error) => (
                Vec::new(),
                Some(format!(
                    "Could not read the local Raycast compatibility list: {error}"
                )),
            ),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (Vec::new(), None),
        Err(error) => (
            Vec::new(),
            Some(format!(
                "Could not read the local Raycast compatibility list: {error}"
            )),
        ),
    }
}

fn persist_sdk_commands(
    path: &std::path::Path,
    commands: &[RaycastSdkCommand],
) -> Result<(), String> {
    let data = serde_json::to_vec_pretty(commands).map_err(|error| error.to_string())?;
    atomic_write(path, &data, 0o600).map_err(|error| error.to_string())
}

fn render_sdk_commands(inner: &Rc<Inner>) {
    while let Some(child) = inner.sdk_installed.first_child() {
        inner.sdk_installed.remove(&child);
    }
    let commands = inner.sdk_commands.borrow().clone();
    if commands.is_empty() {
        inner.sdk_installed.append(&helper_text(
            "No Raycast JavaScript commands have been imported.",
        ));
        return;
    }
    for command in commands {
        let section = gtk::Box::new(gtk::Orientation::Vertical, 5);
        section.add_css_class("card");
        section.set_margin_start(2);
        section.set_margin_end(2);
        section.set_margin_top(2);
        section.set_margin_bottom(2);
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let enabled = gtk::CheckButton::with_label(&command.title);
        enabled.set_active(command.enabled);
        enabled.set_hexpand(true);
        enabled.update_property(&[gtk::accessible::Property::Label(&format!(
            "Enable {}",
            command.title
        ))]);
        let review = gtk::Button::with_label(&tr("Review Source"));
        let run = gtk::Button::with_label(&tr("Run"));
        run.set_sensitive(command.enabled);
        let remove = gtk::Button::with_label(&tr("Remove"));
        actions.append(&enabled);
        actions.append(&review);
        actions.append(&run);
        actions.append(&remove);
        section.append(&actions);
        let source_note = helper_text(&command.source);
        section.append(&source_note);

        for preference in &command.preferences {
            let title = preference
                .title
                .as_deref()
                .filter(|title| !title.trim().is_empty())
                .unwrap_or(&preference.name);
            let entry = gtk::Entry::builder()
                .placeholder_text(title)
                .text(
                    command
                        .values
                        .get(&preference.name)
                        .or(preference.default_value.as_ref())
                        .map(String::as_str)
                        .unwrap_or_default(),
                )
                .hexpand(true)
                .build();
            entry.update_property(&[gtk::accessible::Property::Label(&format!(
                "{} preference for {}",
                title, command.title
            ))]);
            let save = gtk::Button::with_label(&tr("Save"));
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            row.append(
                &gtk::Label::builder()
                    .label(title)
                    .xalign(0.0)
                    .width_chars(16)
                    .build(),
            );
            row.append(&entry);
            row.append(&save);
            section.append(&row);
            let weak = Rc::downgrade(inner);
            let command_id = command.id.clone();
            let preference_name = preference.name.clone();
            save.connect_clicked(move |_| {
                let Some(inner) = weak.upgrade() else { return };
                let mut updated = inner.sdk_commands.borrow().clone();
                let Some(command) = updated.iter_mut().find(|item| item.id == command_id) else {
                    return;
                };
                command
                    .values
                    .insert(preference_name.clone(), entry.text().to_string());
                match persist_sdk_commands(&inner.sdk_store_path, &updated) {
                    Ok(()) => {
                        *inner.sdk_commands.borrow_mut() = updated;
                        show_status(&inner, &tr("Preference saved."));
                    }
                    Err(error) => show_status(&inner, &error),
                }
            });
        }

        let weak = Rc::downgrade(inner);
        let command_id = command.id.clone();
        let run_button = run.clone();
        let reverting = Rc::new(Cell::new(false));
        let reverting_toggle = reverting.clone();
        enabled.connect_toggled(move |toggle| {
            if reverting_toggle.get() {
                return;
            }
            let Some(inner) = weak.upgrade() else { return };
            let mut updated = inner.sdk_commands.borrow().clone();
            let Some(command) = updated.iter_mut().find(|item| item.id == command_id) else {
                return;
            };
            command.enabled = toggle.is_active();
            match persist_sdk_commands(&inner.sdk_store_path, &updated) {
                Ok(()) => {
                    let enabled = toggle.is_active();
                    *inner.sdk_commands.borrow_mut() = updated;
                    run_button.set_sensitive(enabled);
                    let message = if enabled {
                        tr("Enabled. Review the source and preferences before running.")
                    } else {
                        tr("Command disabled.")
                    };
                    show_status(&inner, &message);
                }
                Err(error) => {
                    reverting_toggle.set(true);
                    toggle.set_active(!toggle.is_active());
                    reverting_toggle.set(false);
                    show_status(&inner, &error);
                }
            }
        });

        let weak = Rc::downgrade(inner);
        let source = command.source.clone();
        let title = command.title.clone();
        review.connect_clicked(move |_| {
            let Some(inner) = weak.upgrade() else { return };
            show_source_window(&inner, &source, &title);
        });

        let weak = Rc::downgrade(inner);
        let command_id = command.id.clone();
        run.connect_clicked(move |_| {
            let Some(inner) = weak.upgrade() else { return };
            let command = inner
                .sdk_commands
                .borrow()
                .iter()
                .find(|item| item.id == command_id)
                .cloned();
            if let Some(command) = command {
                run_sdk_command(&inner, command);
            }
        });

        let weak = Rc::downgrade(inner);
        let command_id = command.id.clone();
        remove.connect_clicked(move |_| {
            let Some(inner) = weak.upgrade() else { return };
            let mut updated = inner.sdk_commands.borrow().clone();
            updated.retain(|item| item.id != command_id);
            match persist_sdk_commands(&inner.sdk_store_path, &updated) {
                Ok(()) => {
                    *inner.sdk_commands.borrow_mut() = updated;
                    render_sdk_commands(&inner);
                    show_status(&inner, &tr("Raycast command removed."));
                }
                Err(error) => show_status(&inner, &error),
            }
        });
        inner.sdk_installed.append(&section);
    }
}

fn show_source_window(inner: &Rc<Inner>, path: &str, title: &str) {
    let source = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) => {
            show_status(inner, &error.to_string());
            return;
        }
    };
    let editor = gtk::TextView::builder()
        .editable(false)
        .cursor_visible(false)
        .monospace(true)
        .wrap_mode(gtk::WrapMode::None)
        .left_margin(10)
        .right_margin(10)
        .top_margin(8)
        .bottom_margin(8)
        .build();
    editor.buffer().set_text(&source);
    editor.update_property(&[gtk::accessible::Property::Label(&tr(
        "Raycast command source",
    ))]);
    let scroll = gtk::ScrolledWindow::builder()
        .child(&editor)
        .hexpand(true)
        .vexpand(true)
        .build();
    let window = gtk::Window::builder()
        .title(format!("{} — {title}", tr("Review Source")))
        .default_width(720)
        .default_height(520)
        .child(&scroll)
        .build();
    if let Some(parent) = parent_window(&inner.root) {
        window.set_transient_for(Some(&parent));
    }
    window.present();
}

fn run_sdk_command(inner: &Rc<Inner>, command: RaycastSdkCommand) {
    let Some(ctx) = inner.ctx.upgrade() else {
        return;
    };
    if !command.enabled {
        show_status(inner, &tr("Review and enable this command before running."));
        return;
    }
    let runner = AutomationRunner::new();
    let cancel = runner.cancel_handle();
    let token = ctx.register_run(cancel);
    let (sender, receiver) = async_channel::bounded::<Result<_, AutomationError>>(1);
    let spawned = std::thread::Builder::new()
        .name("kebloop-raycast-sdk".into())
        .spawn(move || {
            let result = execute_sdk(&command, &runner);
            let _ = sender.send_blocking(result);
        });
    show_status(inner, &tr("Running Raycast command…"));
    let weak = Rc::downgrade(inner);
    glib::spawn_future_local(async move {
        let outcome = if spawned.is_ok() {
            receiver.recv().await.ok()
        } else {
            None
        };
        ctx.finish_run(token);
        let Some(inner) = weak.upgrade() else { return };
        match outcome {
            Some(Ok(response)) => apply_sdk_effects(&inner, &response.effects),
            Some(Err(error)) => show_status(&inner, &error.to_string()),
            None => show_status(&inner, &tr("Could not start the Raycast command.")),
        }
    });
}

fn apply_sdk_effects(inner: &Rc<Inner>, effects: &[RaycastSdkEffect]) {
    let mut messages = Vec::new();
    for effect in effects {
        match effect.effect_type.as_str() {
            "copy" => {
                if let Some(display) = gdk::Display::default() {
                    display.clipboard().set_text(&effect.value);
                    messages.push(tr("Copied text."));
                } else {
                    messages.push(tr("Clipboard is unavailable."));
                }
            }
            "open" => {
                if let Err(error) =
                    gio::AppInfo::launch_default_for_uri(&effect.value, gio::AppLaunchContext::NONE)
                {
                    messages.push(format!("{} {error}", tr("Could not open URL:")));
                }
            }
            "message" => messages.push(effect.value.clone()),
            _ => {}
        }
    }
    if messages.is_empty() {
        show_status(inner, &tr("Command finished."));
    } else {
        show_status(inner, &messages.join("\n"));
    }
}

fn parent_window(widget: &impl IsA<gtk::Widget>) -> Option<gtk::Window> {
    widget.root()?.downcast::<gtk::Window>().ok()
}

fn show_status(inner: &Rc<Inner>, text: &str) {
    inner.status.set_text(text);
    inner.status.set_visible(!text.is_empty());
}
