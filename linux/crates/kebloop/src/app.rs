//! Application lifecycle: the single-instance `AdwApplication`, startup, command-line
//! dispatch and shutdown.

use std::cell::RefCell;
use std::ffi::OsString;
use std::process::ExitCode;
use std::rc::Rc;
use std::time::SystemTime;

use adw::prelude::*;
use gtk::{gio, glib};
use kebloop_platform::files::FileIndexer;
use kebloop_platform::paths::XdgDirs;
use kebloop_services::clipboard::{ClipboardPaths, ClipboardStore};
use kebloop_services::config::{ConfigPaths, ConfigStore};
use kebloop_services::l10n::Localizer;

use crate::cli::{Cli, Request, app_id};
use crate::context::{AppContext, ClipboardStatus};
use crate::i18n::{tr, trf};
use crate::launcher::Launcher;
use crate::style::Styling;
use crate::{i18n, icons, services};
use kebloop_services::l10n::FormatArg;

thread_local! {
    static CONTEXT: RefCell<Option<Rc<AppContext>>> = const { RefCell::new(None) };
}

/// The context of the running primary instance (main thread only).
pub fn context() -> Option<Rc<AppContext>> {
    CONTEXT.with(|slot| slot.borrow().clone())
}

/// Run the primary instance until it quits.
pub fn run(_cli: Cli, args: &[OsString]) -> ExitCode {
    let app = adw::Application::builder()
        .application_id(app_id())
        .flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();
    app.connect_startup(startup);
    app.connect_shutdown(|_| {
        // Whatever ended the main loop (menu, signal, session end): nothing may stay behind.
        if let Some(ctx) = context() {
            shut_down_services(&ctx);
        }
    });
    app.connect_command_line(command_line);
    let args: Vec<String> = args
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let status = app.run_with_args(&args);
    ExitCode::from(status.get())
}

fn startup(app: &adw::Application) {
    // Menu-bar style app: no window means keep running.
    std::mem::forget(app.hold());
    match build_context(app) {
        Ok(ctx) => {
            CONTEXT.with(|slot| *slot.borrow_mut() = Some(ctx));
        }
        Err(error) => {
            eprintln!("kebloop: {error:#}");
            app.quit();
        }
    }
}

fn build_context(app: &adw::Application) -> anyhow::Result<Rc<AppContext>> {
    let xdg = XdgDirs::from_process_env();
    let localizer = Localizer::from_env(&|key| std::env::var(key).ok());
    let localizer = localizer
        .clone()
        .with_overlay_dir(&xdg.app_config_dir())
        .unwrap_or(localizer);
    i18n::init(localizer.clone());
    gtk::Window::set_default_icon_name(icons::APP_ICON_NAME);
    icons::register_development_icons();

    let home = xdg.home.to_string_lossy().into_owned();
    // A store that cannot be opened must not keep the launcher from starting: fall back to
    // defaults / no history and tell the user (Help window and a desktop notification).
    let mut notices: Vec<String> = Vec::new();
    let config_paths = ConfigPaths::in_dir(&xdg.app_config_dir());
    let config = match ConfigStore::open(config_paths.clone(), &home) {
        Ok(config) => config,
        Err(error) => {
            notices.push(trf(
                "Could not open the configuration (%@). Kebloop started with default settings and will not save changes in this session.",
                &[FormatArg::Str(error.to_string())],
            ));
            ConfigStore::in_memory(config_paths, &home, error.to_string())
        }
    };
    if let Some(aside) = config.recovered_corrupt_file() {
        notices.push(trf(
            "The configuration file was unreadable and was moved to %@. Kebloop started with default settings.",
            &[FormatArg::Str(aside.display().to_string())],
        ));
    }
    let clipboard_paths = ClipboardPaths::in_dir(&xdg.app_data_dir());
    let clipboard = match ClipboardStore::open(clipboard_paths.clone(), SystemTime::now()) {
        Ok(store) => store,
        Err(error) => {
            notices.push(trf(
                "Could not open the clipboard history (%@). Clipboard history is turned off in this session.",
                &[FormatArg::Str(error.to_string())],
            ));
            ClipboardStore::unavailable(clipboard_paths, error.to_string())
        }
    };
    if let Some(aside) = clipboard.recovered_corrupt_file() {
        notices.push(trf(
            "The clipboard history file was unreadable and was moved to %@.",
            &[FormatArg::Str(aside.display().to_string())],
        ));
    }

    let styling = Styling::install();
    let (file_indexer, file_events) = FileIndexer::new();
    let ctx = Rc::new(AppContext::new(
        app.clone(),
        xdg,
        localizer,
        config,
        clipboard,
        file_indexer,
        styling,
    ));
    ctx.with_settings(|s| ctx.styling.apply(&s.personalization));
    if let Some(reason) = ctx.clipboard.borrow().unavailable_reason() {
        *ctx.clipboard_status.borrow_mut() = ClipboardStatus::HistoryDisabled(reason.to_owned());
    }
    announce(&ctx, notices);
    install_signal_handlers(&ctx);
    let first_run = crate::onboarding::should_show(&ctx);
    if first_run {
        // Clipboard recording is opt-in: it stays off until the walkthrough turns it on.
        if let Err(error) = ctx.clipboard.borrow_mut().set_enabled(false) {
            eprintln!("kebloop: could not turn clipboard recording off: {error}");
        }
    } else {
        crate::onboarding::mark_dismissed(&ctx);
    }
    let launcher = Launcher::new(&ctx);
    *ctx.launcher.borrow_mut() = Some(launcher);
    services::start(&ctx, file_events);
    if first_run {
        let welcoming = ctx.clone();
        glib::idle_add_local_once(move || {
            crate::onboarding::open_onboarding(&welcoming.app, &welcoming);
        });
    }
    Ok(ctx)
}

