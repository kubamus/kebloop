//! The settings window: an `AdwNavigationSplitView` with a searchable sidebar of pages.
//!
//! Every page reads its state from the [`AppContext`] and writes through `ConfigStore::update`
//! (via `AppContext::update_settings`), so changes reach the launcher at once. Pages follow
//! changes made elsewhere through [`Page::refresh`], which the window calls on every
//! [`Change`](crate::context::Change).

mod about;
mod aliases;
mod appearance;
mod clipboard;
mod configuration;
mod extensions;
pub(crate) mod general;
pub mod logic;
mod profiles;
mod quicklinks;
mod recorder;
mod scripts;
mod search;
pub(crate) mod shortcuts;
mod snippets;
pub mod widgets;
mod window_management;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use self::logic::{Group, PageId};
use self::widgets::{Env, Page};
use crate::context::AppContext;
use crate::i18n::tr;

/// Show the settings window (creating it on first use), optionally on `page` (a command-line
/// page name such as `clipboard`; unknown names fall back to the current page). The launcher
/// is hidden.
pub fn open_settings(app: &adw::Application, ctx: &Rc<AppContext>, page: Option<&str>) {
    if let Some(launcher) = ctx.launcher() {
        launcher.hide();
    }
    let existing = ctx.settings_window.borrow().clone();
    let window = existing.unwrap_or_else(|| {
        let window = SettingsWindow::build(app, ctx);
        *ctx.settings_window.borrow_mut() = Some(window.clone());
        window
    });
    if let Some(page) = page.and_then(PageId::from_slug) {
        select_page(&window, page);
    }
    window.present();
}

/// Show `page` in an open settings window.
fn select_page(window: &gtk::Window, page: PageId) {
    let _ = WidgetExt::activate_action(window, "win.show-page", Some(&page.slug().to_variant()));
}

struct SettingsWindow;

impl SettingsWindow {
    fn build(app: &adw::Application, ctx: &Rc<AppContext>) -> gtk::Window {
        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title(tr("Kebloop Settings"))
            .default_width(1020)
            .default_height(720)
            .width_request(360)
            .height_request(300)
            .build();
        let toasts = adw::ToastOverlay::new();
        let plain: gtk::Window = window.clone().upcast();
        let env = Env::new(ctx.clone(), toasts.clone(), &plain);

        // Pages.
        let pages: Rc<HashMap<PageId, Page>> = Rc::new(
            PageId::ALL
                .into_iter()
                .map(|id| (id, build_page(id, &env)))
                .collect(),
        );
        let stack = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .hexpand(true)
            .vexpand(true)
            .build();
        for id in PageId::ALL {
            if let Some(page) = pages.get(&id) {
                stack.add_named(&page.widget, Some(id.slug()));
            }
        }

        // Sidebar.
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .css_classes(["navigation-sidebar"])
            .build();
        let rows: Rc<Vec<(PageId, gtk::ListBoxRow)>> = Rc::new(
            PageId::ALL
                .into_iter()
                .map(|id| (id, sidebar_row(id)))
                .collect(),
        );
        for (_, row) in rows.iter() {
            list.append(row);
        }
        list.set_header_func(|row, before| {
            let group = row_page(row).map(PageId::group);
            let previous = before.and_then(row_page).map(PageId::group);
            if group != previous
                && let Some(group) = group
            {
                row.set_header(Some(&section_header(group)));
                return;
            }
            row.set_header(gtk::Widget::NONE);
        });
        let search = gtk::SearchEntry::builder()
            .placeholder_text(tr("Search settings"))
            .hexpand(true)
            .build();
        search.update_property(&[gtk::accessible::Property::Label(&tr("Search settings"))]);
        let query = Rc::new(RefCell::new(String::new()));
        {
            let query = query.clone();
            list.set_filter_func(move |row| {
                row_page(row).is_none_or(|page| page.matches(&query.borrow()))
            });
        }
        let no_match = adw::StatusPage::builder()
            .icon_name("system-search-symbolic")
            .title(tr("No matching settings"))
            .visible(false)
            .vexpand(true)
            .build();
        no_match.add_css_class("compact");

        let content_title = adw::WindowTitle::new(&PageId::General.title(), "");
        let split = adw::NavigationSplitView::builder()
            .min_sidebar_width(230.0)
            .max_sidebar_width(300.0)
            .sidebar_width_fraction(0.28)
            .build();

        let show = {
            let stack = stack.clone();
            let list = list.clone();
            let rows = rows.clone();
            let content_title = content_title.clone();
            let split = split.clone();
            Rc::new(move |id: PageId| {
                stack.set_visible_child_name(id.slug());
                content_title.set_title(&id.title());
                if let Some((_, row)) = rows.iter().find(|(page, _)| *page == id)
                    && list.selected_row().as_ref() != Some(row)
                {
                    list.select_row(Some(row));
                }
                split.set_show_content(true);
            })
        };
        {
            let show = show.clone();
            list.connect_row_selected(move |_, row| {
                if let Some(page) = row.and_then(row_page) {
                    show(page);
                }
            });
        }
        {
            let list = list.clone();
            let query = query.clone();
            let rows = rows.clone();
            let no_match = no_match.clone();
            let scroller_list = list.clone();
            search.connect_search_changed(move |entry| {
                *query.borrow_mut() = entry.text().to_string();
                list.invalidate_filter();
                let any = rows.iter().any(|(page, _)| page.matches(&query.borrow()));
                no_match.set_visible(!any);
                scroller_list.set_visible(any);
            });
        }
        {
            // Return in the search field opens the first match.
            let rows = rows.clone();
            let query = query.clone();
            let show = show.clone();
            search.connect_activate(move |_| {
                if let Some((page, _)) = rows.iter().find(|(p, _)| p.matches(&query.borrow())) {
                    show(*page);
                }
            });
        }

        let scroller = gtk::ScrolledWindow::builder()
            .child(&list)
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();
        let sidebar_body = gtk::Box::new(gtk::Orientation::Vertical, 0);
        sidebar_body.append(&scroller);
        sidebar_body.append(&no_match);
        let search_box = gtk::Box::builder()
            .margin_start(8)
            .margin_end(8)
            .margin_bottom(8)
            .build();
        search_box.append(&search);
        let sidebar_toolbar = adw::ToolbarView::new();
        let sidebar_header = adw::HeaderBar::new();
        sidebar_header.set_title_widget(Some(&adw::WindowTitle::new(&tr("Settings"), "")));
        sidebar_toolbar.add_top_bar(&sidebar_header);
        sidebar_toolbar.add_top_bar(&search_box);
        sidebar_toolbar.set_content(Some(&sidebar_body));
        let sidebar_page = adw::NavigationPage::builder()
            .title(tr("Settings"))
            .tag("sidebar")
            .child(&sidebar_toolbar)
            .build();

        let content_toolbar = adw::ToolbarView::new();
        let content_header = adw::HeaderBar::new();
        content_header.set_title_widget(Some(&content_title));
        content_toolbar.add_top_bar(&content_header);
        content_toolbar.set_content(Some(&stack));
        let content_page = adw::NavigationPage::builder()
            .title(tr("Kebloop Settings"))
            .tag("content")
            .child(&content_toolbar)
            .build();
        split.set_sidebar(Some(&sidebar_page));
        split.set_content(Some(&content_page));
        toasts.set_child(Some(&split));
        window.set_content(Some(&toasts));

        // `win.show-page` (also used by the pages to link to each other).
        let action = gtk::gio::SimpleAction::new("show-page", Some(glib::VariantTy::STRING));
        {
            let show = show.clone();
            action.connect_activate(move |_, parameter| {
                if let Some(page) = parameter.and_then(|p| p.str()).and_then(PageId::from_slug) {
                    show(page);
                }
            });
        }
        window.add_action(&action);

        // Ctrl+F focuses the search; Escape closes.
        let shortcuts = gtk::ShortcutController::new();
        shortcuts.set_scope(gtk::ShortcutScope::Local);
        {
            let search = search.clone();
            shortcuts.add_shortcut(gtk::Shortcut::new(
                gtk::ShortcutTrigger::parse_string("<Control>f"),
                Some(gtk::CallbackAction::new(move |_, _| {
                    search.grab_focus();
                    glib::Propagation::Stop
                })),
            ));
        }
        window.add_controller(shortcuts);

        // Follow changes made elsewhere.
        let listener = {
            let pages = pages.clone();
            ctx.subscribe(Rc::new(move |_| {
                for page in pages.values() {
                    page.refresh();
                }
            }))
        };

        let closing_ctx = ctx.clone();
        window.connect_close_request(move |_| {
            closing_ctx.unsubscribe(listener);
            *closing_ctx.settings_window.borrow_mut() = None;
            glib::Propagation::Proceed
        });

        show(PageId::General);
        plain
    }
}

