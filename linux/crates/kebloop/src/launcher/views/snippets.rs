//! Snippet flow inside the panel: placeholder form (prefilled with the `{name:default}`
//! values), then a preview with Copy and Insert.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use gtk::prelude::*;
use jiff::tz::TimeZone;
use kebloop_core::Locale;
use kebloop_services::snippets::{RenderContext, Snippet};

use crate::i18n::tr;

type TextCallback = Rc<dyn Fn(String)>;
type PlainCallback = Rc<dyn Fn()>;

/// What `{date}`, `{time}` and `{clipboard}` are resolved with when the text is rendered.
#[derive(Clone)]
pub struct RenderEnv {
    /// Formats the date and the time.
    pub locale: Locale,
    /// The zone `{date}` and `{time}` are shown in.
    pub time_zone: TimeZone,
    /// The text on the clipboard when the snippet was opened.
    pub clipboard: String,
}

impl RenderEnv {
    /// Render `snippet` with `values` and the current time.
    fn render(&self, snippet: &Snippet, values: &BTreeMap<String, String>) -> Option<String> {
        let clipboard = self.clipboard.clone();
        let read = move || clipboard.clone();
        snippet
            .render_with(
                values,
                &RenderContext {
                    now: jiff::Timestamp::now(),
                    locale: self.locale.clone(),
                    time_zone: self.time_zone.clone(),
                    clipboard: &read,
                },
            )
            .ok()
    }
}

/// The page. Create it once; call [`SnippetPage::begin`] for each snippet.
#[derive(Clone)]
pub struct SnippetPage {
    inner: Rc<Inner>,
}

struct Inner {
    root: gtk::Box,
    title: gtk::Label,
    subtitle: gtk::Label,
    form: gtk::Box,
    preview_scroll: gtk::ScrolledWindow,
    preview: gtk::TextView,
    buttons: gtk::Box,
    snippet: RefCell<Option<Snippet>>,
    entries: RefCell<Vec<(String, gtk::Entry)>>,
    on_copy: RefCell<Option<TextCallback>>,
    on_insert: RefCell<Option<TextCallback>>,
    on_cancel: RefCell<Option<PlainCallback>>,
    env: RefCell<Option<RenderEnv>>,
}

