//! The welcome walkthrough: launch at login, clipboard recording and how to bind the launcher
//! shortcut on this compositor. Skippable; completion is persisted so it shows once.

use std::rc::Rc;

use adw::prelude::*;
use kebloop_platform::shortcuts::{SnippetTarget, snippet_for};

use crate::context::{AppContext, PortalState};
use crate::i18n::{tr, trf};
use crate::settings::logic::{self, accelerator_label};
use crate::settings::widgets::{
    Env, SwitchControl, copy_button, error_label, group, mono_label, note, set_error,
};
use kebloop_services::l10n::FormatArg;

/// Whether the walkthrough should open on its own: a brand-new configuration that was never
/// dismissed.
pub fn should_show(ctx: &AppContext) -> bool {
    let config = ctx.config.borrow();
    config.was_created() && !config.global().onboarding_dismissed
}

/// Remember that the walkthrough was seen.
pub fn mark_dismissed(ctx: &AppContext) {
    let already = ctx.config.borrow().global().onboarding_dismissed;
    if already {
        return;
    }
    if let Err(error) = ctx
        .config
        .borrow_mut()
        .update_global(|g| g.onboarding_dismissed = true)
    {
        eprintln!("kebloop: could not save the onboarding state: {error}");
    }
}

/// The compositor snippet that binds the launcher shortcut, for the detected compositor.
pub fn launcher_snippet(ctx: &AppContext) -> Option<(SnippetTarget, String)> {
    let target =
        SnippetTarget::ALL[crate::settings::shortcuts::default_target(&ctx.session.compositor)];
    let program = logic::kebloop_program(
        &kebloop_platform::paths::Env::from_process().path_dirs(),
        std::env::current_exe().ok().as_deref(),
    );
    let accelerator = ctx.with_settings(|s| s.launcher_accelerator().to_string());
    snippet_for(target, &accelerator, &logic::toggle_command(&program))
        .ok()
        .map(|snippet| (target, snippet.text))
}

/// Show the welcome window.
pub fn open_onboarding(app: &adw::Application, ctx: &Rc<AppContext>) {
    if let Some(existing) = ctx.onboarding_window.borrow().as_ref() {
        existing.present();
        return;
    }
    if let Some(launcher) = ctx.launcher() {
        launcher.hide();
    }
    let window = adw::Window::builder()
        .application(app)
        .title(tr("Welcome to Kebloop"))
        .default_width(620)
        .default_height(700)
        .build();
    let toasts = adw::ToastOverlay::new();
    let plain: gtk::Window = window.clone().upcast();
    let env = Env::new(ctx.clone(), toasts.clone(), &plain);
    let navigation = adw::NavigationView::new();

    let finish: Rc<dyn Fn()> = {
        let (ctx, window) = (ctx.clone(), window.clone());
        Rc::new(move || {
            mark_dismissed(&ctx);
            window.close();
        })
    };

    let welcome = welcome_page(&env, &navigation, &finish);
    welcome.set_tag(Some("welcome"));
    navigation.add(&welcome);
    navigation.add(&shortcut_page(&env, &navigation, &finish));
    toasts.set_child(Some(&navigation));
    window.set_content(Some(&toasts));

    *ctx.onboarding_window.borrow_mut() = Some(plain);
    let closing = ctx.clone();
    window.connect_close_request(move |_| {
        mark_dismissed(&closing);
        *closing.onboarding_window.borrow_mut() = None;
        if let Some(launcher) = closing.launcher() {
            launcher.show();
        }
        gtk::glib::Propagation::Proceed
    });
    window.present();
}

fn page_shell(
    title: &str,
    body: &impl IsA<gtk::Widget>,
    skip: &Rc<dyn Fn()>,
) -> (adw::NavigationPage, gtk::Box) {
    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    let skip_button = gtk::Button::builder().label(tr("Skip for now")).build();
    {
        let skip = skip.clone();
        skip_button.connect_clicked(move |_| skip());
    }
    header.pack_start(&skip_button);
    toolbar.add_top_bar(&header);
    let scroller = gtk::ScrolledWindow::builder()
        .child(body)
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();
    let actions = gtk::Box::builder()
        .spacing(8)
        .halign(gtk::Align::End)
        .margin_top(8)
        .margin_bottom(16)
        .margin_start(24)
        .margin_end(24)
        .build();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&scroller);
    content.append(&actions);
    toolbar.set_content(Some(&content));
    let page = adw::NavigationPage::builder()
        .title(title)
        .child(&toolbar)
        .build();
    (page, actions)
}

fn hero(title: &str, subtitle: &str, icon: &str) -> gtk::Box {
    let hero = gtk::Box::new(gtk::Orientation::Vertical, 6);
    hero.set_margin_top(12);
    hero.append(&gtk::Image::builder().icon_name(icon).pixel_size(72).build());
    hero.append(
        &gtk::Label::builder()
            .label(title)
            .css_classes(["title-1"])
            .wrap(true)
            .justify(gtk::Justification::Center)
            .build(),
    );
    hero.append(
        &gtk::Label::builder()
            .label(subtitle)
            .css_classes(["dim-label"])
            .wrap(true)
            .justify(gtk::Justification::Center)
            .build(),
    );
    hero
}

