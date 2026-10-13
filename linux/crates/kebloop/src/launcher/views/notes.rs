//! Keyboard-first Markdown notes with a rendered preview and an optional note browser.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use kebloop_services::error::ConfigError;
use kebloop_services::notes::{MarkdownNote, MarkdownNotesStore};

use crate::context::AppContext;
use crate::i18n::tr;

type PlainCallback = Rc<dyn Fn()>;

thread_local! {
    static ACTIVE_NOTES: RefCell<Weak<Inner>> = const { RefCell::new(Weak::new()) };
}

/// Save the active editor before an explicit application quit; retain a rejected draft.
pub fn save_before_quit() -> bool {
    let Some(inner) = ACTIVE_NOTES.with(|active| active.borrow().upgrade()) else {
        return true;
    };
    if inner.save_current() {
        return true;
    }
    if let Some(window) = inner
        .window
        .borrow()
        .as_ref()
        .and_then(glib::WeakRef::upgrade)
    {
        window.present();
    }
    false
}

/// One reusable notes view. `Ctrl+P` opens its keyboard-searchable note list.
#[derive(Clone)]
pub struct NotesPage {
    inner: Rc<Inner>,
}

struct Inner {
    root: gtk::Box,
    title: gtk::Label,
    editor_scroll: gtk::ScrolledWindow,
    editor: gtk::TextView,
    preview_scroll: gtk::ScrolledWindow,
    preview: gtk::TextView,
    browse: gtk::Box,
    browse_search: gtk::Entry,
    browse_list: gtk::ListBox,
    browse_ids: RefCell<Vec<String>>,
    find_bar: gtk::Box,
    find_entry: gtk::Entry,
    status: gtk::Label,
    footer: gtk::Label,
    recovery_actions: gtk::Box,
    edit_button: gtk::Button,
    more_popover: gtk::Popover,
    window: RefCell<Option<glib::WeakRef<gtk::Window>>>,
    store: Rc<RefCell<MarkdownNotesStore>>,
    selected_id: RefCell<Option<String>>,
    editing: Cell<bool>,
    browsing: Cell<bool>,
    suppress_editor_change: Cell<bool>,
    recovery_generation: Cell<u64>,
    on_close: RefCell<Option<PlainCallback>>,
    on_browse: RefCell<Option<PlainCallback>>,
}

impl NotesPage {
    /// Build a view for a store owned by the caller.
    pub fn new(store: Rc<RefCell<MarkdownNotesStore>>) -> Self {
        let title = gtk::Label::builder()
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["kb-page-title"])
            .build();
        let edit = gtk::Button::with_label(&tr("Edit"));
        edit.set_action_name(Some("notes.toggle-edit"));
        let browse_button = gtk::Button::with_label(&tr("Browse Notes"));
        browse_button.set_action_name(Some("notes.browse"));
        let new_note_button = gtk::Button::with_label(&tr("New Note"));
        new_note_button.set_action_name(Some("notes.new"));
        let more = gtk::MenuButton::builder()
            .label(tr("More"))
            .css_classes(["flat"])
            .build();
        more.set_widget_name("notes-more-control");
        let menu = gtk::Popover::new();
        let menu_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
        menu_box.set_margin_top(6);
        menu_box.set_margin_bottom(6);
        menu_box.set_margin_start(6);
        menu_box.set_margin_end(6);
        let export = gtk::Button::with_label(&tr("Export Markdown…   Ctrl+Shift+E"));
        export.set_action_name(Some("notes.export"));
        let delete = gtk::Button::with_label(&tr("Delete Note…   Ctrl+Delete"));
        delete.set_action_name(Some("notes.delete"));
        menu_box.append(&export);
        menu_box.append(&delete);
        menu.set_child(Some(&menu_box));
        more.set_popover(Some(&menu));
        let to_launcher = gtk::Button::with_label(&tr("Back to Launcher"));
        to_launcher.set_action_name(Some("notes.launcher"));
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        header.add_css_class("kb-category");
        header.append(&title);
        header.append(&new_note_button);
        header.append(&to_launcher);
        header.append(&browse_button);
        header.append(&edit);
        header.append(&more);

        let editor = gtk::TextView::builder()
            .wrap_mode(gtk::WrapMode::WordChar)
            .monospace(true)
            .accepts_tab(false)
            .top_margin(12)
            .bottom_margin(12)
            .left_margin(14)
            .right_margin(14)
            .css_classes(["kb-input"])
            .build();
        editor.buffer().set_enable_undo(true);
        editor.update_property(&[gtk::accessible::Property::Label(&tr(
            "Markdown note editor",
        ))]);
        let editor_scroll = gtk::ScrolledWindow::builder()
            .child(&editor)
            .vexpand(true)
            .hexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();

        let preview = markdown_preview_view();
        let preview_scroll = gtk::ScrolledWindow::builder()
            .child(&preview)
            .vexpand(true)
            .hexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();

        let browse_search = gtk::Entry::builder()
            .placeholder_text(tr("Search notes"))
            .hexpand(true)
            .build();
        browse_search.update_property(&[gtk::accessible::Property::Label(&tr("Search notes"))]);
        let browse_list = gtk::ListBox::new();
        browse_list.set_selection_mode(gtk::SelectionMode::Single);
        browse_list.set_activate_on_single_click(false);
        browse_list.add_css_class("boxed-list");
        let browse_scroll = gtk::ScrolledWindow::builder()
            .child(&browse_list)
            .vexpand(true)
            .hexpand(true)
            .build();
        let browse = gtk::Box::new(gtk::Orientation::Vertical, 8);
        browse.add_css_class("kb-page");
        browse.append(&browse_search);
        browse.append(&browse_scroll);
        browse.set_visible(false);