impl SnippetPage {
    /// Build the empty page.
    pub fn new() -> Self {
        let title = gtk::Label::builder()
            .xalign(0.0)
            .css_classes(["kb-page-title"])
            .build();
        let subtitle = gtk::Label::builder()
            .xalign(0.0)
            .wrap(true)
            .css_classes(["kb-page-subtitle"])
            .build();
        let form = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let preview = gtk::TextView::builder()
            .editable(false)
            .cursor_visible(false)
            .wrap_mode(gtk::WrapMode::WordChar)
            .css_classes(["kb-preview"])
            .focusable(false)
            .build();
        let preview_scroll = gtk::ScrolledWindow::builder()
            .child(&preview)
            .min_content_height(120)
            .max_content_height(220)
            .propagate_natural_height(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .visible(false)
            .build();
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        buttons.set_halign(gtk::Align::End);
        let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
        root.add_css_class("kb-page");
        root.append(&title);
        root.append(&subtitle);
        root.append(&form);
        root.append(&preview_scroll);
        root.append(&buttons);
        Self {
            inner: Rc::new(Inner {
                root,
                title,
                subtitle,
                form,
                preview_scroll,
                preview,
                buttons,
                snippet: RefCell::new(None),
                entries: RefCell::new(Vec::new()),
                on_copy: RefCell::new(None),
                on_insert: RefCell::new(None),
                on_cancel: RefCell::new(None),
                env: RefCell::new(None),
            }),
        }
    }

    /// The widget to put into the panel.
    pub fn widget(&self) -> &gtk::Box {
        &self.inner.root
    }

    /// Wire the outcomes: copy the text, insert the text, or give up.
    pub fn connect(
        &self,
        on_copy: impl Fn(String) + 'static,
        on_insert: impl Fn(String) + 'static,
        on_cancel: impl Fn() + 'static,
    ) {
        *self.inner.on_copy.borrow_mut() = Some(Rc::new(on_copy));
        *self.inner.on_insert.borrow_mut() = Some(Rc::new(on_insert));
        *self.inner.on_cancel.borrow_mut() = Some(Rc::new(on_cancel));
    }

    /// Start the flow for `snippet`; `env` resolves the dynamic values.
    pub fn begin(&self, snippet: &Snippet, env: RenderEnv) {
        let inner = &self.inner;
        *inner.snippet.borrow_mut() = Some(snippet.clone());
        *inner.env.borrow_mut() = Some(env);
        inner.title.set_text(&snippet.name);
        let placeholders = snippet.placeholders();
        if placeholders.is_empty() {
            self.continue_to_preview();
            return;
        }
        self.show_form(&placeholders, &snippet.placeholder_defaults());
    }

    fn clear_buttons(&self) {
        while let Some(child) = self.inner.buttons.first_child() {
            self.inner.buttons.remove(&child);
        }
    }

    fn show_form(&self, placeholders: &[String], defaults: &BTreeMap<String, String>) {
        let inner = &self.inner;
        inner.preview_scroll.set_visible(false);
        inner.form.set_visible(true);
        inner
            .subtitle
            .set_text(&tr("Fill in the values, then press Return."));
        while let Some(child) = inner.form.first_child() {
            inner.form.remove(&child);
        }
        inner.entries.borrow_mut().clear();
        let count = placeholders.len();
        for (position, name) in placeholders.iter().enumerate() {
            let label = gtk::Label::builder()
                .label(format!("{} {name}", tr("Value for")))
                .xalign(0.0)
                .css_classes(["dim-label"])
                .build();
            let entry = gtk::Entry::builder()
                .hexpand(true)
                .text(defaults.get(name).map_or("", String::as_str))
                .build();
            entry.update_property(&[gtk::accessible::Property::Label(name)]);
            let weak = Rc::downgrade(&self.inner);
            entry.connect_activate(move |_| {
                let Some(inner) = weak.upgrade() else { return };
                let page = SnippetPage { inner };
                if position + 1 < count {
                    let next = page
                        .inner
                        .entries
                        .borrow()
                        .get(position + 1)
                        .map(|(_, e)| e.clone());
                    if let Some(next) = next {
                        next.grab_focus();
                    }
                } else {
                    page.continue_to_preview();
                }
            });
            inner.form.append(&label);
            inner.form.append(&entry);
            inner.entries.borrow_mut().push((name.clone(), entry));
        }
        self.clear_buttons();
        let cancel = gtk::Button::with_label(&tr("Cancel"));
        let proceed = gtk::Button::with_label(&tr("Continue"));
        proceed.add_css_class("suggested-action");
        {
            let weak = Rc::downgrade(&self.inner);
            cancel.connect_clicked(move |_| {
                if let Some(inner) = weak.upgrade() {
                    SnippetPage { inner }.cancel();
                }
            });
            let weak = Rc::downgrade(&self.inner);
            proceed.connect_clicked(move |_| {
                if let Some(inner) = weak.upgrade() {
                    SnippetPage { inner }.continue_to_preview();
                }
            });
        }
        inner.buttons.append(&cancel);
        inner.buttons.append(&proceed);
        let first = inner.entries.borrow().first().map(|(_, e)| e.clone());
        if let Some(first) = first {
            first.grab_focus();
        }
    }

    fn continue_to_preview(&self) {
        let values: BTreeMap<String, String> = self
            .inner
            .entries
            .borrow()
            .iter()
            .map(|(name, entry)| (name.clone(), entry.text().to_string()))
            .collect();
        let rendered = {
            let snippet = self.inner.snippet.borrow();
            let env = self.inner.env.borrow();
            match (snippet.as_ref(), env.as_ref()) {
                (Some(snippet), Some(env)) => env.render(snippet, &values),
                _ => None,
            }
        };
        match rendered {
            Some(text) => self.show_preview(&text),
            // Nothing can be shown for this text (Swift returns to the launcher too).
            None => self.cancel(),
        }
    }

    fn show_preview(&self, text: &str) {
        let inner = &self.inner;
        inner.form.set_visible(false);
        inner.preview_scroll.set_visible(true);
        inner.preview.buffer().set_text(text);
        inner.subtitle.set_text(&tr(
            "Review the text, then copy it or insert it into the previous window.",
        ));
        self.clear_buttons();
        let cancel = gtk::Button::with_label(&tr("Cancel"));
        let copy = gtk::Button::with_label(&tr("Copy"));
        copy.add_css_class("suggested-action");
        let text = text.to_owned();
        {
            let weak = Rc::downgrade(&self.inner);
            cancel.connect_clicked(move |_| {
                if let Some(inner) = weak.upgrade() {
                    SnippetPage { inner }.cancel();
                }
            });
            let weak = Rc::downgrade(&self.inner);
            let copied = text.clone();
            copy.connect_clicked(move |_| {
                let Some(inner) = weak.upgrade() else { return };
                let callback = inner.on_copy.borrow().clone();
                if let Some(callback) = callback {
                    callback(copied.clone());
                }
            });
        }
        inner.buttons.append(&cancel);
        let insert = gtk::Button::with_label(&tr("Insert"));
        {
            let weak = Rc::downgrade(&self.inner);
            let inserted = text.clone();
            insert.connect_clicked(move |_| {
                let Some(inner) = weak.upgrade() else { return };
                let callback = inner.on_insert.borrow().clone();
                if let Some(callback) = callback {
                    callback(inserted.clone());
                }
            });
        }
        inner.buttons.append(&insert);
        inner.buttons.append(&copy);
        copy.grab_focus();
    }

    fn cancel(&self) {
        let callback = self.inner.on_cancel.borrow().clone();
        if let Some(callback) = callback {
            callback();
        }
    }
}

impl Default for SnippetPage {
    fn default() -> Self {
        Self::new()
    }
}
