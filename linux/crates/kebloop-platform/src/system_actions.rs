//! System commands (port of `SystemActionService`): lock, sleep, restart, log
//! out, toggle mute and toggle appearance.
//!
//! Everything that touches the machine goes through [`SystemBackend`], so tests
//! use a mock and never lock, suspend, reboot or log out. Each action returns
//! an explicit error instead of claiming success; [`SystemActions::availability`]
//! probes capabilities so the UI can hide commands that cannot work.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::compositor::Compositor;
use crate::power::PowerCapability;

/// Same identifiers as the Swift `LauncherCommand` raw values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SystemAction {
    Lock,
    Sleep,
    Restart,
    LogOut,
    ToggleMute,
    ToggleAppearance,
}

impl SystemAction {
    pub const ALL: [Self; 6] = [
        Self::Lock,
        Self::Sleep,
        Self::Restart,
        Self::LogOut,
        Self::ToggleMute,
        Self::ToggleAppearance,
    ];

    /// Persisted command id (`lock`, `sleep`, `restart`, `logOut`, `toggleMute`, `toggleAppearance`).
    pub fn id(self) -> &'static str {
        match self {
            Self::Lock => "lock",
            Self::Sleep => "sleep",
            Self::Restart => "restart",
            Self::LogOut => "logOut",
            Self::ToggleMute => "toggleMute",
            Self::ToggleAppearance => "toggleAppearance",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.id() == id)
    }

    /// Restart and Log Out end the session: the UI shows its own confirmation first.
    pub fn needs_confirmation(self) -> bool {
        matches!(self, Self::Restart | Self::LogOut)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Availability {
    Available,
    Unavailable(String),
}

impl Availability {
    pub fn is_available(&self) -> bool {
        matches!(self, Self::Available)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SystemActionError {
    #[error("{0}")]
    Unavailable(String),
    #[error("{0}")]
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorScheme {
    /// The portal's value 0.
    NoPreference,
    Dark,
    Light,
}

impl ColorScheme {
    /// `org.freedesktop.appearance` `color-scheme` value.
    pub fn from_portal(value: u32) -> Self {
        match value {
            1 => Self::Dark,
            2 => Self::Light,
            _ => Self::NoPreference,
        }
    }

    /// Parses `gsettings get ... color-scheme` output (`'prefer-dark'`).
    pub fn from_gsettings(output: &str) -> Self {
        match output.trim().trim_matches('\'') {
            "prefer-dark" => Self::Dark,
            "prefer-light" => Self::Light,
            _ => Self::NoPreference,
        }
    }
}

/// logind operations on the caller's own session or the manager.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogindOp {
    /// `Session.Lock` of the current session.
    LockSession,
    /// `Manager.Suspend(interactive = true)`.
    Suspend,
    /// `Manager.Reboot(interactive = true)`.
    Reboot,
    /// `Session.Terminate` of the current session.
    TerminateSession,
}

/// The D-Bus and subprocess layer; mocked in tests.
pub trait SystemBackend: Send + Sync {
    fn logind(&self, op: LogindOp) -> Result<(), String>;
    /// `CanSuspend` / `CanReboot`; for the session operations whether the
    /// current session can be resolved.
    fn logind_can(&self, op: LogindOp) -> Result<PowerCapability, String>;
    /// Whether `program` is found in `PATH`.
    fn command_exists(&self, program: &str) -> bool;
    /// Runs a program to completion, returning stdout; non-zero exit is an error.
    fn run(&self, program: &str, args: &[&str]) -> Result<String, String>;
    /// Reads `org.freedesktop.appearance` `color-scheme` from the settings portal.
    fn portal_color_scheme(&self) -> Result<ColorScheme, String>;
}

const COLOR_SCHEMA: &str = "org.gnome.desktop.interface";
const COLOR_KEY: &str = "color-scheme";

pub struct SystemActions<'a> {
    backend: &'a dyn SystemBackend,
    compositor: Option<&'a dyn Compositor>,
}

impl<'a> SystemActions<'a> {
    /// `compositor` is used for Log Out (a clean compositor exit is preferred
    /// over terminating the logind session).
    pub fn new(backend: &'a dyn SystemBackend, compositor: Option<&'a dyn Compositor>) -> Self {
        Self {
            backend,
            compositor,
        }
    }

    /// Whether the action can work here, with the reason if not.
    pub fn availability(&self, action: SystemAction) -> Availability {
        let unavailable = |reason: &str| Availability::Unavailable(reason.to_owned());
        match action {
            SystemAction::Lock => {
                if self.backend.logind_can(LogindOp::LockSession).is_ok()
                    || self.backend.command_exists("loginctl")
                {
                    Availability::Available
                } else {
                    unavailable("systemd-logind is not reachable and loginctl is not installed.")
                }
            }
            SystemAction::Sleep => self.power_availability(LogindOp::Suspend, "Sleep"),
            SystemAction::Restart => self.power_availability(LogindOp::Reboot, "Restart"),
            SystemAction::LogOut => {
                if self.compositor_can_quit()
                    || self.backend.logind_can(LogindOp::TerminateSession).is_ok()
                {
                    Availability::Available
                } else {
                    unavailable("No supported compositor or logind session to end.")
                }
            }
            SystemAction::ToggleMute => {
                if self.backend.command_exists("wpctl") || self.backend.command_exists("pactl") {
                    Availability::Available
                } else {
                    unavailable("Neither wpctl (WirePlumber) nor pactl is installed.")
                }
            }
            SystemAction::ToggleAppearance => match self.appearance_writer() {
                Ok(()) => Availability::Available,
                Err(reason) => Availability::Unavailable(reason),
            },
        }
    }

    fn power_availability(&self, op: LogindOp, name: &str) -> Availability {
        match self.backend.logind_can(op) {
            Ok(cap) if cap.is_allowed() => Availability::Available,
            Ok(_) => {
                Availability::Unavailable(format!("{name} is not permitted for this session."))
            }
            Err(error) => Availability::Unavailable(format!("systemd-logind: {error}")),
        }
    }

    fn compositor_can_quit(&self) -> bool {
        self.compositor
            .is_some_and(|c| c.capabilities().quit_session)
    }

    /// Runs the action. Never claims success: every failure is returned.
    pub fn perform(&self, action: SystemAction) -> Result<(), SystemActionError> {
        match action {
            SystemAction::Lock => self.lock(),
            SystemAction::Sleep => self.power(LogindOp::Suspend, "Sleep"),
            SystemAction::Restart => self.power(LogindOp::Reboot, "Restart"),
            SystemAction::LogOut => self.log_out(),
            SystemAction::ToggleMute => self.toggle_mute(),
            SystemAction::ToggleAppearance => self.toggle_appearance(),
        }
    }

    fn lock(&self) -> Result<(), SystemActionError> {
        let logind_error = match self.backend.logind(LogindOp::LockSession) {
            Ok(()) => return Ok(()),
            Err(error) => error,
        };
        if !self.backend.command_exists("loginctl") {
            return Err(SystemActionError::Failed(format!(
                "Could not lock the session: {logind_error}"
            )));
        }
        self.backend
            .run("loginctl", &["lock-session"])
            .map(|_| ())
            .map_err(|e| {
                SystemActionError::Failed(format!(
                    "Could not lock the session: {logind_error}; loginctl: {e}"
                ))
            })
    }

    fn power(&self, op: LogindOp, name: &str) -> Result<(), SystemActionError> {
        match self.backend.logind_can(op) {
            Ok(cap) if cap.is_allowed() => {}
            Ok(_) => {
                return Err(SystemActionError::Unavailable(format!(
                    "{name} is not permitted for this session."
                )));
            }
            Err(error) => {
                return Err(SystemActionError::Failed(format!(
                    "systemd-logind: {error}"
                )));
            }
        }
        self.backend
            .logind(op)
            .map_err(|e| SystemActionError::Failed(format!("{name} failed: {e}")))
    }

    fn log_out(&self) -> Result<(), SystemActionError> {
        let mut problems = Vec::new();
        if let Some(compositor) = self.compositor.filter(|c| c.capabilities().quit_session) {
            match compositor.quit_session() {
                Ok(()) => return Ok(()),
                Err(error) => problems.push(format!("{}: {error}", compositor.kind().name())),
            }
        }
        match self.backend.logind(LogindOp::TerminateSession) {
            Ok(()) => Ok(()),
            Err(error) => {
                problems.push(format!("logind: {error}"));
                Err(SystemActionError::Failed(format!(
                    "Could not log out ({}).",
                    problems.join("; ")
                )))
            }
        }
    }

    fn toggle_mute(&self) -> Result<(), SystemActionError> {
        let mut problems = Vec::new();
        for (program, args) in [
            ("wpctl", ["set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"]),
            ("pactl", ["set-sink-mute", "@DEFAULT_SINK@", "toggle"]),
        ] {
            if !self.backend.command_exists(program) {
                continue;
            }
            match self.backend.run(program, &args) {
                Ok(_) => return Ok(()),
                Err(error) => problems.push(format!("{program}: {error}")),
            }
        }
        if problems.is_empty() {
            Err(SystemActionError::Unavailable(
                "Neither wpctl (WirePlumber) nor pactl is installed.".into(),
            ))
        } else {
            Err(SystemActionError::Failed(format!(
                "Could not toggle mute ({}).",
                problems.join("; ")
            )))
        }
    }

    /// The setting can be written when `gsettings` exists and knows the key.
    fn appearance_writer(&self) -> Result<(), String> {
        if !self.backend.command_exists("gsettings") {
            return Err("gsettings is not installed, so the appearance cannot be changed.".into());
        }
        self.backend
            .run("gsettings", &["get", COLOR_SCHEMA, COLOR_KEY])
            .map(|_| ())
            .map_err(|e| format!("The {COLOR_SCHEMA} {COLOR_KEY} setting is not available: {e}"))
    }

    fn current_scheme(&self) -> Result<ColorScheme, String> {
        match self.backend.portal_color_scheme() {
            Ok(scheme) => Ok(scheme),
            Err(portal_error) => self
                .backend
                .run("gsettings", &["get", COLOR_SCHEMA, COLOR_KEY])
                .map(|out| ColorScheme::from_gsettings(&out))
                .map_err(|e| format!("portal: {portal_error}; gsettings: {e}")),
        }
    }

    fn toggle_appearance(&self) -> Result<(), SystemActionError> {
        self.appearance_writer()
            .map_err(SystemActionError::Unavailable)?;
        let current = self.current_scheme().map_err(|e| {
            SystemActionError::Failed(format!("Could not read the appearance: {e}"))
        })?;
        let target = if current == ColorScheme::Dark {
            "default"
        } else {
            "prefer-dark"
        };
        self.backend
            .run("gsettings", &["set", COLOR_SCHEMA, COLOR_KEY, target])
            .map(|_| ())
            .map_err(|e| SystemActionError::Failed(format!("Could not change the appearance: {e}")))
    }
}

// ---- real backend ----

const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);

/// logind over the system bus, portal over the session bus, subprocesses for the rest.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemBus;

impl SystemBus {
    fn manager() -> Result<zbus::blocking::Proxy<'static>, String> {
        let connection = zbus::blocking::Connection::system().map_err(|e| e.to_string())?;
        zbus::blocking::Proxy::new(
            &connection,
            "org.freedesktop.login1",
            "/org/freedesktop/login1",
            "org.freedesktop.login1.Manager",
        )
        .map_err(|e| e.to_string())
    }

    fn own_session(
        manager: &zbus::blocking::Proxy<'static>,
    ) -> Result<zbus::blocking::Proxy<'static>, String> {
        // `$XDG_SESSION_ID` is exact; the `auto` alias lets logind resolve the
        // caller's session (by process, else the user's display session) when
        // the process was not started inside the session scope.
        let by_id: Option<zbus::zvariant::OwnedObjectPath> = std::env::var("XDG_SESSION_ID")
            .ok()
            .filter(|id| !id.is_empty())
            .and_then(|id| manager.call("GetSession", &(id,)).ok());
        let path = match by_id {
            Some(path) => path,
            None => {
                zbus::zvariant::OwnedObjectPath::try_from("/org/freedesktop/login1/session/auto")
                    .map_err(|e| e.to_string())?
            }
        };
        let session = zbus::blocking::Proxy::new(
            manager.connection(),
            "org.freedesktop.login1",
            path,
            "org.freedesktop.login1.Session",
        )
        .map_err(|e| e.to_string())?;
        // Proves the session exists before anything is asked of it.
        session
            .get_property::<String>("Id")
            .map_err(|e| format!("no logind session: {e}"))?;
        Ok(session)
    }
}