        let recovery_label = gtk::Label::builder()
            .label(tr("A recovered draft is available."))
            .xalign(0.0)
            .hexpand(true)
            .build();
        let restore_button = gtk::Button::with_label(&tr("Restore   Ctrl+Shift+R"));
        restore_button.set_action_name(Some("notes.restore"));
        let discard_button = gtk::Button::with_label(&tr("Discard   Ctrl+Shift+D"));
        discard_button.set_action_name(Some("notes.discard"));
        let recovery_actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        recovery_actions.set_margin_start(12);
        recovery_actions.set_margin_end(12);
        recovery_actions.set_margin_top(6);
        recovery_actions.append(&recovery_label);
        recovery_actions.append(&restore_button);
        recovery_actions.append(&discard_button);
        recovery_actions.set_visible(false);

        let find_entry = gtk::Entry::builder()
            .placeholder_text(tr("Find in note"))
            .hexpand(true)
            .build();
        find_entry.update_property(&[gtk::accessible::Property::Label(&tr("Find in note"))]);
        let find_hint = gtk::Label::new(Some(&tr("Enter: next match · Esc: close find")));
        find_hint.add_css_class("dim-label");
        let find_bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        find_bar.set_margin_start(12);
        find_bar.set_margin_end(12);
        find_bar.set_margin_top(6);
        find_bar.append(&find_entry);
        find_bar.append(&find_hint);
        find_bar.set_visible(false);

        let status = gtk::Label::builder()
            .xalign(0.0)
            .wrap(true)
            .css_classes(["kb-message"])
            .visible(false)
            .build();
        let footer = gtk::Label::builder()
            .xalign(0.0)
            .wrap(true)
            .css_classes(["dim-label"])
            .build();
        let bottom = gtk::Box::new(gtk::Orientation::Vertical, 4);
        bottom.set_margin_start(12);
        bottom.set_margin_end(12);
        bottom.set_margin_top(8);
        bottom.set_margin_bottom(8);
        bottom.append(&find_bar);
        bottom.append(&status);
        bottom.append(&footer);

        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("kb-page");
        root.append(&header);
        root.append(&recovery_actions);
        root.append(&editor_scroll);
        root.append(&preview_scroll);
        root.append(&browse);
        root.append(&bottom);

        let inner = Rc::new(Inner {
            root,
            title,
            editor_scroll,
            editor,
            preview_scroll,
            preview,
            browse,
            browse_search,
            browse_list,
            browse_ids: RefCell::new(Vec::new()),
            find_bar,
            find_entry,
            status,
            footer,
            recovery_actions,
            edit_button: edit.clone(),
            more_popover: menu,
            window: RefCell::new(None),
            store,
            selected_id: RefCell::new(None),
            editing: Cell::new(false),
            browsing: Cell::new(false),
            suppress_editor_change: Cell::new(false),
            recovery_generation: Cell::new(0),
            on_close: RefCell::new(None),
            on_browse: RefCell::new(None),
        });
        connect_actions(
            &inner,
            [
                &edit,
                &new_note_button,
                &browse_button,
                &to_launcher,
                &restore_button,
                &discard_button,
                &export,
                &delete,
            ],
        );
        connect_editor(&inner);
        connect_browser(&inner);
        connect_find(&inner);
        connect_keys(&inner);
        Self { inner }
    }

    /// Root widget to put in a panel or a floating window.
    pub fn widget(&self) -> &gtk::Box {
        &self.inner.root
    }

    /// Connect closing the note surface and switching back to launcher browsing.
    pub fn connect(&self, on_close: impl Fn() + 'static, on_browse: impl Fn() + 'static) {
        *self.inner.on_close.borrow_mut() = Some(Rc::new(on_close));
        *self.inner.on_browse.borrow_mut() = Some(Rc::new(on_browse));
    }

    /// Open a note, or the newest note when `initial_id` is absent. An editing open with no
    /// existing notes creates a blank note and focuses its editor.
    pub fn begin(&self, initial_id: Option<&str>, editing: bool) {
        let create_new = initial_id.is_none() && editing;
        if create_new {
            Inner::create_blank(&self.inner);
            return;
        }
        let selected = {
            let store = self.inner.store.borrow();
            initial_id
                .and_then(|id| store.note(id).map(|note| note.id.clone()))
                .or_else(|| store.notes().first().map(|note| note.id.clone()))
        };
        let selection_changed = *self.inner.selected_id.borrow() != selected;
        if !selection_changed && self.inner.editing.get() {
            self.focus();
            return;
        }
        if selection_changed && !self.inner.save_current() {
            return;
        }
        if selection_changed {
            self.inner.editing.set(false);
            *self.inner.selected_id.borrow_mut() = selected;
        }
        if self.inner.selected_id.borrow().is_none() {
            self.inner.editing.set(false);
            self.inner.browsing.set(false);
            self.inner.refresh_selected();
        } else {
            if editing && !self.inner.editing.get() {
                self.inner.begin_editing();
            } else {
                self.inner.refresh_selected();
            }
        }
    }

    /// Search note titles and bodies in the in-view browser and take focus there.
    pub fn set_query(&self, query: &str) {
        self.inner.browse_search.set_text(query);
        self.inner.enter_browser();
    }

    /// Focus the editor or the most useful current view.
    pub fn focus(&self) {
        if self.inner.browsing.get() {
            self.inner.browse_search.grab_focus();
        } else if self.inner.editing.get() {
            self.inner.editor.grab_focus();
        } else {
            self.inner.preview.grab_focus();
        }
    }

    fn save_current(&self) -> bool {
        self.inner.save_current()
    }
}

impl Inner {
    fn selected_note(&self) -> Option<MarkdownNote> {
        let id = self.selected_id.borrow().clone()?;
        self.store.borrow().note(&id).cloned()
    }

