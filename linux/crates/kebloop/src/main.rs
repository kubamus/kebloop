//! Kebloop for Linux: GTK4 + libadwaita + layer-shell launcher.
//!
//! The first process becomes the primary instance (single-instance `GApplication`). Later
//! invocations (`kebloop toggle`, ...) forward their command line over D-Bus without initialising
//! GTK, so a compositor key binding that runs `kebloop toggle` reacts immediately.

mod app;
mod cli;
mod context;
mod direct_paste;
mod help;
mod i18n;
mod icons;
mod launcher;
mod onboarding;
mod platform;
mod remote;
mod services;
mod settings;
mod style;
mod work;

use std::ffi::OsString;
use std::process::ExitCode;

use clap::Parser as _;

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().collect();
    let parsed = match cli::Cli::try_parse_from(&args) {
        Ok(parsed) => parsed,
        Err(error) => {
            // Prints help/version to stdout and usage errors to stderr.
            let _ = error.print();
            return if error.use_stderr() {
                ExitCode::from(2)
            } else {
                ExitCode::SUCCESS
            };
        }
    };
    if let Some(code) = remote::forward_if_running(&args) {
        return code;
    }
    app::run(parsed, &args)
}
