//! Keyboard-first window, menu and layout lists; the launcher search entry keeps focus.
use crate::i18n::tr;
use gtk::prelude::*;
use kebloop_services::selected_text::{SelectedTextAction, SelectedTextProvider, TransformRequest};
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Debug, Clone)]
pub struct WorkflowRow {
    pub id: String,
    pub title: String,
    pub subtitle: String,
}
type Activate = Rc<dyn Fn(&WorkflowRow)>;
#[derive(Clone)]
pub struct DesktopWorkflowView {
    inner: Rc<Inner>,
}
struct Inner {
    root: gtk::Box,
    list: gtk::ListBox,
    status: gtk::Label,
    rows: RefCell<Vec<WorkflowRow>>,
    on_activate: RefCell<Option<Activate>>,
}
impl DesktopWorkflowView {
    pub fn new() -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let status = gtk::Label::builder().xalign(0.0).wrap(true).build();
        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::Single);
        list.set_activate_on_single_click(true);
        let scroll = gtk::ScrolledWindow::builder()
            .child(&list)
            .min_content_height(220)
            .max_content_height(340)
            .propagate_natural_height(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();
        root.append(&scroll);
        root.append(&status);
        let view = Self {
            inner: Rc::new(Inner {
                root,
                list,
                status,
                rows: RefCell::new(Vec::new()),
                on_activate: RefCell::new(None),
            }),
        };
        let weak = Rc::downgrade(&view.inner);
        view.inner.list.connect_row_activated(move |_, row| {
            let Some(inner) = weak.upgrade() else {
                return;
            };
            let selected = inner.rows.borrow().get(row.index() as usize).cloned();
            let callback = inner.on_activate.borrow().clone();
            if let (Some(row), Some(callback)) = (selected, callback) {
                callback(&row);
            }
        });
        view
    }
    pub fn widget(&self) -> &gtk::Box {
        &self.inner.root
    }
    pub fn set_status(&self, text: &str) {
        self.inner.status.set_text(text);
    }
    pub fn set_rows(&self, rows: Vec<WorkflowRow>) {
        while let Some(child) = self.inner.list.first_child() {
            self.inner.list.remove(&child);
        }
        for item in &rows {
            let body = gtk::Box::new(gtk::Orientation::Vertical, 2);
            body.set_margin_top(8);
            body.set_margin_bottom(8);
            let title = gtk::Label::builder()
                .label(&item.title)
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .build();
            let subtitle = gtk::Label::builder()
                .label(&item.subtitle)
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .css_classes(["dim-label"])
                .build();
            body.append(&title);
            body.append(&subtitle);
            self.inner.list.append(&body);
        }
        *self.inner.rows.borrow_mut() = rows;
        self.inner
            .list
            .select_row(self.inner.list.row_at_index(0).as_ref());
    }
    pub fn connect_activate(&self, callback: impl Fn(&WorkflowRow) + 'static) {
        *self.inner.on_activate.borrow_mut() = Some(Rc::new(callback));
    }
    pub fn move_selection(&self, delta: i32) {
        let count = self.inner.rows.borrow().len() as i32;
        if count == 0 {
            return;
        }
        let selected = self.inner.list.selected_row().map_or(0, |row| row.index());
        self.inner.list.select_row(
            self.inner
                .list
                .row_at_index((selected + delta).clamp(0, count - 1))
                .as_ref(),
        );
    }
    pub fn selected(&self) -> Option<WorkflowRow> {
        self.inner
            .list
            .selected_row()
            .and_then(|row| self.inner.rows.borrow().get(row.index() as usize).cloned())
    }
    pub fn activate_selected(&self) {
        let callback = self.inner.on_activate.borrow().clone();
        if let (Some(row), Some(callback)) = (self.selected(), callback) {
            callback(&row);
        }
    }
}
impl Default for DesktopWorkflowView {
    fn default() -> Self {
        Self::new()
    }
}