    fn refresh_selected(&self) {
        let selected = self.selected_note();
        self.title.set_text(
            &selected
                .as_ref()
                .map_or_else(|| tr("Notes"), MarkdownNote::title),
        );
        self.edit_button.set_label(&if self.editing.get() {
            tr("Preview")
        } else {
            tr("Edit")
        });
        let editing = self.editing.get();
        self.editor_scroll
            .set_visible(editing && !self.browsing.get());
        self.preview_scroll
            .set_visible(!editing && !self.browsing.get());
        self.browse.set_visible(self.browsing.get());
        self.recovery_actions.set_visible(
            !editing
                && !self.browsing.get()
                && self
                    .selected_id
                    .borrow()
                    .as_ref()
                    .is_some_and(|id| self.store.borrow().recovery_candidates().contains(id)),
        );
        if let Some(note) = selected {
            if editing {
                self.set_editor_text(&note.contents);
            } else {
                render_markdown(&self.preview.buffer(), &note.contents);
            }
        } else {
            render_markdown(&self.preview.buffer(), &tr("Create a note with Ctrl+N."));
        }
        self.footer.set_text(&if self.browsing.get() {
            tr("Type to filter · Up/Down select · Enter open · Esc return")
        } else if editing {
            tr("Ctrl+Enter save · Ctrl+F find · Esc preview · Ctrl+N new · Ctrl+P notes")
        } else {
            tr("Enter edit · Ctrl+N new · Ctrl+P notes · Ctrl+F find · Esc close")
        });
        self.refresh_header_actions();
        if self.browsing.get() {
            self.refresh_browser_rows();
        }
    }

    fn set_editor_text(&self, contents: &str) {
        let buffer = self.editor.buffer();
        let current = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
        if current.as_str() == contents {
            return;
        }
        self.suppress_editor_change.set(true);
        // Loading another saved note is not an edit the user should be able to undo into.
        // Disable/re-enable undo around this replacement so the next Ctrl+Z starts from the
        // selected note's actual contents.
        buffer.set_enable_undo(false);
        buffer.set_text(contents);
        buffer.set_enable_undo(true);
        self.suppress_editor_change.set(false);
    }

    fn refresh_header_actions(&self) {
        let has_note = self.selected_id.borrow().is_some();
        // Actions are associated with the header widgets and are hidden when there is no note.
        if let Some(header) = self
            .root
            .first_child()
            .and_then(|child| child.downcast::<gtk::Box>().ok())
        {
            let mut child = header.first_child();
            while let Some(widget) = child {
                let next = widget.next_sibling();
                if widget.widget_name() == "notes-edit-control"
                    || widget.widget_name() == "notes-more-control"
                {
                    widget.set_visible(has_note && !self.browsing.get());
                }
                if widget.widget_name() == "notes-browse-control" {
                    widget.set_visible(!self.browsing.get());
                }
                child = next;
            }
        }
    }

    fn set_status(&self, message: Option<&str>, error: bool) {
        if let Some(message) = message {
            self.status.set_text(message);
            self.status.set_visible(true);
            if error {
                self.status.add_css_class("error");
            } else {
                self.status.remove_css_class("error");
            }
        } else {
            self.status.set_visible(false);
            self.status.set_text("");
        }
    }

    fn current_text(&self) -> String {
        let buffer = self.editor.buffer();
        buffer
            .text(&buffer.start_iter(), &buffer.end_iter(), false)
            .to_string()
    }

    fn schedule_recovery(this: &Rc<Self>) {
        let Some(id) = this.selected_id.borrow().clone() else {
            return;
        };
        let contents = this.current_text();
        this.recovery_generation
            .set(this.recovery_generation.get().wrapping_add(1));
        let generation = this.recovery_generation.get();
        let weak = Rc::downgrade(this);
        glib::timeout_add_local_once(Duration::from_millis(350), move || {
            let Some(inner) = weak.upgrade() else { return };
            if inner.recovery_generation.get() != generation || !inner.editing.get() {
                return;
            }
            let result = inner.store.borrow_mut().write_recovery(&id, &contents);
            match result {
                Ok(()) => {
                    inner.set_status(Some(&tr("Draft saved for recovery.")), false);
                    inner.refresh_selected();
                }
                // Preserve the editor buffer if recovery storage fails (for example, when a
                // note exceeds the size limit). Refreshing from the store here would replace
                // the unsaved text with the older Markdown file contents.
                Err(error) => inner.set_status(Some(&error.to_string()), true),
            }
        });
    }

    fn save_current(&self) -> bool {
        if !self.editing.get() {
            return true;
        }
        let Some(id) = self.selected_id.borrow().clone() else {
            self.editing.set(false);
            self.refresh_selected();
            return true;
        };
        self.recovery_generation
            .set(self.recovery_generation.get().wrapping_add(1));
        let contents = self.current_text();
        let result = {
            let mut store = self.store.borrow_mut();
            store
                .write_recovery(&id, &contents)
                .and_then(|()| store.flush(&id, &contents))
        };
        match result {
            Ok(()) => {
                self.editing.set(false);
                self.set_status(None, false);
                self.refresh_selected();
                self.preview.grab_focus();
                true
            }
            Err(error) => {
                self.set_status(Some(&error.to_string()), true);
                false
            }
        }
    }

    fn begin_editing(&self) -> bool {
        let Some(id) = self.selected_id.borrow().clone() else {
            return false;
        };
        if self.store.borrow().recovery_candidates().contains(&id) {
            self.set_status(
                Some(&tr(
                    "Restore or discard the recovered draft before editing this note.",
                )),
                true,
            );
            return false;
        }
        self.browsing.set(false);
        self.editing.set(true);
        self.refresh_selected();
        self.editor.grab_focus();
        let buffer = self.editor.buffer();
        buffer.place_cursor(&buffer.end_iter());
        true
    }

    fn create_blank(this: &Rc<Self>) {
        if !this.save_current() {
            return;
        }
        let created = { this.store.borrow_mut().create("") };
        match created {
            Ok(note) => {
                *this.selected_id.borrow_mut() = Some(note.id);
                this.browsing.set(false);
                this.editing.set(true);
                this.set_status(None, false);
                this.refresh_selected();
                this.editor.grab_focus();
                this.editor
                    .buffer()
                    .place_cursor(&this.editor.buffer().end_iter());
            }
            Err(error) => this.set_status(Some(&error.to_string()), true),
        }
    }