impl SystemBackend for SystemBus {
    fn logind(&self, op: LogindOp) -> Result<(), String> {
        let manager = Self::manager()?;
        let result = match op {
            LogindOp::LockSession => Self::own_session(&manager)?.call::<_, _, ()>("Lock", &()),
            LogindOp::TerminateSession => {
                Self::own_session(&manager)?.call::<_, _, ()>("Terminate", &())
            }
            LogindOp::Suspend => manager.call::<_, _, ()>("Suspend", &(true,)),
            LogindOp::Reboot => manager.call::<_, _, ()>("Reboot", &(true,)),
        };
        result.map_err(|e| e.to_string())
    }

    fn logind_can(&self, op: LogindOp) -> Result<PowerCapability, String> {
        let manager = Self::manager()?;
        let method = match op {
            LogindOp::Suspend => "CanSuspend",
            LogindOp::Reboot => "CanReboot",
            LogindOp::LockSession | LogindOp::TerminateSession => {
                Self::own_session(&manager)?;
                return Ok(PowerCapability::Yes);
            }
        };
        let answer: String = manager.call(method, &()).map_err(|e| e.to_string())?;
        Ok(PowerCapability::parse(&answer))
    }

    fn command_exists(&self, program: &str) -> bool {
        crate::paths::Env::from_process()
            .path_dirs()
            .iter()
            .any(|dir| {
                std::fs::metadata(dir.join(program)).is_ok_and(|m| {
                    use std::os::unix::fs::PermissionsExt;
                    m.is_file() && m.permissions().mode() & 0o111 != 0
                })
            })
    }

