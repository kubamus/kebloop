//! Building blocks shared by the settings pages: bound rows that guard against feedback loops,
//! dynamic row lists that only rebuild when their content changed, dialogs and file choosers.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};

use crate::context::AppContext;
use crate::i18n::tr;

/// What every page needs: the application context, the toast overlay and the window (as the
/// parent of dialogs).
pub struct Env {
    /// Application context.
    pub ctx: Rc<AppContext>,
    toasts: adw::ToastOverlay,
    window: glib::WeakRef<gtk::Window>,
}

impl Env {
    /// Create the environment for `window`, whose content is wrapped by `toasts`.
    pub fn new(ctx: Rc<AppContext>, toasts: adw::ToastOverlay, window: &gtk::Window) -> Rc<Self> {
        Rc::new(Self {
            ctx,
            toasts,
            window: window.downgrade(),
        })
    }

    /// Show a short message at the bottom of the window.
    pub fn toast(&self, text: &str) {
        let toast = adw::Toast::new(text);
        toast.set_timeout(4);
        self.toasts.add_toast(toast);
    }

    /// The settings window.
    pub fn window(&self) -> Option<gtk::Window> {
        self.window.upgrade()
    }

    /// Show `dialog` over the settings window.
    pub fn present(&self, dialog: &adw::Dialog) {
        match self.window() {
            Some(window) => dialog.present(Some(&window)),
            None => dialog.present(gtk::Widget::NONE),
        }
    }
}

/// One settings page: its widget and how it follows changes made elsewhere.
pub struct Page {
    /// The page content (an `AdwPreferencesPage` or any scrollable widget).
    pub widget: gtk::Widget,
    refresh: Box<dyn Fn()>,
}

impl Page {
    /// A page whose `refresh` re-reads the application state into the widgets.
    pub fn new(widget: &impl IsA<gtk::Widget>, refresh: impl Fn() + 'static) -> Self {
        Self {
            widget: widget.clone().upcast(),
            refresh: Box::new(refresh),
        }
    }

    /// A page that never changes behind the user's back.
    pub fn fixed(widget: &impl IsA<gtk::Widget>) -> Self {
        Self::new(widget, || {})
    }

    /// Re-read state.
    pub fn refresh(&self) {
        (self.refresh)();
    }
}

/// An empty `AdwPreferencesPage`.
pub fn preferences_page() -> adw::PreferencesPage {
    adw::PreferencesPage::new()
}

/// A titled group; the description is optional.
pub fn group(title: &str, description: &str) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    if !title.is_empty() {
        group.set_title(&glib::markup_escape_text(title));
    }
    if !description.is_empty() {
        group.set_description(Some(&glib::markup_escape_text(description)));
    }
    group
}

/// A dimmed, wrapping explanation line.
pub fn note(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .wrap(true)
        .xalign(0.0)
        .css_classes(["dim-label", "caption"])
        .margin_top(4)
        .margin_bottom(4)
        .build()
}

/// A wrapping label in the error colour that is hidden while it has no text.
pub fn error_label() -> gtk::Label {
    gtk::Label::builder()
        .wrap(true)
        .xalign(0.0)
        .visible(false)
        .selectable(true)
        .css_classes(["error", "caption"])
        .margin_top(4)
        .margin_bottom(4)
        .build()
}

/// Show `message` in `label`, or hide it when `None`.
pub fn set_error(label: &gtk::Label, message: Option<&str>) {
    label.set_label(message.unwrap_or(""));
    label.set_visible(message.is_some());
}

/// A flat icon button with a tooltip that doubles as its accessible name.
pub fn icon_button(icon: &str, tooltip: &str) -> gtk::Button {
    let button = gtk::Button::builder()
        .icon_name(icon)
        .tooltip_text(tooltip)
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    button.update_property(&[gtk::accessible::Property::Label(tooltip)]);
    button
}

/// A text button that fits into a row suffix.
pub fn row_button(label: &str) -> gtk::Button {
    gtk::Button::builder()
        .label(label)
        .valign(gtk::Align::Center)
        .build()
}

