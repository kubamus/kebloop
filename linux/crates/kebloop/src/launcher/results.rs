//! The result list: a reused `GtkListBox` whose rows are updated in place, so typing never
//! creates or destroys widgets once the pool has grown to the largest list seen.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;

use super::model::{IconSpec, ResultItem};
use crate::i18n::tr;
use crate::icons;
use kebloop_services::result_action::row_accessibility_label;

/// Decoded thumbnails kept at most; the cache starts over beyond this.
const MAX_CACHED_THUMBNAILS: usize = 256;

type IndexCallback = Rc<dyn Fn(usize)>;

struct Row {
    widget: gtk::ListBoxRow,
    icon: gtk::Image,
    title: gtk::Label,
    subtitle: gtk::Label,
    pin: gtk::Image,
    hint: gtk::Label,
    remove: gtk::Button,
    index: Rc<Cell<usize>>,
    shown_icon: RefCell<Option<IconSpec>>,
    thumbnail_id: RefCell<Option<String>>,
}

/// Options that change how rows look.
#[derive(Debug, Clone, Copy)]
pub struct RowStyle {
    /// Show the `↵` hint on the selected row.
    pub show_hint: bool,
    /// Icon size in px.
    pub icon_px: i32,
}

/// Scrollable, keyboard-driven list of [`ResultItem`]s.
#[derive(Clone)]
pub struct ResultList {
    inner: Rc<Inner>,
}

struct Inner {
    root: gtk::Box,
    scroller: gtk::ScrolledWindow,
    list: gtk::ListBox,
    empty: gtk::Label,
    rows: RefCell<Vec<Row>>,
    count: Cell<usize>,
    selected: Cell<usize>,
    style: Cell<RowStyle>,
    thumbnails: RefCell<HashMap<String, gdk::Texture>>,
    on_activate: RefCell<Option<IndexCallback>>,
    on_context: RefCell<Option<IndexCallback>>,
    on_remove: RefCell<Option<IndexCallback>>,
}

impl ResultList {
    /// An empty list.
    pub fn new() -> Self {
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .activate_on_single_click(true)
            .css_classes(["kb-results"])
            .focusable(false)
            .build();
        let scroller = gtk::ScrolledWindow::builder()
            .child(&list)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .min_content_height(0)
            .max_content_height(340)
            .focusable(false)
            .build();
        let empty = gtk::Label::builder()
            .css_classes(["kb-empty"])
            .wrap(true)
            .xalign(0.5)
            .justify(gtk::Justification::Center)
            .visible(false)
            .build();
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.append(&empty);
        root.append(&scroller);
        let inner = Rc::new(Inner {
            root,
            scroller,
            list,
            empty,
            rows: RefCell::new(Vec::new()),
            count: Cell::new(0),
            selected: Cell::new(0),
            style: Cell::new(RowStyle {
                show_hint: true,
                icon_px: 32,
            }),
            thumbnails: RefCell::new(HashMap::new()),
            on_activate: RefCell::new(None),
            on_context: RefCell::new(None),
            on_remove: RefCell::new(None),
        });
        let weak = Rc::downgrade(&inner);
        inner.list.connect_row_activated(move |_, row| {
            let Some(inner) = weak.upgrade() else { return };
            let index = usize::try_from(row.index()).unwrap_or(usize::MAX);
            let callback = inner.on_activate.borrow().clone();
            if let Some(callback) = callback {
                callback(index);
            }
        });
        Self { inner }
    }

    /// The widget to put into the window.
    pub fn widget(&self) -> &gtk::Box {
        &self.inner.root
    }

    /// The scroller (for max-height changes).
    pub fn set_max_height(&self, height: i32) {
        self.inner.scroller.set_max_content_height(height);
    }

    /// Called when a row is clicked or activated.
    pub fn connect_activate(&self, callback: impl Fn(usize) + 'static) {
        *self.inner.on_activate.borrow_mut() = Some(Rc::new(callback));
    }

    /// Called on right-click of a row.
    pub fn connect_context(&self, callback: impl Fn(usize) + 'static) {
        *self.inner.on_context.borrow_mut() = Some(Rc::new(callback));
    }

    /// Called when the row's remove button is pressed (clipboard history).
    pub fn connect_remove(&self, callback: impl Fn(usize) + 'static) {
        *self.inner.on_remove.borrow_mut() = Some(Rc::new(callback));
    }