    fn run(&self, program: &str, args: &[&str]) -> Result<String, String> {
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("{program}: {e}"))?;
        let started = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if started.elapsed() < COMMAND_TIMEOUT => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Ok(None) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("{program} timed out"));
                }
                Err(e) => return Err(format!("{program}: {e}")),
            }
        };
        let read = |pipe: Option<&mut dyn Read>| {
            let mut text = String::new();
            if let Some(pipe) = pipe {
                let _ = pipe.take(1 << 20).read_to_string(&mut text);
            }
            text
        };
        let stdout = read(child.stdout.as_mut().map(|p| p as &mut dyn Read));
        let stderr = read(child.stderr.as_mut().map(|p| p as &mut dyn Read));
        if status.success() {
            Ok(stdout)
        } else {
            Err(format!("{program} failed: {}", stderr.trim()))
        }
    }

    fn portal_color_scheme(&self) -> Result<ColorScheme, String> {
        let connection = zbus::blocking::Connection::session().map_err(|e| e.to_string())?;
        let proxy = zbus::blocking::Proxy::new(
            &connection,
            "org.freedesktop.portal.Desktop",
            "/org/freedesktop/portal/desktop",
            "org.freedesktop.portal.Settings",
        )
        .map_err(|e| e.to_string())?;
        let value: zbus::zvariant::OwnedValue = proxy
            .call("ReadOne", &("org.freedesktop.appearance", "color-scheme"))
            .map_err(|e| e.to_string())?;
        u32::try_from(value)
            .map(ColorScheme::from_portal)
            .map_err(|e| format!("unexpected color-scheme value: {e}"))
    }
}