/// Remember startup problems for the Help window and show them as desktop notifications.
fn announce(ctx: &AppContext, notices: Vec<String>) {
    for (index, text) in notices.into_iter().enumerate() {
        let notification = gio::Notification::new(&tr("Kebloop"));
        notification.set_body(Some(&text));
        ctx.app
            .send_notification(Some(&format!("startup-notice-{index}")), &notification);
        ctx.add_notice(text);
    }
}

/// Quit cleanly (flushing saves, stopping scripts) on SIGTERM / SIGINT, e.g. at logout.
fn install_signal_handlers(ctx: &Rc<AppContext>) {
    use async_signal::{Signal, Signals};
    use futures_lite::StreamExt;
    let Ok(mut signals) = Signals::new([Signal::Term, Signal::Int]) else {
        return;
    };
    let ctx = Rc::downgrade(ctx);
    glib::spawn_future_local(async move {
        if signals.next().await.is_some()
            && let Some(ctx) = ctx.upgrade()
        {
            quit(&ctx);
        }
    });
}

fn command_line(
    app: &adw::Application,
    command_line: &gio::ApplicationCommandLine,
) -> glib::ExitCode {
    let args = command_line.arguments();
    let cli = match Cli::parse_from_os(args) {
        Ok(cli) => cli,
        Err(error) => {
            eprintln!("{error}");
            return glib::ExitCode::from(2);
        }
    };
    let Some(ctx) = context() else {
        return glib::ExitCode::FAILURE;
    };
    let _ = app;
    dispatch(&ctx, cli.request());
    glib::ExitCode::SUCCESS
}

/// Carry out a request from the command line, tray or a global shortcut.
pub fn dispatch(ctx: &Rc<AppContext>, request: Request) {
    let Some(launcher): Option<Launcher> = ctx.launcher() else {
        return;
    };
    match request {
        Request::Background => {}
        Request::Show => launcher.show(),
        Request::Toggle => launcher.toggle(),
        Request::Hide => {
            launcher.hide();
            hide_secondary_windows(ctx);
        }
        Request::Settings(page) => {
            crate::settings::open_settings(&ctx.app, ctx, page.as_deref());
        }
        Request::Command(id) => launcher.execute_command(&id),
        Request::Launch(id) => launcher.launch_application_id(&id),
        Request::Quit => quit(ctx),
    }
}

fn hide_secondary_windows(ctx: &AppContext) {
    for slot in [
        &ctx.settings_window,
        &ctx.help_window,
        &ctx.onboarding_window,
    ] {
        if let Some(window) = slot.borrow().as_ref() {
            window.set_visible(false);
        }
    }
}

/// How long running scripts get to react to `SIGTERM` before they are killed on quit.
const QUIT_GRACE: std::time::Duration = std::time::Duration::from_millis(300);

/// Stop everything that outlives the main loop: cancel background runs, terminate their process
/// groups synchronously (`SIGTERM`, short wait, `SIGKILL`) and write pending saves.
fn shut_down_services(ctx: &AppContext) {
    ctx.cancel_all_runs();
    ctx.file_indexer.cancel();
    kebloop_services::automation::terminate_all_groups(QUIT_GRACE);
    ctx.flush_persistence();
}

/// Cancel background runs, stop their subprocesses, save and leave the main loop.
pub fn quit(ctx: &AppContext) {
    shut_down_services(ctx);
    ctx.app.quit();
}