    /// Update the row look.
    pub fn set_style(&self, style: RowStyle) {
        self.inner.style.set(style);
    }

    /// Currently selected index.
    pub fn selected(&self) -> usize {
        self.inner.selected.get()
    }

    /// Number of visible rows.
    pub fn len(&self) -> usize {
        self.inner.count.get()
    }

    /// Replace the rows. `selected` is clamped; `with_remove` shows a remove button per row;
    /// rows whose id is in `pinned` carry the favorites pin. `missing_thumbnail` is called once
    /// per clipboard image id without a cached thumbnail.
    pub fn set_items(
        &self,
        items: &[ResultItem],
        selected: usize,
        empty_message: &str,
        with_remove: bool,
        pinned: &HashSet<String>,
        missing_thumbnail: &dyn Fn(&str),
    ) {
        let inner = &self.inner;
        inner.count.set(items.len());
        let style = inner.style.get();
        self.grow(items.len());
        let rows = inner.rows.borrow();
        for (position, (row, item)) in rows.iter().zip(items).enumerate() {
            row.index.set(position);
            row.widget.set_visible(true);
            set_text(&row.title, &item.title);
            set_text(&row.subtitle, &item.subtitle);
            row.subtitle.set_visible(!item.subtitle.is_empty());
            row.remove.set_visible(with_remove);
            let is_pinned = pinned.contains(&item.id);
            row.pin.set_visible(is_pinned);
            row.hint.set_text("");
            let spec_changed = row.shown_icon.borrow().as_ref() != Some(&item.icon);
            if let IconSpec::ClipboardImage(id) = &item.icon {
                let cached = inner.thumbnails.borrow().get(id).cloned();
                match cached {
                    Some(texture) => {
                        row.icon.set_paintable(Some(&texture));
                        row.icon.set_pixel_size(style.icon_px);
                        row.icon.add_css_class("kb-thumbnail");
                    }
                    None => {
                        icons::apply(&row.icon, &item.icon, style.icon_px);
                        missing_thumbnail(id);
                    }
                }
                *row.shown_icon.borrow_mut() = None;
                *row.thumbnail_id.borrow_mut() = Some(id.clone());
            } else if spec_changed {
                *row.thumbnail_id.borrow_mut() = None;
                row.icon.remove_css_class("kb-thumbnail");
                icons::apply(&row.icon, &item.icon, style.icon_px);
                *row.shown_icon.borrow_mut() = Some(item.icon.clone());
            }
            row.widget
                .update_property(&[gtk::accessible::Property::Label(&row_accessibility_label(
                    &item.title,
                    &item.subtitle,
                    &item.action,
                    is_pinned,
                    crate::i18n::localizer(),
                ))]);
        }
        for row in rows.iter().skip(items.len()) {
            row.widget.set_visible(false);
        }
        drop(rows);
        inner.empty.set_text(empty_message);
        inner.empty.set_visible(items.is_empty());
        inner.scroller.set_visible(!items.is_empty());
        self.select(selected, true);
    }

    fn grow(&self, wanted: usize) {
        let inner = &self.inner;
        let mut rows = inner.rows.borrow_mut();
        while rows.len() < wanted {
            let row = self.build_row();
            inner.list.append(&row.widget);
            rows.push(row);
        }
    }

    fn build_row(&self) -> Row {
        let icon = gtk::Image::builder().pixel_size(32).build();
        let title = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["kb-title"])
            .build();
        let subtitle = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .css_classes(["kb-subtitle"])
            .build();
        let text = gtk::Box::new(gtk::Orientation::Vertical, 1);
        text.set_valign(gtk::Align::Center);
        text.set_hexpand(true);
        text.append(&title);
        text.append(&subtitle);
        let pin = gtk::Image::builder()
            .icon_name("view-pin-symbolic")
            .pixel_size(12)
            .css_classes(["kb-pin"])
            .tooltip_text(tr("Pinned to Favorites"))
            .valign(gtk::Align::Center)
            .visible(false)
            .build();
        // The pin is announced as part of the row's label, not as a separate element.
        pin.update_state(&[gtk::accessible::State::Hidden(true)]);
        let hint = gtk::Label::builder().css_classes(["kb-hint"]).build();
        let remove = gtk::Button::builder()
            .icon_name("user-trash-symbolic")
            .css_classes(["kb-row-button", "flat"])
            .tooltip_text(format!("{} (Ctrl+⌫)", tr("Remove Item")))
            .focusable(false)
            .valign(gtk::Align::Center)
            .visible(false)
            .build();
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        content.append(&icon);
        content.append(&text);
        content.append(&pin);
        content.append(&hint);
        content.append(&remove);
        let widget = gtk::ListBoxRow::builder()
            .child(&content)
            .css_classes(["kb-row"])
            .focusable(false)
            .activatable(true)
            .build();
        let index = Rc::new(Cell::new(0));

