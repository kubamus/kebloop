//! Background services wired to the GTK main loop: application index, file index, clipboard
//! watcher, tray, global shortcuts and the power-off capability probe.

pub mod apps;
pub mod clipboard;
pub mod files;
pub mod rates;
pub mod shortcuts;
pub mod snippets;
pub mod system;
pub mod tray;

use std::rc::Rc;

use gtk::glib;
use kebloop_platform::files::FileIndexEvent;

use crate::context::{AppContext, Change};

/// Services listed in `KEBLOOP_DISABLE_SERVICES` (comma separated: `tray`, `shortcuts`,
/// `clipboard`) are not started. Meant for tests that must not touch the user's desktop session.
fn disabled(name: &str) -> bool {
    std::env::var("KEBLOOP_DISABLE_SERVICES")
        .is_ok_and(|list| list.split(',').any(|item| item.trim() == name))
}

/// Start every service. Each one degrades quietly when its backend is missing.
pub fn start(ctx: &Rc<AppContext>, file_events: async_channel::Receiver<FileIndexEvent>) {
    apps::init(ctx);
    files::init(ctx, file_events);
    if !disabled("clipboard") {
        clipboard::init(ctx);
    }
    clipboard::start_maintenance(ctx);
    if !disabled("tray") {
        tray::init(ctx);
    }
    if disabled("shortcuts") {
        *ctx.portal.borrow_mut() = crate::context::PortalState::Unavailable(
            "global shortcuts are disabled for this run".into(),
        );
    } else {
        shortcuts::init(ctx);
    }
    probe_power_off(ctx);
    probe_system_actions(ctx);
    rates::init(ctx);
    snippets::init(ctx);
    let probing = ctx.clone();
    glib::timeout_add_seconds_local(10 * 60, move || {
        probe_power_off(&probing);
        probe_system_actions(&probing);
        glib::ControlFlow::Continue
    });
    watch_settings(ctx);
}

/// Ask logind (off the main thread) whether this session may power off.
pub fn probe_power_off(ctx: &Rc<AppContext>) {
    let ctx = ctx.clone();
    crate::work::spawn(
        "kebloop-power-probe",
        system::can_power_off,
        move |allowed| ctx.set_shutdown_available(allowed),
    );
}

/// Probe (off the main thread) which system commands can work: logind, `wpctl`/`pactl`,
/// `gsettings`, the compositor's session exit.
pub fn probe_system_actions(ctx: &Rc<AppContext>) {
    let compositor = ctx.compositor.clone();
    let ctx = ctx.clone();
    crate::work::spawn(
        "kebloop-system-probe",
        move || system::probe(compositor.as_deref()),
        move |probe| ctx.set_system_probe(probe),
    );
}

/// Apply settings that have side effects outside the launcher window.
fn watch_settings(ctx: &Rc<AppContext>) {
    let applying = ctx.clone();
    let last_policy = std::cell::RefCell::new(files::IndexPolicy::of(ctx));
    ctx.subscribe(Rc::new(move |change| {
        if !matches!(change, Change::Settings | Change::Profile) {
            return;
        }
        applying.with_settings(|s| applying.styling.apply(&s.personalization));
        let policy = files::IndexPolicy::of(&applying);
        if *last_policy.borrow() != policy {
            *last_policy.borrow_mut() = policy;
            files::rebuild(&applying);
        }
    }));
    ctx.with_settings(|s| ctx.styling.apply(&s.personalization));
}