/// Actions on the real machine. Never called from tests.
pub fn perform(
    action: SystemAction,
    compositor: Option<&dyn Compositor>,
) -> Result<(), SystemActionError> {
    SystemActions::new(&SystemBus, compositor).perform(action)
}

/// Availability on the real machine (read-only probes).
pub fn availability(action: SystemAction, compositor: Option<&dyn Compositor>) -> Availability {
    SystemActions::new(&SystemBus, compositor).availability(action)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compositor::{
        CompositorCapabilities, CompositorError, CompositorKind, LayoutCommand, Output, Window,
        WindowId,
    };
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Mock {
        /// Executed side effects, in order.
        calls: Mutex<Vec<String>>,
        logind_fail: Vec<LogindOp>,
        can: HashMap<&'static str, Result<PowerCapability, String>>,
        programs: Vec<&'static str>,
        failing_programs: Vec<&'static str>,
        portal: Option<ColorScheme>,
        gsettings_value: &'static str,
    }

    fn op_name(op: LogindOp) -> &'static str {
        match op {
            LogindOp::LockSession => "lock",
            LogindOp::Suspend => "suspend",
            LogindOp::Reboot => "reboot",
            LogindOp::TerminateSession => "terminate",
        }
    }

    impl SystemBackend for Mock {
        fn logind(&self, op: LogindOp) -> Result<(), String> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("logind {}", op_name(op)));
            if self.logind_fail.contains(&op) {
                Err("bus down".into())
            } else {
                Ok(())
            }
        }
        fn logind_can(&self, op: LogindOp) -> Result<PowerCapability, String> {
            self.can
                .get(op_name(op))
                .cloned()
                .unwrap_or(Ok(PowerCapability::Yes))
        }
        fn command_exists(&self, program: &str) -> bool {
            self.programs.contains(&program)
        }
        fn run(&self, program: &str, args: &[&str]) -> Result<String, String> {
            let is_read = program == "gsettings" && args.first() == Some(&"get");
            if !is_read {
                self.calls
                    .lock()
                    .unwrap()
                    .push(format!("{program} {}", args.join(" ")));
            }
            if self.failing_programs.contains(&program) {
                return Err(format!("{program} exploded"));
            }
            Ok(if is_read {
                self.gsettings_value.to_owned()
            } else {
                String::new()
            })
        }
        fn portal_color_scheme(&self) -> Result<ColorScheme, String> {
            self.portal.ok_or_else(|| "no portal".into())
        }
    }

    impl Mock {
        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    struct FakeCompositor {
        quits: Mutex<u32>,
        fail: bool,
        can_quit: bool,
    }

    impl Compositor for FakeCompositor {
        fn kind(&self) -> CompositorKind {
            CompositorKind::Niri
        }
        fn capabilities(&self) -> CompositorCapabilities {
            CompositorCapabilities {
                list_windows: true,
                focus: true,
                close: true,
                hide: false,
                exact_geometry: false,
                column_layout: true,
                quit_session: self.can_quit,
            }
        }
        fn windows(&self) -> Result<Vec<Window>, CompositorError> {
            Ok(vec![])
        }
        fn outputs(&self) -> Result<Vec<Output>, CompositorError> {
            Ok(vec![])
        }
        fn focus_window(&self, _: &WindowId) -> Result<(), CompositorError> {
            Ok(())
        }
        fn close_window(&self, _: &WindowId) -> Result<(), CompositorError> {
            Ok(())
        }
        fn hide_window(&self, _: &WindowId) -> Result<(), CompositorError> {
            Ok(())
        }
        fn quit_session(&self) -> Result<(), CompositorError> {
            *self.quits.lock().unwrap() += 1;
            if self.fail {
                Err(CompositorError::Io("gone".into()))
            } else {
                Ok(())
            }
        }
        fn layout(&self, _: &LayoutCommand) -> Result<(), CompositorError> {
            Ok(())
        }
    }

    #[test]
    fn ids_round_trip() {
        for action in SystemAction::ALL {
            assert_eq!(SystemAction::from_id(action.id()), Some(action));
        }
        assert_eq!(SystemAction::from_id("shutdown"), None);
        assert!(SystemAction::LogOut.needs_confirmation());
        assert!(!SystemAction::Lock.needs_confirmation());
    }

    #[test]
    fn lock_prefers_logind_and_falls_back_to_loginctl() {
        let mock = Mock {
            programs: vec!["loginctl"],
            ..Mock::default()
        };
        SystemActions::new(&mock, None)
            .perform(SystemAction::Lock)
            .unwrap();
        assert_eq!(mock.calls(), vec!["logind lock"]);

        let mock = Mock {
            programs: vec!["loginctl"],
            logind_fail: vec![LogindOp::LockSession],
            ..Mock::default()
        };
        SystemActions::new(&mock, None)
            .perform(SystemAction::Lock)
            .unwrap();
        assert_eq!(mock.calls(), vec!["logind lock", "loginctl lock-session"]);

        let mock = Mock {
            programs: vec!["loginctl"],
            logind_fail: vec![LogindOp::LockSession],
            failing_programs: vec!["loginctl"],
            ..Mock::default()
        };
        let error = SystemActions::new(&mock, None)
            .perform(SystemAction::Lock)
            .unwrap_err();
        assert!(error.to_string().contains("bus down") && error.to_string().contains("exploded"));

        let mock = Mock {
            logind_fail: vec![LogindOp::LockSession],
            ..Mock::default()
        };
        assert!(
            SystemActions::new(&mock, None)
                .perform(SystemAction::Lock)
                .is_err()
        );
        assert_eq!(mock.calls(), vec!["logind lock"]);
    }

    #[test]
    fn sleep_and_restart_check_capability_first() {
        let mut mock = Mock::default();
        mock.can.insert("suspend", Ok(PowerCapability::Challenge));
        mock.can.insert("reboot", Ok(PowerCapability::No));
        let actions = SystemActions::new(&mock, None);
        actions.perform(SystemAction::Sleep).unwrap();
        assert!(matches!(
            actions.perform(SystemAction::Restart),
            Err(SystemActionError::Unavailable(_))
        ));
        assert_eq!(mock.calls(), vec!["logind suspend"]);
        assert!(actions.availability(SystemAction::Sleep).is_available());
        assert!(!actions.availability(SystemAction::Restart).is_available());

        let mut mock = Mock::default();
        mock.can.insert("reboot", Err("no bus".into()));
        let actions = SystemActions::new(&mock, None);
        assert!(matches!(
            actions.perform(SystemAction::Restart),
            Err(SystemActionError::Failed(_))
        ));
        assert!(mock.calls().is_empty());
    }

    #[test]
    fn log_out_prefers_the_compositor() {
        let mock = Mock::default();
        let compositor = FakeCompositor {
            quits: Mutex::new(0),
            fail: false,
            can_quit: true,
        };
        SystemActions::new(&mock, Some(&compositor))
            .perform(SystemAction::LogOut)
            .unwrap();
        assert_eq!(*compositor.quits.lock().unwrap(), 1);
        assert!(mock.calls().is_empty());

        // Compositor failure falls back to logind.
        let failing = FakeCompositor {
            quits: Mutex::new(0),
            fail: true,
            can_quit: true,
        };
        SystemActions::new(&mock, Some(&failing))
            .perform(SystemAction::LogOut)
            .unwrap();
        assert_eq!(mock.calls(), vec!["logind terminate"]);

        // Both failing reports both.
        let mock = Mock {
            logind_fail: vec![LogindOp::TerminateSession],
            ..Mock::default()
        };
        let error = SystemActions::new(&mock, Some(&failing))
            .perform(SystemAction::LogOut)
            .unwrap_err();
        assert!(error.to_string().contains("niri") && error.to_string().contains("logind"));

        // Without quit support the compositor is not asked.
        let passive = FakeCompositor {
            quits: Mutex::new(0),
            fail: false,
            can_quit: false,
        };
        let mock = Mock::default();
        SystemActions::new(&mock, Some(&passive))
            .perform(SystemAction::LogOut)
            .unwrap();
        assert_eq!(*passive.quits.lock().unwrap(), 0);
    }

    #[test]
    fn mute_falls_back_to_pactl() {
        let mock = Mock {
            programs: vec!["wpctl", "pactl"],
            failing_programs: vec!["wpctl"],
            ..Mock::default()
        };
        SystemActions::new(&mock, None)
            .perform(SystemAction::ToggleMute)
            .unwrap();
        assert_eq!(
            mock.calls(),
            vec![
                "wpctl set-mute @DEFAULT_AUDIO_SINK@ toggle",
                "pactl set-sink-mute @DEFAULT_SINK@ toggle"
            ]
        );

        let mock = Mock::default();
        let actions = SystemActions::new(&mock, None);
        assert!(matches!(
            actions.perform(SystemAction::ToggleMute),
            Err(SystemActionError::Unavailable(_))
        ));
        assert!(
            !actions
                .availability(SystemAction::ToggleMute)
                .is_available()
        );

        let mock = Mock {
            programs: vec!["wpctl"],
            failing_programs: vec!["wpctl"],
            ..Mock::default()
        };
        assert!(matches!(
            SystemActions::new(&mock, None).perform(SystemAction::ToggleMute),
            Err(SystemActionError::Failed(_))
        ));
    }

    #[test]
    fn appearance_toggles_from_the_portal_value() {
        for (portal, expected) in [
            (ColorScheme::Dark, "default"),
            (ColorScheme::Light, "prefer-dark"),
            (ColorScheme::NoPreference, "prefer-dark"),
        ] {
            let mock = Mock {
                programs: vec!["gsettings"],
                portal: Some(portal),
                ..Mock::default()
            };
            SystemActions::new(&mock, None)
                .perform(SystemAction::ToggleAppearance)
                .unwrap();
            assert_eq!(
                mock.calls(),
                vec![format!(
                    "gsettings set org.gnome.desktop.interface color-scheme {expected}"
                )]
            );
        }
    }

    #[test]
    fn appearance_falls_back_to_gsettings_and_reports_unavailable() {
        let mock = Mock {
            programs: vec!["gsettings"],
            portal: None,
            gsettings_value: "'prefer-dark'\n",
            ..Mock::default()
        };
        SystemActions::new(&mock, None)
            .perform(SystemAction::ToggleAppearance)
            .unwrap();
        assert!(mock.calls()[0].ends_with("color-scheme default"));

        let mock = Mock {
            portal: Some(ColorScheme::Dark),
            ..Mock::default()
        };
        let actions = SystemActions::new(&mock, None);
        assert!(matches!(
            actions.perform(SystemAction::ToggleAppearance),
            Err(SystemActionError::Unavailable(_))
        ));
        assert!(
            !actions
                .availability(SystemAction::ToggleAppearance)
                .is_available()
        );
        assert!(mock.calls().is_empty());

        let mock = Mock {
            programs: vec!["gsettings"],
            failing_programs: vec!["gsettings"],
            portal: Some(ColorScheme::Dark),
            ..Mock::default()
        };
        assert!(
            !SystemActions::new(&mock, None)
                .availability(SystemAction::ToggleAppearance)
                .is_available()
        );
    }

    #[test]
    fn scheme_parsing() {
        assert_eq!(ColorScheme::from_portal(1), ColorScheme::Dark);
        assert_eq!(ColorScheme::from_portal(2), ColorScheme::Light);
        assert_eq!(ColorScheme::from_portal(0), ColorScheme::NoPreference);
        assert_eq!(
            ColorScheme::from_gsettings("'default'\n"),
            ColorScheme::NoPreference
        );
        assert_eq!(
            ColorScheme::from_gsettings("'prefer-light'"),
            ColorScheme::Light
        );
    }

    /// Read-only probes against the real machine: capability queries and the
    /// current color scheme. Executes none of the actions.
    #[test]
    #[ignore = "requires logind and the session bus"]
    fn live_read_only_probes() {
        let bus = SystemBus;
        eprintln!("CanSuspend: {:?}", bus.logind_can(LogindOp::Suspend));
        eprintln!("CanReboot: {:?}", bus.logind_can(LogindOp::Reboot));
        eprintln!("own session: {:?}", bus.logind_can(LogindOp::LockSession));
        eprintln!("portal color-scheme: {:?}", bus.portal_color_scheme());
        let actions = SystemActions::new(&bus, None);
        for action in SystemAction::ALL {
            eprintln!("{}: {:?}", action.id(), actions.availability(action));
        }
    }
}
