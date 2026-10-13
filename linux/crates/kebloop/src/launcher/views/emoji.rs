//! Emoji & Symbols: category chips above a virtualised grid, arrow-key navigation while the
//! search field keeps focus.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;
use kebloop_core::{EmojiCatalog, EmojiEntry};

use crate::i18n::tr;

/// Grid columns.
pub const COLUMNS: u32 = 8;

/// Called with the chosen emoji and whether to paste it (`false`: copy only).
type PickCallback = Rc<dyn Fn(&'static EmojiEntry, bool)>;

/// The grid view.
#[derive(Clone)]
pub struct EmojiView {
    inner: Rc<Inner>,
}

struct Inner {
    root: gtk::Box,
    grid: gtk::GridView,
    selection: gtk::SingleSelection,
    model: gtk::StringList,
    name: gtk::Label,
    copy: gtk::Button,
    chips: gtk::Box,
    entries: RefCell<Vec<&'static EmojiEntry>>,
    recents: RefCell<Vec<String>>,
    query: RefCell<String>,
    category: RefCell<String>,
    syncing_chips: Cell<bool>,
    on_pick: RefCell<Option<PickCallback>>,
}

impl EmojiView {
    /// Build the (empty) view.
    pub fn new() -> Self {
        let model = gtk::StringList::new(&[]);
        let selection = gtk::SingleSelection::new(Some(model.clone()));
        selection.set_autoselect(true);
        selection.set_can_unselect(false);
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(|_, object| {
            let Some(item) = object.downcast_ref::<gtk::ListItem>() else {
                return;
            };
            let label = gtk::Label::builder()
                .css_classes(["kb-emoji"])
                .width_request(54)
                .build();
            item.set_child(Some(&label));
        });
        factory.connect_bind(|_, object| {
            let Some(item) = object.downcast_ref::<gtk::ListItem>() else {
                return;
            };
            let text = item
                .item()
                .and_then(|o| o.downcast::<gtk::StringObject>().ok())
                .map(|o| o.string());
            if let (Some(label), Some(text)) = (
                item.child().and_then(|c| c.downcast::<gtk::Label>().ok()),
                text,
            ) {
                label.set_text(&text);
            }
        });
        let grid = gtk::GridView::builder()
            .model(&selection)
            .factory(&factory)
            .min_columns(COLUMNS)
            .max_columns(COLUMNS)
            .single_click_activate(true)
            .focusable(false)
            .build();
        let scroller = gtk::ScrolledWindow::builder()
            .child(&grid)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .min_content_height(260)
            .max_content_height(300)
            .propagate_natural_height(true)
            .focusable(false)
            .build();
        let chips = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        chips.add_css_class("kb-chips");
        let chip_scroller = gtk::ScrolledWindow::builder()
            .child(&chips)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .hscrollbar_policy(gtk::PolicyType::Automatic)
            .focusable(false)
            .build();
        let name = gtk::Label::builder()
            .css_classes(["kb-emoji-name"])
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        let hint = gtk::Label::builder()
            .label(tr("Return pastes · Ctrl+Return copies"))
            .css_classes(["kb-emoji-hint"])
            .xalign(0.0)
            .hexpand(true)
            .build();
        let copy = gtk::Button::builder()
            .label(tr("Copy"))
            .focusable(false)
            .css_classes(["flat"])
            .sensitive(false)
            .build();
        let footer = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(8)
            .margin_start(16)
            .margin_end(10)
            .margin_top(4)
            .margin_bottom(4)
            .build();
        footer.append(&hint);
        footer.append(&copy);
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("kb-emoji-view");
        root.append(&footer);
        root.append(&chip_scroller);
        root.append(&scroller);
        root.append(&name);

        let inner = Rc::new(Inner {
            root,
            grid,
            selection,
            model,
            name,
            copy,
            chips,
            entries: RefCell::new(Vec::new()),
            recents: RefCell::new(Vec::new()),
            query: RefCell::new(String::new()),
            category: RefCell::new("All".into()),
            syncing_chips: Cell::new(false),
            on_pick: RefCell::new(None),
        });
        let view = Self { inner };
        view.build_chips();

        let weak = Rc::downgrade(&view.inner);
        view.inner.grid.connect_activate(move |_, position| {
            let Some(inner) = weak.upgrade() else { return };
            Self { inner }.pick(position, true);
        });
        let weak = Rc::downgrade(&view.inner);
        view.inner.copy.connect_clicked(move |_| {
            let Some(inner) = weak.upgrade() else { return };
            Self { inner }.copy_selected();
        });
        let weak = Rc::downgrade(&view.inner);
        view.inner.selection.connect_selected_notify(move |_| {
            let Some(inner) = weak.upgrade() else { return };
            Self { inner }.update_name();
        });
        view
    }

    /// The widget to put into the panel.
    pub fn widget(&self) -> &gtk::Box {
        &self.inner.root
    }

    /// Called when an emoji is chosen: Return or a click pastes it, Ctrl+Return or the Copy
    /// button only copies it.
    pub fn connect_pick(&self, callback: impl Fn(&'static EmojiEntry, bool) + 'static) {
        *self.inner.on_pick.borrow_mut() = Some(Rc::new(callback));
    }

    fn build_chips(&self) {
        let inner = &self.inner;
        let mut names = vec!["All"];
        names.extend(EmojiCatalog::categories());
        let mut first: Option<gtk::ToggleButton> = None;
        for name in names {
            let button = gtk::ToggleButton::builder()
                .label(category_label(name))
                .focusable(false)
                .css_classes(["flat"])
                .build();
            if let Some(group) = &first {
                button.set_group(Some(group));
            } else {
                button.set_active(true);
                first = Some(button.clone());
            }
            let weak = Rc::downgrade(&self.inner);
            let category = name.to_owned();
            button.connect_toggled(move |button| {
                let Some(inner) = weak.upgrade() else { return };
                if !button.is_active() || inner.syncing_chips.get() {
                    return;
                }
                *inner.category.borrow_mut() = category.clone();
                Self { inner }.rebuild();
            });
            inner.chips.append(&button);
        }
    }

    /// Start a fresh session: empty query, all categories, `recents` first.
    pub fn reset(&self, recents: &[String]) {
        let inner = &self.inner;
        *inner.recents.borrow_mut() = recents.to_vec();
        inner.query.borrow_mut().clear();
        inner.syncing_chips.set(true);
        *inner.category.borrow_mut() = "All".into();
        if let Some(first) = inner
            .chips
            .first_child()
            .and_then(|c| c.downcast::<gtk::ToggleButton>().ok())
        {
            first.set_active(true);
        }
        inner.syncing_chips.set(false);
        self.rebuild();
    }

    /// Filter by text.
    pub fn set_query(&self, query: &str) {
        *self.inner.query.borrow_mut() = query.to_owned();
        self.rebuild();
    }

    fn rebuild(&self) {
        let inner = &self.inner;
        let query = inner.query.borrow().clone();
        let category = inner.category.borrow().clone();
        let mut entries: Vec<&'static EmojiEntry> = EmojiCatalog::search(&query)
            .into_iter()
            .filter(|e| category == "All" || e.category == category)
            .collect();
        if query.trim().is_empty() && category == "All" {
            let recents: Vec<&'static EmojiEntry> = inner
                .recents
                .borrow()
                .iter()
                .filter_map(|symbol| EmojiCatalog::entries().iter().find(|e| e.symbol == symbol))
                .collect();
            let mut with_recents = recents;
            with_recents.extend(entries);
            entries = with_recents;
        }
        let symbols: Vec<&str> = entries.iter().map(|e| e.symbol).collect();
        let count = inner.model.n_items();
        inner.model.splice(0, count, &symbols);
        *inner.entries.borrow_mut() = entries;
        inner.selection.set_selected(0);
        self.update_name();
        if inner.model.n_items() > 0 {
            inner.grid.scroll_to(0, gtk::ListScrollFlags::NONE, None);
        }
    }

    fn update_name(&self) {
        let inner = &self.inner;
        let text = self
            .selected()
            .map(|e| format!("{} · {}", e.name, category_label(e.category)))
            .unwrap_or_else(|| tr("No results"));
        inner.name.set_text(&text);
        inner.copy.set_sensitive(self.selected().is_some());
    }

    /// The highlighted emoji.
    pub fn selected(&self) -> Option<&'static EmojiEntry> {
        let position = self.inner.selection.selected();
        self.inner
            .entries
            .borrow()
            .get(usize::try_from(position).ok()?)
            .copied()
    }

    /// Move the highlight by whole cells; vertical moves are one row (`COLUMNS` cells).
    pub fn move_by(&self, dx: isize, dy: isize) {
        let count = self.inner.entries.borrow().len();
        if count == 0 {
            return;
        }
        let current = usize::try_from(self.inner.selection.selected()).unwrap_or(0);
        let delta = dx + dy * COLUMNS as isize;
        let next = current.saturating_add_signed(delta).min(count - 1);
        self.select(next);
    }

    /// Move by `pages` pages of five rows.
    pub fn page(&self, pages: isize) {
        self.move_by(0, pages * 5);
    }

    fn select(&self, position: usize) {
        let inner = &self.inner;
        let position = u32::try_from(position).unwrap_or(0);
        inner.selection.set_selected(position);
        inner
            .grid
            .scroll_to(position, gtk::ListScrollFlags::NONE, None);
    }

    /// Paste the highlighted emoji.
    pub fn activate_selected(&self) {
        self.pick(self.inner.selection.selected(), true);
    }

    /// Copy the highlighted emoji without pasting it.
    pub fn copy_selected(&self) {
        self.pick(self.inner.selection.selected(), false);
    }

    fn pick(&self, position: u32, paste: bool) {
        let entry = self.inner.entries.borrow().get(position as usize).copied();
        let callback = self.inner.on_pick.borrow().clone();
        if let (Some(entry), Some(callback)) = (entry, callback) {
            callback(entry, paste);
        }
    }
}

/// Localized name of an emoji category (`"All"` or a catalog category).
pub fn category_label(category: &str) -> String {
    tr(category)
}

impl Default for EmojiView {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use kebloop_core::EmojiCatalog;
    use kebloop_services::l10n::{Language, catalog_keys};

    #[test]
    fn every_emoji_category_is_translated_to_polish() {
        let polish = catalog_keys(Language::Polish);
        let english = catalog_keys(Language::English);
        for category in std::iter::once("All").chain(EmojiCatalog::categories()) {
            assert!(
                english.contains(category),
                "no English entry for {category:?}"
            );
            assert!(
                polish.contains(category),
                "no Polish entry for {category:?}"
            );
        }
    }
}