    fn enter_browser(&self) {
        if self.editing.get() && !self.save_current() {
            return;
        }
        self.browsing.set(true);
        self.editing.set(false);
        self.refresh_selected();
        self.browse_search.grab_focus();
        self.refresh_browser_rows();
    }

    fn leave_browser(&self) {
        self.browsing.set(false);
        self.refresh_selected();
        self.preview.grab_focus();
    }

    fn refresh_browser_rows(&self) {
        while let Some(child) = self.browse_list.first_child() {
            self.browse_list.remove(&child);
        }
        let query = self.browse_search.text().to_string();
        let selected_id = self.selected_id.borrow().clone();
        let notes: Vec<MarkdownNote> = self
            .store
            .borrow()
            .notes()
            .iter()
            .filter(|note| note.matches(&query))
            .cloned()
            .collect();
        let mut ids = Vec::with_capacity(notes.len());
        for note in notes {
            let heading = gtk::Label::builder()
                .label(note.title())
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .css_classes(["heading"])
                .build();
            let snippet = gtk::Label::builder()
                .label(
                    note.contents
                        .lines()
                        .find(|line| !line.trim().is_empty())
                        .unwrap_or(""),
                )
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .css_classes(["dim-label"])
                .build();
            let row_box = gtk::Box::new(gtk::Orientation::Vertical, 3);
            row_box.set_margin_top(8);
            row_box.set_margin_bottom(8);
            row_box.set_margin_start(10);
            row_box.set_margin_end(10);
            row_box.append(&heading);
            row_box.append(&snippet);
            let row = gtk::ListBoxRow::new();
            row.set_child(Some(&row_box));
            row.set_widget_name(&note.id);
            self.browse_list.append(&row);
            if selected_id.as_deref() == Some(note.id.as_str()) {
                self.browse_list.select_row(Some(&row));
            }
            ids.push(note.id);
        }
        if !ids.is_empty()
            && self.browse_list.selected_row().is_none()
            && let Some(row) = self.browse_list.row_at_index(0)
        {
            self.browse_list.select_row(Some(&row));
        }
        *self.browse_ids.borrow_mut() = ids;
    }

    fn activate_selected_row(&self) {
        let Some(row) = self.browse_list.selected_row() else {
            return;
        };
        let Some(id) = self.browse_ids.borrow().get(row.index() as usize).cloned() else {
            return;
        };
        *self.selected_id.borrow_mut() = Some(id);
        self.browsing.set(false);
        self.editing.set(false);
        self.set_status(None, false);
        self.refresh_selected();
        self.preview.grab_focus();
    }

    fn show_find(this: &Rc<Self>) {
        if !this.editing.get() && !this.begin_editing() {
            return;
        }
        this.find_bar.set_visible(true);
        this.find_entry.grab_focus();
        this.find_entry.select_region(0, -1);
        this.select_find_match(false);
    }

    fn close_find(&self) {
        self.find_bar.set_visible(false);
        self.editor.grab_focus();
    }

    fn select_find_match(&self, backwards: bool) {
        let needle = self.find_entry.text().to_string();
        if needle.is_empty() {
            return;
        }
        let text = self.current_text();
        let folded = text.to_lowercase();
        let needle_folded = needle.to_lowercase();
        let matches: Vec<(usize, usize)> = folded
            .match_indices(&needle_folded)
            .map(|(start, value)| (start, start + value.len()))
            .collect();
        if matches.is_empty() {
            self.set_status(Some(&tr("No matches found.")), false);
            return;
        }
        let buffer = self.editor.buffer();
        let current = buffer
            .selection_bounds()
            .and_then(|(_, end)| usize::try_from(end.offset()).ok())
            .unwrap_or(0);
        let selected = if backwards {
            matches
                .iter()
                .rev()
                .find(|(_, end)| *end < current)
                .copied()
                .unwrap_or_else(|| *matches.last().unwrap_or(&(0, 0)))
        } else {
            matches
                .iter()
                .find(|(start, _)| *start >= current)
                .copied()
                .unwrap_or(matches[0])
        };
        // TextBuffer offsets count Unicode characters. Convert the byte positions from the
        // folded search string back to character offsets before selecting the source range.
        let start = folded[..selected.0].chars().count();
        let end = folded[..selected.1].chars().count();
        let start = i32::try_from(start).unwrap_or(i32::MAX);
        let end = i32::try_from(end).unwrap_or(i32::MAX);
        let begin = buffer.iter_at_offset(start);
        let finish = buffer.iter_at_offset(end);
        buffer.select_range(&finish, &begin);
        let mut scroll = begin;
        let _ = self
            .editor
            .scroll_to_iter(&mut scroll, 0.25, false, 0.0, 0.0);
        self.set_status(None, false);
    }

    fn restore_recovery(&self) {
        if self.editing.get() {
            return;
        }
        let Some(id) = self.selected_id.borrow().clone() else {
            return;
        };
        let result = self.store.borrow_mut().restore_recovery(&id);
        match result {
            Ok(()) => {
                self.set_status(Some(&tr("Recovered draft restored.")), false);
                self.editing.set(false);
                self.refresh_selected();
            }
            Err(error) => self.set_status(Some(&error.to_string()), true),
        }
    }

    fn discard_recovery(&self) {
        if self.editing.get() {
            return;
        }
        let Some(id) = self.selected_id.borrow().clone() else {
            return;
        };
        match self.store.borrow_mut().discard_recovery(&id) {
            Ok(()) => {
                self.set_status(Some(&tr("Recovered draft discarded.")), false);
                self.refresh_selected();
            }
            Err(error) => self.set_status(Some(&error.to_string()), true),
        }
    }

