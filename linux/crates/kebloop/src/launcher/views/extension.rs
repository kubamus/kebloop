//! Extension browser inside the panel: results list on top, detail and actions below. The
//! v1 protocol runs on worker threads; errors are shown inline.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use kebloop_services::automation::ExtensionResult;

use crate::i18n::tr;
use crate::launcher::model::{IconSpec, ResultItem, Target};
use crate::launcher::results::ResultList;

type ActionCallback = Rc<dyn Fn(String, String)>;

/// The page: owns its own result list.
#[derive(Clone)]
pub struct ExtensionPage {
    inner: Rc<Inner>,
}

struct Inner {
    root: gtk::Box,
    list: ResultList,
    status: gtk::Label,
    spinner: gtk::Spinner,
    detail: gtk::Box,
    detail_title: gtk::Label,
    detail_text: gtk::Label,
    actions: gtk::Box,
    results: RefCell<Vec<ExtensionResult>>,
    on_action: RefCell<Option<ActionCallback>>,
}

impl ExtensionPage {
    /// Build the page.
    pub fn new() -> Self {
        let list = ResultList::new();
        list.set_max_height(190);
        let status = gtk::Label::builder()
            .xalign(0.0)
            .wrap(true)
            .selectable(true)
            .hexpand(true)
            .visible(false)
            .build();
        let spinner = gtk::Spinner::builder().visible(false).build();
        let status_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        status_row.set_margin_start(16);
        status_row.set_margin_end(16);
        status_row.set_margin_top(4);
        status_row.append(&spinner);
        status_row.append(&status);
        let detail_title = gtk::Label::builder()
            .xalign(0.0)
            .css_classes(["kb-page-title"])
            .wrap(true)
            .build();
        let detail_text = gtk::Label::builder()
            .xalign(0.0)
            .yalign(0.0)
            .wrap(true)
            .selectable(true)
            .build();
        let detail_scroll = gtk::ScrolledWindow::builder()
            .child(&detail_text)
            .min_content_height(40)
            .max_content_height(120)
            .propagate_natural_height(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let detail = gtk::Box::new(gtk::Orientation::Vertical, 6);
        detail.add_css_class("kb-detail");
        detail.set_margin_start(12);
        detail.set_margin_end(12);
        detail.set_margin_bottom(10);
        detail.set_visible(false);
        detail.append(&detail_title);
        detail.append(&detail_scroll);
        detail.append(&actions);
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.append(list.widget());
        root.append(&status_row);
        root.append(&detail);
        Self {
            inner: Rc::new(Inner {
                root,
                list,
                status,
                spinner,
                detail,
                detail_title,
                detail_text,
                actions,
                results: RefCell::new(Vec::new()),
                on_action: RefCell::new(None),
            }),
        }
    }

    /// The widget for the panel's stack.
    pub fn widget(&self) -> &gtk::Box {
        &self.inner.root
    }

    /// The result list (keyboard navigation and mouse hooks are wired by the launcher).
    pub fn list(&self) -> &ResultList {
        &self.inner.list
    }

    /// Called with (result id, action id) when an action button is pressed.
    pub fn connect_action(&self, callback: impl Fn(String, String) + 'static) {
        *self.inner.on_action.borrow_mut() = Some(Rc::new(callback));
    }

    /// Forget everything shown.
    pub fn clear(&self) {
        self.inner.results.borrow_mut().clear();
        self.inner.list.set_items(
            &[],
            0,
            &tr("Nothing to show"),
            false,
            &std::collections::HashSet::new(),
            &|_| {},
        );
        self.inner.detail.set_visible(false);
        self.show_status(None, false);
    }

    /// Show a progress or error line. `busy` shows the spinner.
    pub fn show_status(&self, text: Option<&str>, busy: bool) {
        let inner = &self.inner;
        inner.spinner.set_visible(busy);
        inner.spinner.set_spinning(busy);
        match text {
            Some(text) => {
                inner.status.set_text(text);
                inner.status.set_visible(true);
            }
            None => inner.status.set_visible(false),
        }
    }

    /// Replace the results.
    pub fn set_results(&self, results: &[ExtensionResult], selected: Option<&str>) {
        let inner = &self.inner;
        *inner.results.borrow_mut() = results.to_vec();
        let items: Vec<ResultItem> = results
            .iter()
            .map(|r| ResultItem {
                id: format!("extension-result:{}", r.id),
                target: Target::ExtensionResult {
                    result_id: r.id.clone(),
                },
                title: r.title.clone(),
                subtitle: r.subtitle.clone().unwrap_or_default(),
                icon: IconSpec::Symbolic("application-x-addon-symbolic"),
                action: r
                    .actions
                    .first()
                    .map(|a| a.title.clone())
                    .unwrap_or_default(),
            })
            .collect();
        let position = selected
            .and_then(|id| results.iter().position(|r| r.id == id))
            .unwrap_or(0);
        inner.list.set_items(
            &items,
            position,
            &tr("No results"),
            false,
            &std::collections::HashSet::new(),
            &|_| {},
        );
        self.show_detail();
    }

    /// The selected result.
    pub fn selected_result(&self) -> Option<ExtensionResult> {
        self.inner
            .results
            .borrow()
            .get(self.inner.list.selected())
            .cloned()
    }

    /// Select a row and refresh the detail pane.
    pub fn select(&self, index: usize) {
        self.inner.list.select(index, false);
        self.show_detail();
    }

    /// Move the selection by `delta` rows.
    pub fn move_selection(&self, delta: isize) {
        let count = self.inner.results.borrow().len();
        if count == 0 {
            return;
        }
        let next = self
            .inner
            .list
            .selected()
            .saturating_add_signed(delta)
            .min(count - 1);
        self.select(next);
    }

    fn show_detail(&self) {
        let inner = &self.inner;
        while let Some(child) = inner.actions.first_child() {
            inner.actions.remove(&child);
        }
        let Some(result) = self.selected_result() else {
            inner.detail.set_visible(false);
            return;
        };
        inner.detail.set_visible(true);
        inner.detail_title.set_text(&result.title);
        let text = result
            .detail
            .clone()
            .or_else(|| result.subtitle.clone())
            .unwrap_or_default();
        inner.detail_text.set_text(&text);
        inner.detail_text.set_visible(!text.is_empty());
        for action in &result.actions {
            let button = gtk::Button::with_label(&action.title);
            button.set_focusable(false);
            let weak = Rc::downgrade(&self.inner);
            let result_id = result.id.clone();
            let action_id = action.id.clone();
            button.connect_clicked(move |_| {
                let Some(inner) = weak.upgrade() else { return };
                let callback = inner.on_action.borrow().clone();
                if let Some(callback) = callback {
                    callback(result_id.clone(), action_id.clone());
                }
            });
            inner.actions.append(&button);
        }
    }
}

impl Default for ExtensionPage {
    fn default() -> Self {
        Self::new()
    }
}