/// A selectable monospace label.
pub fn mono_label(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .selectable(true)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .xalign(0.0)
        .css_classes(["monospace"])
        .build()
}

/// A button that copies `text` to the clipboard and says so.
pub fn copy_button(env: &Rc<Env>, text: impl Fn() -> String + 'static) -> gtk::Button {
    let button = icon_button("edit-copy-symbolic", &tr("Copy"));
    let env = env.clone();
    button.connect_clicked(move |button| {
        button.clipboard().set_text(&text());
        env.toast(&tr("Copied to the clipboard."));
    });
    button
}

/// A switch row whose changes are applied through `apply`. A refused change is undone and
/// reported in a toast. [`SwitchControl::sync`] updates the switch without calling `apply`.
pub struct SwitchControl {
    /// The row to add to a group.
    pub row: adw::SwitchRow,
    quiet: Rc<Cell<bool>>,
}

impl SwitchControl {
    /// Build the row.
    pub fn new(
        env: &Rc<Env>,
        title: &str,
        subtitle: &str,
        active: bool,
        apply: impl Fn(bool) -> Result<(), String> + 'static,
    ) -> Self {
        let row = adw::SwitchRow::builder()
            .title(title)
            .active(active)
            .build();
        if !subtitle.is_empty() {
            row.set_subtitle(subtitle);
        }
        let quiet = Rc::new(Cell::new(false));
        let guard = quiet.clone();
        let env = env.clone();
        row.connect_active_notify(move |row| {
            if guard.get() {
                return;
            }
            if let Err(message) = apply(row.is_active()) {
                guard.set(true);
                row.set_active(!row.is_active());
                guard.set(false);
                env.toast(&message);
            }
        });
        Self { row, quiet }
    }

    /// A switch row without an environment (errors are not reported to a toast; used by views
    /// that have no settings window).
    pub fn with_ctx(
        title: &str,
        active: bool,
        apply: impl Fn(bool) -> Result<(), String> + 'static,
    ) -> Self {
        let row = adw::SwitchRow::builder()
            .title(title)
            .active(active)
            .build();
        let quiet = Rc::new(Cell::new(false));
        let guard = quiet.clone();
        row.connect_active_notify(move |row| {
            if guard.get() {
                return;
            }
            if let Err(message) = apply(row.is_active()) {
                eprintln!("kebloop: {message}");
                guard.set(true);
                row.set_active(!row.is_active());
                guard.set(false);
            }
        });
        Self { row, quiet }
    }

    /// Show `active` without applying it.
    pub fn sync(&self, active: bool) {
        if self.row.is_active() != active {
            self.quiet.set(true);
            self.row.set_active(active);
            self.quiet.set(false);
        }
    }
}

/// A combo row of fixed choices; the selected index is applied through `apply`.
pub struct ComboControl {
    /// The row to add to a group.
    pub row: adw::ComboRow,
    quiet: Rc<Cell<bool>>,
}

impl ComboControl {
    /// Build the row. `apply` receives the chosen index.
    pub fn new(
        env: &Rc<Env>,
        title: &str,
        subtitle: &str,
        choices: &[String],
        selected: u32,
        apply: impl Fn(u32) -> Result<(), String> + 'static,
    ) -> Self {
        let labels: Vec<&str> = choices.iter().map(String::as_str).collect();
        let row = adw::ComboRow::builder()
            .title(title)
            .model(&gtk::StringList::new(&labels))
            .selected(selected)
            .build();
        if !subtitle.is_empty() {
            row.set_subtitle(subtitle);
        }
        let quiet = Rc::new(Cell::new(false));
        let guard = quiet.clone();
        let env = env.clone();
        let last = Rc::new(Cell::new(selected));
        row.connect_selected_notify(move |row| {
            if guard.get() {
                return;
            }
            let chosen = row.selected();
            if let Err(message) = apply(chosen) {
                guard.set(true);
                row.set_selected(last.get());
                guard.set(false);
                env.toast(&message);
            } else {
                last.set(chosen);
            }
        });
        Self { row, quiet }
    }