fn build_page(id: PageId, env: &Rc<Env>) -> Page {
    match id {
        PageId::General => general::build(env),
        PageId::Shortcuts => shortcuts::build(env),
        PageId::Search => search::build(env),
        PageId::Appearance => appearance::build(env),
        PageId::WindowManagement => window_management::build(env),
        PageId::Quicklinks => quicklinks::build(env),
        PageId::Aliases => aliases::build(env),
        PageId::Snippets => snippets::build(env),
        PageId::Scripts => scripts::build(env),
        PageId::Extensions => extensions::build(env),
        PageId::Profiles => profiles::build(env),
        PageId::Clipboard => clipboard::build(env),
        PageId::Configuration => configuration::build(env),
        PageId::Help => Page::new(&crate::help::HelpView::new(env.ctx.clone()).widget(), {
            // The help view refreshes itself through its own context subscription.
            || {}
        }),
        PageId::About => about::build(env),
    }
}

fn sidebar_row(id: PageId) -> gtk::ListBoxRow {
    let icon = gtk::Image::from_icon_name(id.icon());
    let label = gtk::Label::builder()
        .label(id.title())
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    let content = gtk::Box::builder()
        .spacing(12)
        .margin_top(8)
        .margin_bottom(8)
        .margin_start(6)
        .margin_end(6)
        .build();
    content.append(&icon);
    content.append(&label);
    let row = gtk::ListBoxRow::builder().child(&content).build();
    row.set_widget_name(id.slug());
    row.update_property(&[gtk::accessible::Property::Label(&id.title())]);
    row
}

fn row_page(row: &gtk::ListBoxRow) -> Option<PageId> {
    PageId::from_slug(row.widget_name().as_str())
}

fn section_header(group: Group) -> gtk::Widget {
    gtk::Label::builder()
        .label(tr(group.title_key()))
        .xalign(0.0)
        .margin_top(12)
        .margin_bottom(4)
        .margin_start(10)
        .css_classes(["caption-heading", "dim-label"])
        .build()
        .upcast()
}