/// Explicit selected-text review: choose action/provider with Tab, preview before copy/replace.
/// Transform callback runs the service off-thread; caller returns results using set_result.
#[derive(Clone)]
pub struct SelectedTextView {
    root: gtk::Box,
    action: gtk::DropDown,
    provider: gtk::DropDown,
    language: gtk::Entry,
    model: gtk::Entry,
    key: gtk::PasswordEntry,
    source: gtk::TextView,
    preview: gtk::TextView,
    status: gtk::Label,
    handlers: Rc<RefCell<Vec<(gtk::Button, gtk::glib::SignalHandlerId)>>>,
    local: [gtk::Button; 3],
    transform: gtk::Button,
    copy: gtk::Button,
    replace: gtk::Button,
}
impl SelectedTextView {
    pub fn new() -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let choices = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let action = gtk::DropDown::from_strings(
            &SelectedTextAction::ALL
                .map(|a| tr(a.title()))
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
        );
        let provider =
            gtk::DropDown::from_strings(&[&tr("Local model (Ollama)"), "OpenAI", "Claude"]);
        let language = gtk::Entry::builder()
            .text("English")
            .placeholder_text(tr("Target language"))
            .hexpand(true)
            .build();
        choices.append(&action);
        choices.append(&provider);
        choices.append(&language);
        root.append(&choices);
        let credentials = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let model = gtk::Entry::builder()
            .text("llama3.2")
            .placeholder_text(tr("Model"))
            .hexpand(true)
            .build();
        let key = gtk::PasswordEntry::builder()
            .placeholder_text(tr("API key"))
            .hexpand(true)
            .show_peek_icon(true)
            .visible(false)
            .build();
        credentials.append(&model);
        credentials.append(&key);
        root.append(&credentials);
        let source = gtk::TextView::builder()
            .editable(false)
            .cursor_visible(false)
            .wrap_mode(gtk::WrapMode::WordChar)
            .build();
        let preview = gtk::TextView::builder()
            .editable(false)
            .cursor_visible(false)
            .wrap_mode(gtk::WrapMode::WordChar)
            .build();
        for (label, view) in [("Selected text", &source), ("Preview", &preview)] {
            root.append(&gtk::Label::builder().label(tr(label)).xalign(0.0).build());
            root.append(
                &gtk::ScrolledWindow::builder()
                    .child(view)
                    .min_content_height(90)
                    .max_content_height(140)
                    .propagate_natural_height(true)
                    .build(),
            );
        }
        let local = [
            gtk::Button::with_label(&tr("Uppercase")),
            gtk::Button::with_label(&tr("Lowercase")),
            gtk::Button::with_label(&tr("Trim Whitespace")),
        ];
        let local_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        for button in &local {
            local_row.append(button);
        }
        root.append(&local_row);
        let controls = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let transform = gtk::Button::with_label(&tr("Transform"));
        let copy = gtk::Button::with_label(&tr("Copy"));
        let replace = gtk::Button::with_label(&tr("Replace Selection"));
        copy.set_sensitive(false);
        replace.set_sensitive(false);
        controls.append(&transform);
        controls.append(&copy);
        controls.append(&replace);
        root.append(&controls);
        let status = gtk::Label::builder()
            .xalign(0.0)
            .wrap(true)
            .label(tr(
                "Tab to choose an action. Transform, review, then copy or replace.",
            ))
            .build();
        root.append(&status);
        let view = Self {
            root,
            action,
            provider,
            language,
            model,
            key,
            source,
            preview,
            status,
            handlers: Rc::new(RefCell::new(Vec::new())),
            local,
            transform,
            copy,
            replace,
        };
        let model = view.model.clone();
        let key = view.key.clone();
        view.provider.connect_selected_notify(move |provider| {
            model.set_text(match provider.selected() {
                1 => "gpt-5-mini",
                2 => "claude-sonnet-5",
                _ => "llama3.2",
            });
            key.set_visible(provider.selected() != 0);
        });
        view
    }
    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }
    pub fn set_selection(&self, text: &str) {
        self.source.buffer().set_text(text);
        self.preview.buffer().set_text("");
        self.copy.set_sensitive(false);
        self.replace.set_sensitive(false);
        self.transform.set_sensitive(!text.is_empty());
        for button in &self.local {
            button.set_sensitive(!text.is_empty());
        }
    }
    pub fn set_busy(&self, busy: bool) {
        self.transform.set_sensitive(!busy);
        self.copy.set_sensitive(false);
        self.replace.set_sensitive(false);
        if busy {
            self.status.set_text(&tr("Working…"));
        }
    }
    pub fn set_result(&self, result: Result<&str, &str>, editable: bool) {
        self.transform.set_sensitive(true);
        match result {
            Ok(text) => {
                self.preview.buffer().set_text(text);
                self.copy.set_sensitive(true);
                self.replace.set_sensitive(editable);
                self.status
                    .set_text(&tr("Review the result before replacing selected text."));
            }
            Err(error) => self.status.set_text(error),
        }
    }
    pub fn set_status(&self, text: &str) {
        self.status.set_text(text);
    }
    pub fn result(&self) -> String {
        let buffer = self.preview.buffer();
        buffer
            .text(&buffer.start_iter(), &buffer.end_iter(), false)
            .to_string()
    }
    pub fn request(&self) -> TransformRequest {
        let buffer = self.source.buffer();
        TransformRequest {
            action: SelectedTextAction::ALL
                .get(self.action.selected() as usize)
                .copied()
                .unwrap_or(SelectedTextAction::Rewrite),
            provider: match self.provider.selected() {
                1 => SelectedTextProvider::OpenAi,
                2 => SelectedTextProvider::Claude,
                _ => SelectedTextProvider::Local,
            },
            text: buffer
                .text(&buffer.start_iter(), &buffer.end_iter(), false)
                .to_string(),
            target_language: self.language.text().to_string(),
            model: self.model.text().to_string(),
            api_key: self.key.text().to_string(),
        }
    }
    pub fn connect_local(&self, callback: impl Fn(String) + 'static) {
        use kebloop_services::selected_text::LocalTextAction;
        let callback = Rc::new(callback);
        for (button, action) in self.local.iter().zip([
            LocalTextAction::Uppercase,
            LocalTextAction::Lowercase,
            LocalTextAction::TrimWhitespace,
        ]) {
            let callback = callback.clone();
            let source = self.source.clone();
            let handler = button.connect_clicked(move |_| {
                let buffer = source.buffer();
                callback(action.transform(&buffer.text(
                    &buffer.start_iter(),
                    &buffer.end_iter(),
                    false,
                )));
            });
            self.handlers.borrow_mut().push((button.clone(), handler));
        }
    }
    pub fn connect_transform(&self, callback: impl Fn(TransformRequest) + 'static) {
        let view = self.clone();
        let handler = self
            .transform
            .connect_clicked(move |_| callback(view.request()));
        self.handlers
            .borrow_mut()
            .push((self.transform.clone(), handler));
    }
    pub fn connect_copy(&self, callback: impl Fn(String) + 'static) {
        let view = self.clone();
        let handler = self.copy.connect_clicked(move |_| callback(view.result()));
        self.handlers
            .borrow_mut()
            .push((self.copy.clone(), handler));
    }
    pub fn connect_replace(&self, callback: impl Fn(String) + 'static) {
        let view = self.clone();
        let handler = self
            .replace
            .connect_clicked(move |_| callback(view.result()));
        self.handlers
            .borrow_mut()
            .push((self.replace.clone(), handler));
    }
    fn disconnect_callbacks(&self) {
        let handlers = std::mem::take(&mut *self.handlers.borrow_mut());
        for (button, handler) in handlers {
            button.disconnect(handler);
        }
    }
    pub fn focus(&self) {
        self.action.grab_focus();
    }
}
impl Default for SelectedTextView {
    fn default() -> Self {
        Self::new()
    }
}