    fn export_current(this: &Rc<Self>) {
        if this.editing.get() && !this.save_current() {
            return;
        }
        let Some(id) = this.selected_id.borrow().clone() else {
            return;
        };
        let title = this
            .selected_note()
            .map_or_else(|| tr("note"), |note| note.title());
        let safe_name = title
            .chars()
            .map(|character| {
                if character.is_control() || matches!(character, '/' | '\\') {
                    '-'
                } else {
                    character
                }
            })
            .collect::<String>();
        let dialog = gtk::FileDialog::builder()
            .title(tr("Export Markdown"))
            .modal(true)
            .initial_name(format!("{safe_name}.md"))
            .build();
        let Some(window) = this
            .root
            .root()
            .and_then(|root| root.downcast::<gtk::Window>().ok())
        else {
            return;
        };
        let store = this.store.clone();
        let weak = Rc::downgrade(this);
        dialog.save(Some(&window), gtk::gio::Cancellable::NONE, move |result| {
            let Some(path) = result.ok().and_then(|file| file.path()) else {
                return;
            };
            let outcome = store.borrow().export(&id, &path);
            if let Some(inner) = weak.upgrade() {
                match outcome {
                    Ok(()) => inner.set_status(Some(&tr("Markdown note exported.")), false),
                    Err(error) => inner.set_status(Some(&error.to_string()), true),
                }
            }
        });
    }

    fn confirm_delete(this: &Rc<Self>) {
        if this.editing.get() && !this.save_current() {
            return;
        }
        let Some(id) = this.selected_id.borrow().clone() else {
            return;
        };
        let dialog = adw::AlertDialog::new(
            Some(&tr("Delete this note?")),
            Some(&tr(
                "This permanently deletes the note and its recovery draft.",
            )),
        );
        dialog.add_responses(&[("cancel", &tr("Cancel")), ("delete", &tr("Delete Note"))]);
        dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");
        let Some(parent) = this
            .root
            .root()
            .and_then(|root| root.downcast::<gtk::Window>().ok())
        else {
            return;
        };
        let weak = Rc::downgrade(this);
        dialog.choose(
            Some(&parent),
            gtk::gio::Cancellable::NONE,
            move |response| {
                if response != "delete" {
                    return;
                }
                let Some(inner) = weak.upgrade() else { return };
                let removed = { inner.store.borrow_mut().remove(&id) };
                match removed {
                    Ok(()) => {
                        let next = inner
                            .store
                            .borrow()
                            .notes()
                            .first()
                            .map(|note| note.id.clone());
                        *inner.selected_id.borrow_mut() = next;
                        inner.editing.set(false);
                        inner.set_status(None, false);
                        inner.refresh_selected();
                    }
                    Err(error) => inner.set_status(Some(&error.to_string()), true),
                }
            },
        );
    }

    fn close(&self) {
        if self.save_current()
            && let Some(on_close) = self.on_close.borrow().clone()
        {
            on_close();
        }
    }

    fn return_to_launcher(&self) {
        if !self.save_current() {
            return;
        }
        if let Some(on_close) = self.on_close.borrow().clone() {
            on_close();
        }
        if let Some(on_browse) = self.on_browse.borrow().clone() {
            on_browse();
        }
    }
}

/// Open notes in a small floating window. A failed write prevents the window from closing so the
/// user's editor contents stay available for another save attempt.
pub fn open_notes(
    app: &adw::Application,
    ctx: &Rc<AppContext>,
    initial_id: Option<&str>,
    editing: bool,
    on_browse: impl Fn() + 'static,
) -> Result<NotesPage, ConfigError> {
    if let Some(inner) = ACTIVE_NOTES.with(|active| active.borrow().upgrade())
        && let Some(window) = inner
            .window
            .borrow()
            .as_ref()
            .and_then(glib::WeakRef::upgrade)
        && window.is_visible()
    {
        let page = NotesPage {
            inner: inner.clone(),
        };
        page.begin(initial_id, editing);
        window.present();
        page.focus();
        return Ok(page);
    }
    let store = Rc::new(RefCell::new(MarkdownNotesStore::open(
        &ctx.xdg.app_data_dir(),
    )?));
    let page = NotesPage::new(store);
    let window = adw::Window::builder()
        .application(app)
        .title(tr("Floating Markdown Notes"))
        .default_width(640)
        .default_height(520)
        .content(page.widget())
        .build();
    *page.inner.window.borrow_mut() = Some(window.upcast_ref::<gtk::Window>().downgrade());
    ACTIVE_NOTES.with(|active| *active.borrow_mut() = Rc::downgrade(&page.inner));
    let weak_window = window.downgrade();
    page.connect(
        move || {
            if let Some(window) = weak_window.upgrade() {
                window.close();
            }
        },
        on_browse,
    );
    let close_page = page.clone();
    window.connect_close_request(move |_| {
        if close_page.save_current() {
            ACTIVE_NOTES.with(|active| {
                let mut active = active.borrow_mut();
                if active
                    .upgrade()
                    .is_some_and(|current| Rc::ptr_eq(&current, &close_page.inner))
                {
                    *active = Weak::new();
                }
            });
            glib::Propagation::Proceed
        } else {
            glib::Propagation::Stop
        }
    });
    page.begin(initial_id, editing);
    window.present();
    page.focus();
    Ok(page)
}