    /// Replace the choices and selection without applying anything.
    pub fn sync(&self, choices: &[String], selected: u32) {
        let labels: Vec<&str> = choices.iter().map(String::as_str).collect();
        self.quiet.set(true);
        self.row.set_model(Some(&gtk::StringList::new(&labels)));
        self.row.set_selected(selected);
        self.quiet.set(false);
    }

    /// Select `selected` without applying it.
    pub fn select(&self, selected: u32) {
        if self.row.selected() != selected {
            self.quiet.set(true);
            self.row.set_selected(selected);
            self.quiet.set(false);
        }
    }
}

/// A spin row whose value is applied through `apply`.
pub struct SpinControl {
    /// The row to add to a group.
    pub row: adw::SpinRow,
    quiet: Rc<Cell<bool>>,
}

impl SpinControl {
    /// Build the row for integers in `min..=max`.
    pub fn new(
        env: &Rc<Env>,
        title: &str,
        subtitle: &str,
        range: (f64, f64, f64),
        value: f64,
        apply: impl Fn(f64) -> Result<(), String> + 'static,
    ) -> Self {
        let (min, max, step) = range;
        let adjustment = gtk::Adjustment::new(value, min, max, step, step * 5.0, 0.0);
        let row = adw::SpinRow::new(Some(&adjustment), 1.0, 0);
        row.set_title(title);
        if !subtitle.is_empty() {
            row.set_subtitle(subtitle);
        }
        let quiet = Rc::new(Cell::new(false));
        let guard = quiet.clone();
        let env = env.clone();
        let last = Rc::new(Cell::new(value));
        row.connect_value_notify(move |row| {
            if guard.get() {
                return;
            }
            let chosen = row.value();
            if let Err(message) = apply(chosen) {
                guard.set(true);
                row.set_value(last.get());
                guard.set(false);
                env.toast(&message);
            } else {
                last.set(chosen);
            }
        });
        Self { row, quiet }
    }

    /// Show `value` without applying it.
    pub fn sync(&self, value: f64) {
        if (self.row.value() - value).abs() > f64::EPSILON {
            self.quiet.set(true);
            self.row.set_value(value);
            self.quiet.set(false);
        }
    }
}

/// A list of rows inside a group that is rebuilt only when its `signature` changes, so a refresh
/// triggered by the page's own edit never destroys a focused widget.
pub struct DynGroup {
    /// The group (add it to the page; put static rows into it before the first `update`).
    pub group: adw::PreferencesGroup,
    rows: RefCell<Vec<gtk::Widget>>,
    signature: RefCell<Option<String>>,
}

impl DynGroup {
    /// Wrap `group`.
    pub fn new(group: adw::PreferencesGroup) -> Self {
        Self {
            group,
            rows: RefCell::new(Vec::new()),
            signature: RefCell::new(None),
        }
    }

    /// Replace the rows with `build()` unless `signature` equals the one of the last update.
    pub fn update(&self, signature: &str, build: impl FnOnce() -> Vec<gtk::Widget>) {
        if self.signature.borrow().as_deref() == Some(signature) {
            return;
        }
        *self.signature.borrow_mut() = Some(signature.to_owned());
        for row in self.rows.borrow_mut().drain(..) {
            self.group.remove(&row);
        }
        let rows = build();
        for row in &rows {
            self.group.add(row);
        }
        *self.rows.borrow_mut() = rows;
    }
}

/// A row that tells the user a list is empty.
pub fn placeholder_row(title: &str, subtitle: &str) -> gtk::Widget {
    let row = adw::ActionRow::builder()
        .title(glib::markup_escape_text(title))
        .selectable(false)
        .css_classes(["dim-label"])
        .build();
    if !subtitle.is_empty() {
        row.set_subtitle(&glib::markup_escape_text(subtitle));
    }
    row.upcast()
}

/// An `AdwActionRow` with escaped title and subtitle (the row parses Pango markup).
pub fn action_row(title: &str, subtitle: &str) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(glib::markup_escape_text(title))
        .title_lines(1)
        .subtitle_lines(2)
        .build();
    if !subtitle.is_empty() {
        row.set_subtitle(&glib::markup_escape_text(subtitle));
    }
    row
}

