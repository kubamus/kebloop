//! About: version, links, license note and where Kebloop keeps its files.

use std::rc::Rc;

use adw::prelude::*;
use gtk::gio;

use super::logic::PageId;
use super::widgets::{Env, Page, action_row, group, icon_button, note, preferences_page};
use crate::i18n::{tr, trf};
use kebloop_services::l10n::FormatArg;

/// Project home page.
pub const REPOSITORY_URL: &str = "https://github.com/kubamus/kebloop";

/// The application version (from `Cargo.toml`).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

fn open_uri(env: &Rc<Env>, uri: &str) {
    if let Err(error) = gio::AppInfo::launch_default_for_uri(uri, gio::AppLaunchContext::NONE) {
        env.toast(&format!("{} {error}", tr("Could not open the link.")));
    }
}

/// Build the page.
pub fn build(env: &Rc<Env>) -> Page {
    let ctx = &env.ctx;
    let page = preferences_page();
    page.set_title(&PageId::About.title());
    page.set_description(&PageId::About.subtitle());

    let header = group("", "");
    let icon = gtk::Image::builder()
        .icon_name(crate::icons::APP_ICON_NAME)
        .pixel_size(96)
        .margin_top(12)
        .build();
    let name = gtk::Label::builder()
        .label("Kebloop")
        .css_classes(["title-1"])
        .build();
    let version = gtk::Label::builder()
        .label(trf("Version %@", &[FormatArg::Str(VERSION.to_owned())]))
        .css_classes(["dim-label"])
        .build();
    let tagline = gtk::Label::builder()
        .label(tr(
            "Find apps, files, links, emoji, and saved text from one keyboard-first launcher.",
        ))
        .wrap(true)
        .justify(gtk::Justification::Center)
        .margin_bottom(12)
        .build();
    let hero = gtk::Box::new(gtk::Orientation::Vertical, 6);
    hero.append(&icon);
    hero.append(&name);
    hero.append(&version);
    hero.append(&tagline);
    header.add(&hero);
    page.add(&header);

    let links = group(&tr("Links"), "");
    for (title, uri) in [
        (
            tr("Project website and source code"),
            REPOSITORY_URL.to_owned(),
        ),
        (tr("Report a problem"), format!("{REPOSITORY_URL}/issues")),
    ] {
        let row = action_row(&title, &uri);
        row.set_activatable(true);
        row.add_suffix(&gtk::Image::from_icon_name("adw-external-link-symbolic"));
        let (env, uri) = (env.clone(), uri.clone());
        row.connect_activated(move |_| open_uri(&env, &uri));
        links.add(&row);
    }
    page.add(&links);

    let system = group(&tr("This session"), "");
    let session = &ctx.session;
    let display_server = format!("{:?}", session.display_server);
    system.add(&action_row(
        &tr("Desktop"),
        &format!("{} · {display_server}", session.compositor.name()),
    ));
    for (title, path) in [
        (tr("Configuration folder"), ctx.xdg.app_config_dir()),
        (tr("Data folder"), ctx.xdg.app_data_dir()),
    ] {
        let row = action_row(&title, &path.to_string_lossy());
        row.set_subtitle_selectable(true);
        let open = icon_button("folder-open-symbolic", &tr("Open"));
        let (env, uri) = (env.clone(), gio::File::for_path(&path).uri().to_string());
        open.connect_clicked(move |_| open_uri(&env, &uri));
        row.add_suffix(&open);
        system.add(&row);
    }
    page.add(&system);

    let legal = group(&tr("License"), "");
    legal.add(&note(&tr(
        "Kebloop is free software released under the MIT License and is provided as is, without warranty.",
    )));
    page.add(&legal);
    Page::fixed(&page)
}