fn connect_actions(inner: &Rc<Inner>, buttons: [&gtk::Button; 8]) {
    let [
        edit,
        new_note,
        browse,
        to_launcher,
        restore,
        discard,
        export,
        delete,
    ] = buttons;
    edit.set_widget_name("notes-edit-control");
    new_note.set_widget_name("notes-new-control");
    browse.set_widget_name("notes-browse-control");
    to_launcher.set_widget_name("notes-launcher-control");
    // The More menu owns its action controls; their parent chain isn't in the root header, so
    // the menu itself remains available and its actions safely no-op when no note is selected.
    export.set_widget_name("notes-export-control");
    delete.set_widget_name("notes-delete-control");

    let group = gio::SimpleActionGroup::new();
    for (name, callback) in [
        (
            "new",
            Rc::new({
                let weak = Rc::downgrade(inner);
                move || {
                    if let Some(inner) = weak.upgrade() {
                        Inner::create_blank(&inner);
                    }
                }
            }) as PlainCallback,
        ),
        (
            "toggle-edit",
            Rc::new({
                let weak = Rc::downgrade(inner);
                move || {
                    let Some(inner) = weak.upgrade() else { return };
                    if inner.editing.get() {
                        inner.save_current();
                    } else {
                        inner.begin_editing();
                    }
                }
            }) as PlainCallback,
        ),
        (
            "browse",
            Rc::new({
                let weak = Rc::downgrade(inner);
                move || {
                    if let Some(inner) = weak.upgrade() {
                        inner.enter_browser();
                    }
                }
            }) as PlainCallback,
        ),
        (
            "launcher",
            Rc::new({
                let weak = Rc::downgrade(inner);
                move || {
                    if let Some(inner) = weak.upgrade() {
                        inner.return_to_launcher();
                    }
                }
            }) as PlainCallback,
        ),
        (
            "restore",
            Rc::new({
                let weak = Rc::downgrade(inner);
                move || {
                    if let Some(inner) = weak.upgrade() {
                        inner.restore_recovery();
                    }
                }
            }) as PlainCallback,
        ),
        (
            "discard",
            Rc::new({
                let weak = Rc::downgrade(inner);
                move || {
                    if let Some(inner) = weak.upgrade() {
                        inner.discard_recovery();
                    }
                }
            }) as PlainCallback,
        ),
        (
            "export",
            Rc::new({
                let weak = Rc::downgrade(inner);
                move || {
                    if let Some(inner) = weak.upgrade() {
                        Inner::export_current(&inner);
                    }
                }
            }) as PlainCallback,
        ),
        (
            "delete",
            Rc::new({
                let weak = Rc::downgrade(inner);
                move || {
                    if let Some(inner) = weak.upgrade() {
                        Inner::confirm_delete(&inner);
                    }
                }
            }) as PlainCallback,
        ),
    ] {
        let action = gio::SimpleAction::new(name, None);
        action.connect_activate(move |_, _| callback());
        group.add_action(&action);
    }
    inner.root.insert_action_group("notes", Some(&group));
    let _ = (restore, discard);
}

fn connect_editor(inner: &Rc<Inner>) {
    let weak = Rc::downgrade(inner);
    inner.editor.buffer().connect_changed(move |_| {
        let Some(inner) = weak.upgrade() else { return };
        if inner.suppress_editor_change.get() || !inner.editing.get() {
            return;
        }
        Inner::schedule_recovery(&inner);
    });
}

fn connect_browser(inner: &Rc<Inner>) {
    let weak = Rc::downgrade(inner);
    inner.browse_search.connect_changed(move |_| {
        if let Some(inner) = weak.upgrade() {
            inner.refresh_browser_rows();
        }
    });
    let weak = Rc::downgrade(inner);
    inner.browse_list.connect_row_activated(move |_, _| {
        if let Some(inner) = weak.upgrade() {
            inner.activate_selected_row();
        }
    });
}

fn connect_find(inner: &Rc<Inner>) {
    let weak = Rc::downgrade(inner);
    inner.find_entry.connect_changed(move |_| {
        if let Some(inner) = weak.upgrade() {
            inner.select_find_match(false);
        }
    });
    let weak = Rc::downgrade(inner);
    inner.find_entry.connect_activate(move |_| {
        if let Some(inner) = weak.upgrade() {
            inner.select_find_match(false);
        }
    });
}

fn connect_keys(inner: &Rc<Inner>) {
    // Commands such as Ctrl+N must run before TextView's built-in keybindings. Escape is
    // different: let the focused text input process it first so an IME can cancel preedit.
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = Rc::downgrade(inner);
    keys.connect_key_pressed(move |_, key, _, state| {
        if key == gdk::Key::Escape {
            return glib::Propagation::Proceed;
        }
        weak.upgrade().map_or(glib::Propagation::Proceed, |inner| {
            handle_key(&inner, key, state)
        })
    });
    inner.root.add_controller(keys);

    let escape_keys = gtk::EventControllerKey::new();
    escape_keys.set_propagation_phase(gtk::PropagationPhase::Bubble);
    let weak = Rc::downgrade(inner);
    escape_keys.connect_key_pressed(move |_, key, _, state| {
        if key != gdk::Key::Escape {
            return glib::Propagation::Proceed;
        }
        weak.upgrade().map_or(glib::Propagation::Proceed, |inner| {
            handle_key(&inner, key, state)
        })
    });
    inner.root.add_controller(escape_keys);
}