/// A row that runs `activate` and shows a leading icon, for "add" style actions.
pub fn button_row(title: &str, icon: &str, activate: impl Fn() + 'static) -> adw::ButtonRow {
    let row = adw::ButtonRow::builder()
        .title(title)
        .start_icon_name(icon)
        .build();
    row.connect_activated(move |_| activate());
    row
}

/// Ask for confirmation; `confirmed` runs only when the user chooses the action.
pub fn confirm(
    env: &Rc<Env>,
    heading: &str,
    body: &str,
    action: &str,
    destructive: bool,
    confirmed: impl FnOnce() + 'static,
) {
    let Some(window) = env.window() else { return };
    let dialog = adw::AlertDialog::builder()
        .heading(heading)
        .body(body)
        .close_response("cancel")
        .default_response("cancel")
        .build();
    dialog.add_response("cancel", &tr("Cancel"));
    dialog.add_response("confirm", action);
    dialog.set_response_appearance(
        "confirm",
        if destructive {
            adw::ResponseAppearance::Destructive
        } else {
            adw::ResponseAppearance::Suggested
        },
    );
    dialog.choose(Some(&window), gio::Cancellable::NONE, move |response| {
        if response == "confirm" {
            confirmed();
        }
    });
}

/// Choose folders (one or several). Cancelling does nothing.
pub fn choose_folders(
    env: &Rc<Env>,
    title: &str,
    multiple: bool,
    chosen: impl Fn(Vec<PathBuf>) + 'static,
) {
    let dialog = gtk::FileDialog::builder().title(title).modal(true).build();
    let window = env.window();
    if multiple {
        dialog.select_multiple_folders(window.as_ref(), gio::Cancellable::NONE, move |result| {
            if let Ok(model) = result {
                let paths = (0..model.n_items())
                    .filter_map(|i| model.item(i))
                    .filter_map(|object| object.downcast::<gio::File>().ok())
                    .filter_map(|file| file.path())
                    .collect();
                chosen(paths);
            }
        });
    } else {
        dialog.select_folder(window.as_ref(), gio::Cancellable::NONE, move |result| {
            if let Some(path) = result.ok().and_then(|file| file.path()) {
                chosen(vec![path]);
            }
        });
    }
}

/// Choose one existing file.
pub fn choose_file(env: &Rc<Env>, title: &str, chosen: impl Fn(PathBuf) + 'static) {
    let dialog = gtk::FileDialog::builder().title(title).modal(true).build();
    dialog.open(
        env.window().as_ref(),
        gio::Cancellable::NONE,
        move |result| {
            if let Some(path) = result.ok().and_then(|file| file.path()) {
                chosen(path);
            }
        },
    );
}

/// Choose where to save a file.
pub fn choose_save_path(
    env: &Rc<Env>,
    title: &str,
    initial_name: &str,
    chosen: impl Fn(PathBuf) + 'static,
) {
    let dialog = gtk::FileDialog::builder()
        .title(title)
        .modal(true)
        .initial_name(initial_name)
        .build();
    dialog.save(
        env.window().as_ref(),
        gio::Cancellable::NONE,
        move |result| {
            if let Some(path) = result.ok().and_then(|file| file.path()) {
                chosen(path);
            }
        },
    );
}

/// A framed, scrolling multi-line text editor.
pub fn text_editor(
    min_height: i32,
    accessible_label: &str,
) -> (gtk::TextView, gtk::ScrolledWindow) {
    let view = gtk::TextView::builder()
        .wrap_mode(gtk::WrapMode::WordChar)
        .top_margin(8)
        .bottom_margin(8)
        .left_margin(10)
        .right_margin(10)
        .accepts_tab(false)
        .build();
    view.update_property(&[gtk::accessible::Property::Label(accessible_label)]);
    let scroller = gtk::ScrolledWindow::builder()
        .child(&view)
        .min_content_height(min_height)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .css_classes(["card"])
        .build();
    (view, scroller)
}

/// The text of a `TextView`.
pub fn text_of(view: &gtk::TextView) -> String {
    let buffer = view.buffer();
    buffer
        .text(&buffer.start_iter(), &buffer.end_iter(), false)
        .to_string()
}
