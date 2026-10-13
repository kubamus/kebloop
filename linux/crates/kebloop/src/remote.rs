//! Fast path for secondary invocations: if a primary instance owns the bus name, forward the
//! command line to it with a plain `GApplication` (no GTK, no display connection) and exit.

use std::ffi::OsString;
use std::process::ExitCode;

use gtk::gio;
use gtk::gio::prelude::*;

/// Returns `Some(exit code)` when a running instance handled the command line.
pub fn forward_if_running(args: &[OsString]) -> Option<ExitCode> {
    let id = crate::cli::app_id();
    if !name_has_owner(&id) {
        return None;
    }
    let app = gio::Application::new(Some(&id), gio::ApplicationFlags::HANDLES_COMMAND_LINE);
    if app.register(None::<&gio::Cancellable>).is_err() || !app.is_remote() {
        return None;
    }
    let args: Vec<String> = args
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let status = app.run_with_args(&args);
    Some(ExitCode::from(status.get()))
}

fn name_has_owner(name: &str) -> bool {
    let Ok(connection) = gio::bus_get_sync(gio::BusType::Session, None::<&gio::Cancellable>) else {
        return false;
    };
    let reply = connection.call_sync(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "NameHasOwner",
        Some(&(name,).to_variant()),
        None,
        gio::DBusCallFlags::NONE,
        500,
        None::<&gio::Cancellable>,
    );
    reply
        .ok()
        .and_then(|value| value.child_value(0).get::<bool>())
        .unwrap_or(false)
}