        let context = gtk::GestureClick::builder().button(3).build();
        {
            let index = index.clone();
            let weak = Rc::downgrade(&self.inner);
            context.connect_pressed(move |gesture, _, _, _| {
                gesture.set_state(gtk::EventSequenceState::Claimed);
                let Some(inner) = weak.upgrade() else { return };
                let callback = inner.on_context.borrow().clone();
                if let Some(callback) = callback {
                    callback(index.get());
                }
            });
        }
        widget.add_controller(context);
        {
            let index = index.clone();
            let weak = Rc::downgrade(&self.inner);
            remove.connect_clicked(move |_| {
                let Some(inner) = weak.upgrade() else { return };
                let callback = inner.on_remove.borrow().clone();
                if let Some(callback) = callback {
                    callback(index.get());
                }
            });
        }
        Row {
            widget,
            icon,
            title,
            subtitle,
            pin,
            hint,
            remove,
            index,
            shown_icon: RefCell::new(None),
            thumbnail_id: RefCell::new(None),
        }
    }

    /// Select row `index` (clamped), update the `↵` hint and scroll it into view.
    pub fn select(&self, index: usize, reset_scroll: bool) {
        let inner = &self.inner;
        let count = inner.count.get();
        let index = if count == 0 { 0 } else { index.min(count - 1) };
        inner.selected.set(index);
        let show_hint = inner.style.get().show_hint;
        let rows = inner.rows.borrow();
        for (position, row) in rows.iter().take(count).enumerate() {
            let selected = position == index;
            if selected {
                row.widget.add_css_class("kb-selected");
            } else {
                row.widget.remove_css_class("kb-selected");
            }
            row.hint
                .set_text(if selected && show_hint { "↵" } else { "" });
        }
        drop(rows);
        if reset_scroll && index == 0 {
            inner.scroller.vadjustment().set_value(0.0);
        } else {
            self.scroll_to_selected();
        }
    }

    fn scroll_to_selected(&self) {
        let inner = &self.inner;
        let rows = inner.rows.borrow();
        let Some(row) = rows.get(inner.selected.get()) else {
            return;
        };
        let Some(bounds) = row.widget.compute_bounds(&inner.list) else {
            return;
        };
        let adjustment = inner.scroller.vadjustment();
        let top = f64::from(bounds.y()) - 4.0;
        let bottom = f64::from(bounds.y() + bounds.height()) + 4.0;
        if top < adjustment.value() {
            adjustment.set_value(top.max(0.0));
        } else if bottom > adjustment.value() + adjustment.page_size() {
            adjustment.set_value(bottom - adjustment.page_size());
        }
    }

    /// Provide a decoded thumbnail for clipboard image `id` and update rows showing it.
    pub fn set_thumbnail(&self, id: &str, texture: &gdk::Texture) {
        let inner = &self.inner;
        {
            let mut cache = inner.thumbnails.borrow_mut();
            // Keys are per attachment, so replaced or deleted images leave dead entries behind.
            if cache.len() >= MAX_CACHED_THUMBNAILS {
                cache.clear();
            }
            cache.insert(id.to_owned(), texture.clone());
        }
        let style = inner.style.get();
        let rows = inner.rows.borrow();
        for row in rows.iter().take(inner.count.get()) {
            if row.thumbnail_id.borrow().as_deref() == Some(id) {
                row.icon.set_paintable(Some(texture));
                row.icon.set_pixel_size(style.icon_px);
                row.icon.add_css_class("kb-thumbnail");
            }
        }
    }

    /// Forget thumbnails (history was cleared).
    pub fn clear_thumbnails(&self) {
        self.inner.thumbnails.borrow_mut().clear();
    }
}

impl Default for ResultList {
    fn default() -> Self {
        Self::new()
    }
}

fn set_text(label: &gtk::Label, text: &str) {
    if label.text() != text {
        label.set_text(text);
    }
}