fn handle_key(inner: &Rc<Inner>, key: gdk::Key, state: gdk::ModifierType) -> glib::Propagation {
    let control = state.contains(gdk::ModifierType::CONTROL_MASK);
    let shift = state.contains(gdk::ModifierType::SHIFT_MASK);
    let alt = state.contains(gdk::ModifierType::ALT_MASK);
    if alt || state.contains(gdk::ModifierType::SUPER_MASK) {
        return glib::Propagation::Proceed;
    }
    if key == gdk::Key::Escape && !control && !shift {
        if inner.more_popover.is_visible() {
            inner.more_popover.popdown();
        } else if inner.find_bar.is_visible() {
            inner.close_find();
        } else if inner.browsing.get() {
            inner.leave_browser();
        } else if inner.editing.get() {
            inner.save_current();
        } else {
            inner.close();
        }
        return glib::Propagation::Stop;
    }
    if !control && !shift && matches!(key, gdk::Key::Return | gdk::Key::KP_Enter) {
        if !inner.editing.get() && !inner.browsing.get() && inner.preview.has_focus() {
            inner.begin_editing();
            return glib::Propagation::Stop;
        }
        if inner.browsing.get() {
            inner.activate_selected_row();
            return glib::Propagation::Stop;
        }
        if inner.find_bar.is_visible() {
            inner.select_find_match(false);
            return glib::Propagation::Stop;
        }
        return glib::Propagation::Proceed;
    }
    if !control {
        if inner.browsing.get() && key == gdk::Key::Up {
            let current = inner
                .browse_list
                .selected_row()
                .map_or(0, |row| row.index());
            if let Some(row) = inner.browse_list.row_at_index((current - 1).max(0)) {
                inner.browse_list.select_row(Some(&row));
            }
            return glib::Propagation::Stop;
        }
        if inner.browsing.get() && key == gdk::Key::Down {
            let current = inner
                .browse_list
                .selected_row()
                .map_or(-1, |row| row.index());
            if let Some(row) = inner.browse_list.row_at_index(current + 1) {
                inner.browse_list.select_row(Some(&row));
            }
            return glib::Propagation::Stop;
        }
        return glib::Propagation::Proceed;
    }
    let character = key
        .to_unicode()
        .map(|character| character.to_ascii_lowercase());
    match (character, key, shift) {
        (Some('n'), _, false) => Inner::create_blank(inner),
        (Some('p'), _, false) => inner.enter_browser(),
        (Some('f'), _, false) => Inner::show_find(inner),
        (Some('e'), _, false) => {
            if inner.editing.get() {
                inner.save_current();
            } else {
                inner.begin_editing();
            }
        }
        (Some('s'), _, false) => {
            if inner.editing.get() {
                // Ctrl+S saves the note and returns to its preview.
                inner.save_current();
            }
        }
        (Some('w'), _, false) => inner.close(),
        (Some('g'), _, true) => inner.select_find_match(true),
        (Some('g'), _, false) => inner.select_find_match(false),
        (Some('z'), _, true) if inner.editor.has_focus() => inner.editor.buffer().redo(),
        (Some('z'), _, false) if inner.editor.has_focus() => inner.editor.buffer().undo(),
        (Some('r'), _, true) if !inner.editing.get() => inner.restore_recovery(),
        (Some('d'), _, true) if !inner.editing.get() => inner.discard_recovery(),
        (Some('e'), _, true) => Inner::export_current(inner),
        (_, gdk::Key::Delete, false) if inner.preview.has_focus() => {
            Inner::confirm_delete(inner);
        }
        (Some('p'), _, true) => {
            inner.return_to_launcher();
        }
        (None, gdk::Key::Return | gdk::Key::KP_Enter, true) => {
            if inner.editing.get() {
                inner.save_current();
            }
        }
        _ => return glib::Propagation::Proceed,
    }
    glib::Propagation::Stop
}

fn markdown_preview_view() -> gtk::TextView {
    let view = gtk::TextView::builder()
        .editable(false)
        .cursor_visible(false)
        .wrap_mode(gtk::WrapMode::WordChar)
        .top_margin(16)
        .bottom_margin(16)
        .left_margin(16)
        .right_margin(16)
        .css_classes(["kb-preview"])
        .build();
    view.update_property(&[gtk::accessible::Property::Label(&tr("Markdown preview"))]);
    view
}

fn render_markdown(buffer: &gtk::TextBuffer, source: &str) {
    let table = buffer.tag_table();
    for (name, weight, scale, style, family) in [
        ("md-h1", 700, 1.5, gtk::pango::Style::Normal, ""),
        ("md-h2", 700, 1.3, gtk::pango::Style::Normal, ""),
        ("md-h3", 700, 1.15, gtk::pango::Style::Normal, ""),
        ("md-bold", 700, 1.0, gtk::pango::Style::Normal, ""),
        ("md-italic", 400, 1.0, gtk::pango::Style::Italic, ""),
        ("md-code", 400, 0.98, gtk::pango::Style::Normal, "monospace"),
        (
            "md-code-block",
            400,
            1.0,
            gtk::pango::Style::Normal,
            "monospace",
        ),
        (
            "md-table",
            400,
            0.98,
            gtk::pango::Style::Normal,
            "monospace",
        ),
        ("md-quote", 400, 1.0, gtk::pango::Style::Italic, ""),
    ] {
        if table.lookup(name).is_none() {
            let mut builder = gtk::TextTag::builder()
                .name(name)
                .weight(weight)
                .scale(scale)
                .style(style);
            if !family.is_empty() {
                builder = builder.family(family);
            }
            let _ = table.add(&builder.build());
        }
    }
    if table.lookup("md-link").is_none() {
        let _ = table.add(
            &gtk::TextTag::builder()
                .name("md-link")
                .underline(gtk::pango::Underline::Single)
                .build(),
        );
    }
    if table.lookup("md-strike").is_none() {
        let _ = table.add(
            &gtk::TextTag::builder()
                .name("md-strike")
                .strikethrough(true)
                .build(),
        );
    }
    buffer.set_text("");
    let mut iter = buffer.end_iter();
    let mut character_offset = 0usize;
    let mut spans: Vec<(usize, usize, &'static str)> = Vec::new();
    let mut code_fence: Option<char> = None;
    for line in source.split_inclusive('\n') {
        let content = line.strip_suffix('\n').unwrap_or(line);
        let trimmed = content.trim_start();
        if let Some(fence) = code_fence {
            let closing = trimmed
                .chars()
                .take_while(|character| *character == fence)
                .count()
                >= 3;
            if closing {
                code_fence = None;
                continue;
            }
            let line_start = character_offset;
            buffer.insert(&mut iter, content);
            character_offset += content.chars().count();
            spans.push((line_start, character_offset, "md-code-block"));
            if line.ends_with('\n') {
                buffer.insert(&mut iter, "\n");
                character_offset += 1;
            }
            continue;
        }
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            code_fence = trimmed.chars().next();
            continue;
        }
        if let Some(table_row) = markdown_table_row(trimmed) {
            if markdown_table_separator(&table_row) {
                if line.ends_with('\n') {
                    buffer.insert(&mut iter, "\n");
                    character_offset += 1;
                }
            } else {
                let line_start = character_offset;
                buffer.insert(&mut iter, &table_row);
                character_offset += table_row.chars().count();
                spans.push((line_start, character_offset, "md-table"));
                if line.ends_with('\n') {
                    buffer.insert(&mut iter, "\n");
                    character_offset += 1;
                }
            }
            continue;
        }
        let (prefix, text, block_tag) = if let Some((hashes, remainder)) = heading(trimmed) {
            ("", remainder, Some(hashes))
        } else if let Some(quote) = trimmed
            .strip_prefix("> ")
            .or_else(|| trimmed.strip_prefix('>'))
        {
            ("│ ", quote.trim_start(), Some("md-quote"))
        } else if let Some(item) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
            .or_else(|| trimmed.strip_prefix("+ "))
        {
            ("• ", item, None)
        } else if let Some((_, item)) = trimmed.split_once(". ").filter(|(number, _)| {
            !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())
        }) {
            ("• ", item, None)
        } else {
            ("", content, None)
        };
        if !prefix.is_empty() {
            buffer.insert(&mut iter, prefix);
            character_offset += prefix.chars().count();
        }
        let rendered = render_inline(text);
        let line_start = character_offset;
        buffer.insert(&mut iter, &rendered.0);
        character_offset += rendered.0.chars().count();
        if let Some(tag) = block_tag {
            spans.push((line_start, character_offset, tag));
        }
        spans.extend(
            rendered
                .1
                .into_iter()
                .map(|(start, end, tag)| (line_start + start, line_start + end, tag)),
        );
        if line.ends_with('\n') {
            buffer.insert(&mut iter, "\n");
            character_offset += 1;
        }
    }
    for (start, end, name) in spans {
        if let Some(tag) = table.lookup(name) {
            let start = i32::try_from(start).unwrap_or(i32::MAX);
            let end = i32::try_from(end).unwrap_or(i32::MAX);
            buffer.apply_tag(
                &tag,
                &buffer.iter_at_offset(start),
                &buffer.iter_at_offset(end),
            );
        }
    }
}