fn welcome_page(
    env: &Rc<Env>,
    navigation: &adw::NavigationView,
    finish: &Rc<dyn Fn()>,
) -> adw::NavigationPage {
    let ctx = env.ctx.clone();
    let body = gtk::Box::new(gtk::Orientation::Vertical, 18);
    body.set_margin_start(24);
    body.set_margin_end(24);
    body.set_margin_top(6);
    body.append(&hero(
        &tr("Welcome to Kebloop"),
        &tr("Find apps, files, links, emoji, and saved text from one keyboard-first launcher."),
        crate::icons::APP_ICON_NAME,
    ));

    let basics = group(&tr("Choose the basics"), "");
    let login = {
        let ctx = ctx.clone();
        SwitchControl::new(
            env,
            &tr("Launch Kebloop when I log in"),
            "",
            crate::settings::general::autostart_entry(&ctx).is_enabled(),
            move |on| {
                let entry = crate::settings::general::autostart_entry(&ctx);
                let result = if on { entry.enable() } else { entry.disable() };
                result
                    .map_err(|error| format!("{} {error}", tr("Could not change launch at login.")))
            },
        )
    };
    basics.add(&login.row);
    let clipboard_on = ctx.clipboard.borrow().is_enabled();
    let clipboard = {
        let ctx = ctx.clone();
        SwitchControl::new(
            env,
            &tr("Record copied text in clipboard history"),
            "",
            clipboard_on,
            move |on| {
                let outcome = ctx
                    .clipboard
                    .borrow_mut()
                    .set_enabled(on)
                    .map_err(|e| tr(&e.to_string()));
                ctx.emit(crate::context::Change::Clipboard);
                outcome
            },
        )
    };
    basics.add(&clipboard.row);
    basics.add(&note(&tr(
        "Clipboard text stays on this computer. Kebloop skips password-marked, concealed, and oversized items. You can change this choice any time in Settings → Clipboard.",
    )));
    body.append(&basics);

    let try_group = group(&tr("Try these commands"), "");
    for line in [
        tr("clipboard — find and copy saved text"),
        tr("emoji smile — search and copy an emoji"),
        tr("help — browse all commands and shortcuts"),
        tr("Ctrl+K on a result — open its available actions"),
    ] {
        try_group.add(&note(&line));
    }
    body.append(&try_group);

    let (page, actions) = page_shell(&tr("Welcome to Kebloop"), &body, finish);
    let next = gtk::Button::builder()
        .label(tr("Continue"))
        .css_classes(["suggested-action", "pill"])
        .build();
    {
        let navigation = navigation.clone();
        next.connect_clicked(move |_| {
            navigation.push_by_tag("shortcut");
        });
    }
    actions.append(&next);
    page
}

fn shortcut_page(
    env: &Rc<Env>,
    navigation: &adw::NavigationView,
    finish: &Rc<dyn Fn()>,
) -> adw::NavigationPage {
    let ctx = env.ctx.clone();
    let body = gtk::Box::new(gtk::Orientation::Vertical, 18);
    body.set_margin_start(24);
    body.set_margin_end(24);
    body.set_margin_top(6);
    body.append(&hero(
        &tr("Open Kebloop from anywhere"),
        &trf(
            "Your launcher shortcut is %@.",
            &[FormatArg::Str(ctx.with_settings(|s| {
                accelerator_label(&s.launcher_accelerator())
            }))],
        ),
        "input-keyboard-symbolic",
    ));
    let how = group("", "");
    let error = error_label();
    match &*ctx.portal.borrow() {
        PortalState::Active(_) => {
            how.add(&note(&tr(
                "Your desktop supports global shortcuts. Kebloop registered the launcher shortcut through the desktop portal; your desktop may ask you to confirm it.",
            )));
        }
        _ => {
            how.add(&note(&trf(
                "Your desktop (%@) has no global shortcuts portal, so bind the launcher in its configuration. Paste this line:",
                &[FormatArg::Str(ctx.session.compositor.name().to_owned())],
            )));
            if let Some((target, text)) = launcher_snippet(&ctx) {
                how.add(&note(&trf(
                    "Add to %@",
                    &[FormatArg::Str(target.config_hint().to_owned())],
                )));
                let frame = gtk::Box::builder().css_classes(["card"]).spacing(6).build();
                let label = mono_label(&text);
                label.set_hexpand(true);
                label.set_margin_top(10);
                label.set_margin_bottom(10);
                label.set_margin_start(12);
                frame.append(&label);
                let copy = copy_button(env, {
                    let text = text.clone();
                    move || text.clone()
                });
                copy.set_valign(gtk::Align::Start);
                frame.append(&copy);
                how.add(&frame);
            } else {
                set_error(&error, Some(&tr("Could not prepare the key binding line.")));
            }
            how.add(&note(&tr(
                "Settings → Shortcuts has ready-to-paste lines for niri, Hyprland, Sway and i3, and for your own app and command shortcuts.",
            )));
        }
    }
    how.add(&error);
    body.append(&how);

    let (page, actions) = page_shell(&tr("Launcher shortcut"), &body, finish);
    page.set_tag(Some("shortcut"));
    let back = gtk::Button::builder().label(tr("Back")).build();
    {
        let navigation = navigation.clone();
        back.connect_clicked(move |_| {
            navigation.pop();
        });
    }
    let settings = gtk::Button::builder()
        .label(tr("Open Shortcut Settings"))
        .build();
    {
        let (ctx, finish) = (ctx.clone(), finish.clone());
        settings.connect_clicked(move |_| {
            finish();
            crate::settings::open_settings(&ctx.app, &ctx, Some("shortcuts"));
        });
    }
    let start = gtk::Button::builder()
        .label(tr("Get Started"))
        .css_classes(["suggested-action", "pill"])
        .build();
    {
        let finish = finish.clone();
        start.connect_clicked(move |_| finish());
    }
    actions.append(&back);
    actions.append(&settings);
    actions.append(&start);
    page
}