use crate::context::AppContext;
use gtk::{gdk, glib};
use kebloop_platform::compositor::Window;
use kebloop_platform::workflows::{
    AccessibilityWorkflows, DesktopWorkflows, MenuItem, TextSelection,
};
use kebloop_services::window_layouts::WindowLayouts;

#[derive(Debug, Clone, Copy)]
pub enum WorkflowKind {
    Windows,
    Menus,
    Layouts,
    SelectedText,
}
/// Open a fully keyboard-operable workflow. Caller dismisses the launcher first.
/// Selection capture returns focus to the previous window before showing its preview.
pub fn open_workflow(ctx: &Rc<AppContext>, kind: WorkflowKind, previous: Option<Window>) {
    if matches!(kind, WorkflowKind::SelectedText) {
        open_selected_text(ctx, previous);
        return;
    }
    let title = match kind {
        WorkflowKind::Windows => "Search Windows",
        WorkflowKind::Menus => "Search Menu Items",
        WorkflowKind::Layouts => "Saved Window Layouts",
        WorkflowKind::SelectedText => unreachable!(),
    };
    let window = gtk::ApplicationWindow::builder()
        .application(&ctx.app)
        .title(tr(title))
        .default_width(620)
        .default_height(420)
        .build();
    let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
    body.set_margin_start(12);
    body.set_margin_end(12);
    body.set_margin_top(12);
    body.set_margin_bottom(12);
    let entry = gtk::SearchEntry::new();
    entry.set_placeholder_text(Some(&tr("Search…")));
    body.append(&entry);
    let view = DesktopWorkflowView::new();
    body.append(view.widget());
    window.set_child(Some(&body));
    let close_view = view.clone();
    window.connect_close_request(move |_| {
        close_view.inner.on_activate.borrow_mut().take();
        glib::Propagation::Proceed
    });
    let rows = Rc::new(RefCell::new(Vec::<WorkflowRow>::new()));
    let menus = Rc::new(RefCell::new(Vec::<MenuItem>::new()));
    let store = Rc::new(RefCell::new(None::<WindowLayouts>));
    let filter_rows = {
        let rows = rows.clone();
        let view = view.clone();
        move |query: &str| {
            let query = kebloop_services::util::fold(query);
            view.set_rows(
                rows.borrow()
                    .iter()
                    .filter(|row| {
                        kebloop_services::util::fold(&format!("{} {}", row.title, row.subtitle))
                            .contains(&query)
                    })
                    .cloned()
                    .collect(),
            );
        }
    };
    entry.connect_search_changed(move |entry| filter_rows(&entry.text()));
    let on_view = view.clone();
    let on_window = window.clone();
    let compositor = ctx.compositor.clone();
    let on_menus = menus.clone();
    let on_store = store.clone();
    let on_previous = previous.clone();
    view.connect_activate(move |row| {
        let Some(compositor) = compositor.clone() else {
            on_view.set_status(&tr("Window integration is unavailable on this desktop."));
            return;
        };
        let view = on_view.clone();
        let window = on_window.clone();
        let id = row.id.clone();
        let previous = on_previous.clone();
        match kind {
            WorkflowKind::Windows => {
                window.set_visible(false);
                crate::work::spawn(
                    "kebloop-focus-window",
                    move || {
                        DesktopWorkflows::new(compositor)
                            .focus_window(&kebloop_platform::compositor::WindowId(id))
                    },
                    move |result| {
                        if let Err(error) = result {
                            view.set_status(&error);
                            window.present();
                        } else {
                            window.close();
                        }
                    },
                );
            }
            WorkflowKind::Menus => {
                let item = on_menus
                    .borrow()
                    .iter()
                    .find(|item| item.target.path.as_str() == id)
                    .cloned();
                let Some(item) = item else {
                    return;
                };
                window.set_visible(false);
                crate::work::spawn(
                    "kebloop-menu-action",
                    move || {
                        if let Some(previous) = previous {
                            compositor
                                .focus_window(&previous.id)
                                .map_err(|e| e.to_string())?;
                        }
                        AccessibilityWorkflows::connect()?.activate_menu(&item)
                    },
                    move |result| {
                        if let Err(error) = result {
                            view.set_status(&error);
                            window.present();
                        } else {
                            window.close();
                        }
                    },
                );
            }
            WorkflowKind::Layouts => {
                let layout = on_store
                    .borrow()
                    .as_ref()
                    .and_then(|store| store.layouts().iter().find(|layout| layout.id == id))
                    .cloned();
                let Some(layout) = layout else {
                    return;
                };
                view.set_status(&tr("Restoring layout…"));
                crate::work::spawn(
                    "kebloop-restore-layout",
                    move || DesktopWorkflows::new(compositor).restore_layout(&layout),
                    move |result| match result {
                        Ok(report) => view.set_status(&format!(
                            "{} restored, {} missing. {}",
                            report.restored,
                            report.missing,
                            report.failed.join("; ")
                        )),
                        Err(error) => view.set_status(&error),
                    },
                );
            }
            WorkflowKind::SelectedText => {}
        }
    });
    if matches!(kind, WorkflowKind::Layouts) {
        let controls = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let name = gtk::Entry::builder()
            .placeholder_text(tr("Layout name"))
            .hexpand(true)
            .build();
        let save = gtk::Button::with_label(&tr("Save Current Layout"));
        let delete = gtk::Button::with_label(&tr("Delete"));
        controls.append(&name);
        controls.append(&save);
        controls.append(&delete);
        body.append(&controls);
        let busy = Rc::new(std::cell::Cell::new(false));
        let compositor = ctx.compositor.clone();
        let save_store = store.clone();
        let save_view = view.clone();
        let save_rows = rows.clone();
        let save_busy = busy.clone();
        save.connect_clicked(move |_| {
            if save_busy.get() {
                return;
            }
            let Some(compositor) = compositor.clone() else {
                return;
            };
            let Some(mut next_store) = save_store.borrow().clone() else {
                return;
            };
            let name = name.text().to_string();
            let view = save_view.clone();
            let store = save_store.clone();
            let rows = save_rows.clone();
            let busy = save_busy.clone();
            busy.set(true);
            let spawned =
                crate::work::spawn(
                    "kebloop-save-layout",
                    move || {
                        let result = DesktopWorkflows::new(compositor).capture_layout().and_then(
                            |windows| next_store.save(&name, windows).map_err(|e| e.to_string()),
                        );
                        (next_store, result)
                    },
                    move |(next_store, result)| {
                        busy.set(false);
                        match result {
                            Ok(_) => {
                                let next = layout_rows(Some(&next_store));
                                *store.borrow_mut() = Some(next_store);
                                *rows.borrow_mut() = next.clone();
                                view.set_rows(next);
                                view.set_status(&tr("Layout saved."));
                            }
                            Err(error) => view.set_status(&error),
                        }
                    },
                );
            if !spawned {
                save_busy.set(false);
                save_view.set_status(&tr("Could not start the background operation."));
            }
        });
        let delete_store = store.clone();
        let delete_view = view.clone();
        let delete_rows = rows.clone();
        let delete_busy = busy.clone();
        delete.connect_clicked(move |_| {
            if delete_busy.get() {
                return;
            }
            let Some(row) = delete_view.selected() else {
                return;
            };
            let Some(mut next_store) = delete_store.borrow().clone() else {
                return;
            };
            let store = delete_store.clone();
            let view = delete_view.clone();
            let rows = delete_rows.clone();
            let busy = delete_busy.clone();
            busy.set(true);
            let spawned = crate::work::spawn(
                "kebloop-delete-layout",
                move || {
                    let result = next_store.remove(&row.id).map_err(|e| e.to_string());
                    (next_store, result)
                },
                move |(next_store, result)| {
                    busy.set(false);
                    match result {
                        Ok(()) => {
                            let next = layout_rows(Some(&next_store));
                            *store.borrow_mut() = Some(next_store);
                            *rows.borrow_mut() = next.clone();
                            view.set_rows(next);
                        }
                        Err(error) => view.set_status(&error),
                    }
                },
            );
            if !spawned {
                delete_busy.set(false);
                delete_view.set_status(&tr("Could not start the background operation."));
            }
        });
        view.set_status(&tr(
            "Enter restores a layout. Tab to name and save the current layout.",
        ));
    } else {
        view.set_status(&tr("Loading…"));
    }
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let key_view = view.clone();
    let key_window = window.downgrade();
    let key_entry = entry.clone();
    keys.connect_key_pressed(move |_, key, _, mods| {
        let Some(key_window) = key_window.upgrade() else {
            return glib::Propagation::Proceed;
        };
        if key == gdk::Key::Escape {
            key_window.close();
            return glib::Propagation::Stop;
        }
        if mods.contains(gdk::ModifierType::CONTROL_MASK) && key == gdk::Key::f {
            key_entry.grab_focus();
            return glib::Propagation::Stop;
        }
        let search_has_focus =
            gtk::prelude::GtkWindowExt::focus(&key_window).is_some_and(|focus| {
                focus == key_entry.clone().upcast::<gtk::Widget>() || focus.is_ancestor(&key_entry)
            });
        if search_has_focus {
            match key {
                gdk::Key::Down => {
                    key_view.move_selection(1);
                    return glib::Propagation::Stop;
                }
                gdk::Key::Up => {
                    key_view.move_selection(-1);
                    return glib::Propagation::Stop;
                }
                gdk::Key::Return | gdk::Key::KP_Enter => {
                    key_view.activate_selected();
                    return glib::Propagation::Stop;
                }
                _ => {}
            }
        }
        glib::Propagation::Proceed
    });
    window.add_controller(keys);
    window.present();
    entry.grab_focus();
    match kind {
        WorkflowKind::Windows => {
            let Some(compositor) = ctx.compositor.clone() else {
                view.set_status(&tr("Window integration is unavailable on this desktop."));
                return;
            };
            crate::work::spawn(
                "kebloop-search-windows",
                move || DesktopWorkflows::new(compositor).search_windows(""),
                move |result| match result {
                    Ok(windows) => {
                        let next = windows
                            .into_iter()
                            .map(|window| WorkflowRow {
                                id: window.id.0,
                                title: window.title,
                                subtitle: window.app_id.unwrap_or_default(),
                            })
                            .collect::<Vec<_>>();
                        *rows.borrow_mut() = next.clone();
                        view.set_rows(next);
                        view.set_status(&tr("↑/↓ to choose, Enter to focus. Ctrl+F searches."));
                    }
                    Err(error) => view.set_status(&error),
                },
            );
        }
        WorkflowKind::Menus => {
            let Some(pid) = previous.and_then(|window| window.pid) else {
                view.set_status(&tr("No previous application is available."));
                return;
            };
            crate::work::spawn(
                "kebloop-search-menus",
                move || AccessibilityWorkflows::connect()?.menu_items(pid),
                move |result| match result {
                    Ok(items) => {
                        let next = items
                            .iter()
                            .map(|item| WorkflowRow {
                                id: item.target.path.to_string(),
                                title: item.title.clone(),
                                subtitle: item.path.clone(),
                            })
                            .collect::<Vec<_>>();
                        *rows.borrow_mut() = next.clone();
                        *menus.borrow_mut() = items;
                        view.set_rows(next);
                        view.set_status(&tr(
                            "↑/↓ to choose, Enter to run. Only accessible menu items are shown.",
                        ));
                    }
                    Err(error) => view.set_status(&error),
                },
            );
        }
        WorkflowKind::Layouts => {
            let path = ctx.xdg.app_data_dir().join("window-layouts.json");
            crate::work::spawn(
                "kebloop-load-layouts",
                move || WindowLayouts::load(path),
                move |result| match result {
                    Ok(loaded) => {
                        let next = layout_rows(Some(&loaded));
                        *store.borrow_mut() = Some(loaded);
                        *rows.borrow_mut() = next.clone();
                        view.set_rows(next);
                    }
                    Err(error) => view.set_status(&error.to_string()),
                },
            );
        }
        WorkflowKind::SelectedText => {}
    }
}
fn layout_rows(store: Option<&WindowLayouts>) -> Vec<WorkflowRow> {
    store
        .into_iter()
        .flat_map(|store| store.layouts())
        .map(|layout| WorkflowRow {
            id: layout.id.clone(),
            title: layout.name.clone(),
            subtitle: format!("{} windows", layout.windows.len()),
        })
        .collect()
}