fn markdown_table_row(line: &str) -> Option<String> {
    let line = line.trim();
    if !line.starts_with('|') || !line.ends_with('|') {
        return None;
    }
    Some(
        line.trim_matches('|')
            .split('|')
            .map(str::trim)
            .collect::<Vec<_>>()
            .join("  │  "),
    )
}

fn markdown_table_separator(row: &str) -> bool {
    let cells: Vec<&str> = row.split("│").map(str::trim).collect();
    !cells.is_empty()
        && cells.iter().all(|cell| {
            !cell.is_empty()
                && cell
                    .trim_matches(':')
                    .chars()
                    .all(|character| character == '-')
        })
}

fn heading(line: &str) -> Option<(&'static str, &str)> {
    for (prefix, tag) in [("### ", "md-h3"), ("## ", "md-h2"), ("# ", "md-h1")] {
        if let Some(title) = line.strip_prefix(prefix) {
            return Some((tag, title));
        }
    }
    None
}

fn render_inline(text: &str) -> (String, Vec<(usize, usize, &'static str)>) {
    let mut rendered = String::with_capacity(text.len());
    let mut spans = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        if let Some(link) = inline_link(rest) {
            let start = rendered.chars().count();
            rendered.push_str(link.label);
            let finish = rendered.chars().count();
            spans.push((start, finish, "md-link"));
            rest = &rest[link.consumed..];
            continue;
        }
        let pairs: [(&str, &str, &'static str); 5] = [
            ("**", "**", "md-bold"),
            ("__", "__", "md-bold"),
            ("~~", "~~", "md-strike"),
            ("`", "`", "md-code"),
            ("*", "*", "md-italic"),
        ];
        let found = pairs.iter().find_map(|(open, close, tag)| {
            let after_open = rest.strip_prefix(open)?;
            let end = after_open.find(close)?;
            (end > 0).then_some((*open, *close, *tag, end))
        });
        if let Some((open, close, tag, end)) = found {
            let body = &rest[open.len()..open.len() + end];
            let start = rendered.chars().count();
            rendered.push_str(body);
            let finish = rendered.chars().count();
            spans.push((start, finish, tag));
            rest = &rest[open.len() + end + close.len()..];
        } else {
            let Some(character) = rest.chars().next() else {
                break;
            };
            rendered.push(character);
            rest = &rest[character.len_utf8()..];
        }
    }
    (rendered, spans)
}

struct MarkdownLink<'a> {
    label: &'a str,
    consumed: usize,
}

fn inline_link(text: &str) -> Option<MarkdownLink<'_>> {
    let (prefix, body) = if let Some(body) = text.strip_prefix("![") {
        (2, body)
    } else {
        (1, text.strip_prefix('[')?)
    };
    let label_end = body.find("](")?;
    let destination = &body[label_end + 2..];
    let destination_end = destination.find(')')?;
    Some(MarkdownLink {
        label: &body[..label_end],
        consumed: prefix + label_end + 2 + destination_end + 1,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_preview_renders_basic_block_and_inline_syntax() {
        let (line, spans) = render_inline("A **bold** and `code` word.");
        assert_eq!(line, "A bold and code word.");
        assert!(spans.iter().any(|(_, _, tag)| *tag == "md-bold"));
        assert!(spans.iter().any(|(_, _, tag)| *tag == "md-code"));
        assert_eq!(heading("## Plan"), Some(("md-h2", "Plan")));
    }

    #[test]
    fn markdown_preview_renders_links_strikethrough_and_tables_without_source_markup() {
        let (line, spans) = render_inline("[Kebloop](https://example.test) ~~old~~");
        assert_eq!(line, "Kebloop old");
        assert!(spans.iter().any(|(_, _, tag)| *tag == "md-link"));
        assert!(spans.iter().any(|(_, _, tag)| *tag == "md-strike"));
        assert_eq!(
            markdown_table_row("| Name | Count |"),
            Some("Name  │  Count".to_owned())
        );
        assert!(markdown_table_separator("---  │  :---:"));
    }
}
