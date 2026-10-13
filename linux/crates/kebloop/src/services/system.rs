//! System commands (lock, sleep, restart, log out, mute, appearance) and power off, run on
//! worker threads by the launcher.
//!
//! `KEBLOOP_DRY_RUN_SYSTEM_ACTIONS=<file>` replaces every side effect by a line appended to
//! `<file>` (read-only probes stay real), so the interface can be tested without locking,
//! suspending, rebooting or logging out the machine.

use std::io::Write as _;
use std::path::PathBuf;

use kebloop_platform::compositor::Compositor;
use kebloop_platform::power::PowerCapability;
use kebloop_platform::system_actions::{
    Availability, ColorScheme, LogindOp, SystemAction, SystemActions, SystemBackend, SystemBus,
};

/// Environment variable that switches system actions to a dry run.
pub const DRY_RUN_VARIABLE: &str = "KEBLOOP_DRY_RUN_SYSTEM_ACTIONS";

fn dry_run_log() -> Option<PathBuf> {
    std::env::var_os(DRY_RUN_VARIABLE)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// Records the side effects instead of performing them.
struct DryRun {
    log: PathBuf,
    real: SystemBus,
}

impl DryRun {
    fn record(&self, line: &str) -> Result<(), String> {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log)
            .map_err(|error| error.to_string())?;
        writeln!(file, "{line}").map_err(|error| error.to_string())
    }
}

impl SystemBackend for DryRun {
    fn logind(&self, op: LogindOp) -> Result<(), String> {
        self.record(&format!("logind {op:?}"))
    }

    fn logind_can(&self, op: LogindOp) -> Result<PowerCapability, String> {
        let _ = op;
        Ok(PowerCapability::Yes)
    }

    fn command_exists(&self, program: &str) -> bool {
        self.real.command_exists(program)
    }

    fn run(&self, program: &str, args: &[&str]) -> Result<String, String> {
        if args.first() == Some(&"get") {
            return self.real.run(program, args);
        }
        self.record(&format!("run {program} {}", args.join(" ")))?;
        Ok(String::new())
    }

    fn portal_color_scheme(&self) -> Result<ColorScheme, String> {
        self.real.portal_color_scheme()
    }
}

fn with_actions<T>(
    compositor: Option<&dyn Compositor>,
    f: impl FnOnce(&SystemActions<'_>) -> T,
) -> T {
    match dry_run_log() {
        // A dry run never asks the compositor to end the session.
        Some(log) => f(&SystemActions::new(
            &DryRun {
                log,
                real: SystemBus,
            },
            None,
        )),
        None => f(&SystemActions::new(&SystemBus, compositor)),
    }
}

/// Which system actions can work here (blocking: D-Bus and subprocess probes).
pub fn probe(compositor: Option<&dyn Compositor>) -> Vec<(SystemAction, Availability)> {
    with_actions(compositor, |actions| {
        SystemAction::ALL
            .into_iter()
            .map(|action| (action, actions.availability(action)))
            .collect()
    })
}

/// Perform `action` (blocking). Never reports success it cannot vouch for.
pub fn perform(action: SystemAction, compositor: Option<&dyn Compositor>) -> Result<(), String> {
    with_actions(compositor, |actions| {
        actions.perform(action).map_err(|error| error.to_string())
    })
}

/// Whether the session may power off (blocking).
pub fn can_power_off() -> bool {
    match dry_run_log() {
        Some(_) => true,
        None => kebloop_platform::power::can_power_off(),
    }
}

/// Power off (blocking).
pub fn power_off() -> Result<(), String> {
    match dry_run_log() {
        Some(log) => DryRun {
            log,
            real: SystemBus,
        }
        .record("power-off"),
        None => kebloop_platform::power::power_off().map_err(|error| error.to_string()),
    }
}