fn open_selected_text(ctx: &Rc<AppContext>, previous: Option<Window>) {
    let window = gtk::ApplicationWindow::builder()
        .application(&ctx.app)
        .title(tr("Selected Text Actions"))
        .default_width(660)
        .default_height(560)
        .build();
    let view = SelectedTextView::new();
    view.widget().set_margin_start(12);
    view.widget().set_margin_end(12);
    view.widget().set_margin_top(12);
    view.widget().set_margin_bottom(12);
    window.set_child(Some(view.widget()));
    let selection = Rc::new(RefCell::new(None::<TextSelection>));
    let generation = Rc::new(std::cell::Cell::new(0u64));
    let closed = Rc::new(std::cell::Cell::new(false));
    let close_flag = closed.clone();
    let close_generation = generation.clone();
    let close_view = view.clone();
    window.connect_close_request(move |_| {
        close_view.disconnect_callbacks();
        close_flag.set(true);
        close_generation.set(close_generation.get() + 1);
        glib::Propagation::Proceed
    });
    let local_view = view.clone();
    let local_selection = selection.clone();
    let local_generation = generation.clone();
    view.connect_local(move |text| {
        local_generation.set(local_generation.get() + 1);
        local_view.set_result(
            Ok(&text),
            local_selection
                .borrow()
                .as_ref()
                .is_some_and(|s| s.editable),
        );
    });
    let transform_view = view.clone();
    let transform_selection = selection.clone();
    let transform_generation = generation.clone();
    let transform_closed = closed.clone();
    view.connect_transform(move |request| {
        if transform_selection.borrow().is_none() {
            return;
        }
        transform_generation.set(transform_generation.get() + 1);
        let token = transform_generation.get();
        transform_view.set_busy(true);
        let view = transform_view.clone();
        let selection = transform_selection.clone();
        let generation = transform_generation.clone();
        let closed = transform_closed.clone();
        crate::work::spawn(
            "kebloop-selected-text",
            move || kebloop_services::selected_text::transform(&request),
            move |result| {
                if closed.get() || generation.get() != token {
                    return;
                }
                view.set_result(
                    result.as_deref().map_err(String::as_str),
                    selection.borrow().as_ref().is_some_and(|s| s.editable),
                );
            },
        );
    });
    let clipboard = view.widget().clipboard();
    view.connect_copy(move |text| clipboard.set_text(&text));
    let replace_view = view.clone();
    let replace_selection = selection.clone();
    let replace_window = window.clone();
    let replace_previous = previous.clone();
    let compositor = ctx.compositor.clone();
    view.connect_replace(move |replacement| {
        let Some(snapshot) = replace_selection.borrow().clone() else {
            return;
        };
        let Some(compositor) = compositor.clone() else {
            return;
        };
        let Some(previous) = replace_previous.clone() else {
            return;
        };
        replace_window.set_visible(false);
        let view = replace_view.clone();
        let window = replace_window.clone();
        let clipboard = view.widget().clipboard();
        let fallback = replacement.clone();
        crate::work::spawn(
            "kebloop-replace-selected-text",
            move || {
                compositor
                    .focus_window(&previous.id)
                    .map_err(|e| e.to_string())?;
                let accessibility = AccessibilityWorkflows::connect()?;
                // Compositor focus and toolkit accessibility state are asynchronous. Wait briefly,
                // but retry only focus revalidation, never a mutation that might already have run.
                let deadline = std::time::Instant::now() + std::time::Duration::from_millis(800);
                loop {
                    match accessibility.capture_selection(snapshot.pid) {
                        Ok(current)
                            if current.target == snapshot.target
                                && current.start == snapshot.start
                                && current.end == snapshot.end
                                && current.text == snapshot.text =>
                        {
                            break;
                        }
                        Ok(_) => {
                            return Err("The selected text changed; result copied instead".into());
                        }
                        Err(error) if std::time::Instant::now() >= deadline => return Err(error),
                        Err(_) => std::thread::sleep(std::time::Duration::from_millis(50)),
                    }
                }
                accessibility.replace_selection(&snapshot, &replacement)
            },
            move |result| match result {
                Ok(()) => window.close(),
                Err(error) => {
                    clipboard.set_text(&fallback);
                    view.set_status(&format!("{error}. Result copied."));
                    window.present();
                }
            },
        );
    });
    let keys = gtk::EventControllerKey::new();
    let key_window = window.downgrade();
    keys.connect_key_pressed(move |_, key, _, _| {
        let Some(key_window) = key_window.upgrade() else {
            return glib::Propagation::Proceed;
        };
        if key == gdk::Key::Escape {
            key_window.close();
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    window.add_controller(keys);
    let Some(previous) = previous else {
        view.set_status(&tr("No previous application is available."));
        window.present();
        return;
    };
    let Some(pid) = previous.pid else {
        view.set_status(&tr(
            "This application does not expose its process identifier.",
        ));
        window.present();
        return;
    };
    let Some(compositor) = ctx.compositor.clone() else {
        view.set_status(&tr("Window integration is unavailable on this desktop."));
        window.present();
        return;
    };
    view.set_status(&tr("Reading selected text…"));
    crate::work::spawn(
        "kebloop-capture-selected-text",
        move || {
            compositor
                .focus_window(&previous.id)
                .map_err(|e| e.to_string())?;
            let accessibility = AccessibilityWorkflows::connect()?;
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(800);
            loop {
                match accessibility.capture_selection(pid) {
                    Ok(selection) => break Ok(selection),
                    Err(error) if std::time::Instant::now() >= deadline => break Err(error),
                    Err(_) => std::thread::sleep(std::time::Duration::from_millis(50)),
                }
            }
        },
        move |result| {
            if closed.get() {
                return;
            }
            match result {
                Ok(snapshot) => {
                    view.set_selection(&snapshot.text);
                    *selection.borrow_mut() = Some(snapshot);
                    view.set_status(&tr("Choose an action, transform, then review the preview."));
                }
                Err(error) => {
                    view.set_selection("");
                    view.set_status(&error);
                }
            }
            window.present();
            view.focus();
        },
    );
}
