//! Running-application detection and control (port of `ApplicationControlService`).
//!
//! Applications are identified through the windows the compositor reports
//! ([`crate::compositor`]): a window belongs to an application when its
//! `app_id`/class equals the desktop-entry id, `StartupWMClass` or `Exec`
//! program name (case-insensitive). Applications without windows (tray-only
//! daemons) are not seen as running; this is a documented limitation.
//!
//! Actions:
//! * **Focus / ToggleFocus** — focus the most recently used window; a toggle on
//!   the focused application hides it (only where the compositor can hide).
//! * **Hide** — park every window (Sway/i3 scratchpad, Hyprland special workspace);
//!   niri cannot hide windows.
//! * **Quit** — ask every window to close (like its close button, so the
//!   application can still ask about unsaved work).
//! * **ForceQuit** — SIGKILL the pids the compositor reported; needs
//!   `confirmed = true` (the UI asks first).
//! * **Restart** — Quit, wait up to ten seconds for windows and processes to go
//!   away, then tell the caller to relaunch; stops (and reports) if still running.

use std::collections::HashSet;
use std::thread;
use std::time::{Duration, Instant};

use crate::compositor::{Compositor, CompositorError, Window};

/// How an installed application is recognised among windows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppIdentity {
    /// Desktop-entry id, with or without the `.desktop` suffix.
    pub desktop_id: String,
    pub startup_wm_class: Option<String>,
    /// Program name from `Exec` (basename).
    pub exec_basename: Option<String>,
}

/// `Exec` programs that start other programs: they never identify an application.
const GENERIC_LAUNCHERS: &[&str] = &[
    "env",
    "sh",
    "bash",
    "zsh",
    "fish",
    "flatpak",
    "snap",
    "gtk-launch",
    "xdg-open",
    "gio",
    "python",
    "python3",
    "java",
    "electron",
    "node",
    "dbus-launch",
    "dbus-send",
    "systemd-run",
    "uwsm",
    "uwsm-app",
    "sudo",
    "doas",
    "wine",
    "steam",
];

impl AppIdentity {
    /// The identity of a desktop entry: its id, `StartupWMClass` and the program of its `Exec`
    /// line (first word, quotes stripped, basename).
    pub fn from_entry(desktop_id: &str, startup_wm_class: Option<&str>, exec: &str) -> Self {
        let program = exec
            .split_whitespace()
            .next()
            .map(|word| word.trim_matches(|c| c == '"' || c == '\''))
            .and_then(|word| word.rsplit('/').next())
            .filter(|word| !word.is_empty() && !word.starts_with('%'));
        Self {
            desktop_id: desktop_id.to_owned(),
            startup_wm_class: startup_wm_class
                .filter(|class| !class.is_empty())
                .map(str::to_owned),
            exec_basename: program.map(str::to_owned),
        }
    }

    fn candidates(&self) -> Vec<String> {
        let mut names = Vec::new();
        let stem = self
            .desktop_id
            .strip_suffix(".desktop")
            .unwrap_or(&self.desktop_id);
        names.push(stem.to_lowercase());
        if let Some(class) = &self.startup_wm_class {
            names.push(class.to_lowercase());
        }
        if let Some(exec) = &self.exec_basename {
            let exec = exec.to_lowercase();
            if !GENERIC_LAUNCHERS.contains(&exec.as_str()) {
                names.push(exec);
            }
        }
        names.retain(|n| !n.is_empty());
        names
    }

    /// Whether `window` belongs to this application.
    pub fn matches(&self, window: &Window) -> bool {
        let Some(app_id) = window.app_id.as_deref() else {
            return false;
        };
        self.matches_app_id(app_id)
    }

    /// Resolve a display-server application identity without inventing a window.
    pub fn matches_app_id(&self, app_id: &str) -> bool {
        let app_id = app_id.to_lowercase();
        if app_id.is_empty() {
            return false;
        }
        let names = self.candidates();
        if names.contains(&app_id) {
            return true;
        }
        // Reverse-DNS ids (`org.mozilla.firefox`) against a plain class (`firefox`).
        !app_id.contains('.')
            && self
                .desktop_id
                .strip_suffix(".desktop")
                .unwrap_or(&self.desktop_id)
                .rsplit_once('.')
                .is_some_and(|(_, last)| last.to_lowercase() == app_id)
    }
}

/// Windows of `app` among `windows`.
pub fn windows_of<'a>(app: &AppIdentity, windows: &'a [Window]) -> Vec<&'a Window> {
    windows.iter().filter(|w| app.matches(w)).collect()
}

/// Desktop ids (as given in [`AppIdentity::desktop_id`]) that have a window.
pub fn running_desktop_ids(apps: &[AppIdentity], windows: &[Window]) -> HashSet<String> {
    apps.iter()
        .filter(|app| windows.iter().any(|w| app.matches(w)))
        .map(|app| app.desktop_id.clone())
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppAction {
    /// Focus a running app, or ask the caller to launch it; hide it if already focused.
    ToggleFocus,
    /// Like `ToggleFocus` but never hides.
    Focus,
    Hide,
    Quit,
    ForceQuit,
    Restart,
}

/// What the caller still has to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppOutcome {
    /// Finished; nothing else to do.
    Done,
    /// The application is not running: the caller launches it.
    Launch,
    /// Restart: the old instance is gone; the caller launches it again.
    Relaunch,
    /// Toggle on a focused app where hiding is unsupported: nothing was done.
    AlreadyFocused,
}

#[derive(Debug, thiserror::Error)]
pub enum AppControlError {
    #[error("This application is not running.")]
    NotRunning,
    #[error("Force Quit needs confirmation.")]
    ConfirmationRequired,
    #[error("{0}")]
    Unsupported(String),
    #[error("The application did not quit. Restart was cancelled to protect unsaved work.")]
    RestartTimeout,
    #[error("Could not signal process {pid}: {reason}")]
    Signal { pid: u32, reason: String },
    #[error(transparent)]
    Compositor(CompositorError),
}

impl From<CompositorError> for AppControlError {
    fn from(error: CompositorError) -> Self {
        match error {
            CompositorError::Unsupported { backend, what } => {
                Self::Unsupported(format!("{backend}: {what}"))
            }
            other => Self::Compositor(other),
        }
    }
}

/// Sends signals to processes (mockable).
pub trait ProcessControl {
    /// SIGKILL.
    fn kill(&self, pid: u32) -> Result<(), String>;
    fn is_alive(&self, pid: u32) -> bool;
}

/// Real signals through `rustix`.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemProcesses;

fn to_pid(pid: u32) -> Result<rustix::process::Pid, String> {
    i32::try_from(pid)
        .ok()
        .and_then(rustix::process::Pid::from_raw)
        .ok_or_else(|| format!("invalid pid {pid}"))
}

impl ProcessControl for SystemProcesses {
    fn kill(&self, pid: u32) -> Result<(), String> {
        // Never signal init or ourselves.
        if pid <= 1 || pid == std::process::id() {
            return Err("refusing to signal this process".into());
        }
        rustix::process::kill_process(to_pid(pid)?, rustix::process::Signal::KILL)
            .map_err(|e| e.to_string())
    }

    fn is_alive(&self, pid: u32) -> bool {
        to_pid(pid).is_ok_and(|p| {
            match rustix::process::test_kill_process(p) {
                Ok(()) => true,
                // Exists but belongs to someone else.
                Err(e) => e == rustix::io::Errno::PERM,
            }
        })
    }
}

/// Waiting policy for [`AppAction::Restart`].
#[derive(Debug, Clone, Copy)]
pub struct RestartPolicy {
    pub timeout: Duration,
    pub interval: Duration,
}

impl Default for RestartPolicy {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(10),
            interval: Duration::from_millis(100),
        }
    }
}

pub struct AppController<'a> {
    compositor: &'a dyn Compositor,
    processes: &'a dyn ProcessControl,
    restart: RestartPolicy,
}

impl<'a> AppController<'a> {
    pub fn new(compositor: &'a dyn Compositor, processes: &'a dyn ProcessControl) -> Self {
        Self {
            compositor,
            processes,
            restart: RestartPolicy::default(),
        }
    }

    #[must_use]
    pub fn with_restart_policy(mut self, policy: RestartPolicy) -> Self {
        self.restart = policy;
        self
    }

    /// Whether the application has at least one window.
    pub fn is_running(&self, app: &AppIdentity) -> Result<bool, AppControlError> {
        Ok(self.compositor.windows()?.iter().any(|w| app.matches(w)))
    }

    /// Whether Hide / toggle-hide can work with this compositor.
    pub fn hide_supported(&self) -> bool {
        self.compositor.capabilities().hide
    }

    /// `confirmed` must be true for [`AppAction::ForceQuit`] (the UI asks first).
    pub fn perform(
        &self,
        action: AppAction,
        app: &AppIdentity,
        confirmed: bool,
    ) -> Result<AppOutcome, AppControlError> {
        if action == AppAction::ForceQuit && !confirmed {
            return Err(AppControlError::ConfirmationRequired);
        }
        let windows = self.compositor.windows()?;
        let mine: Vec<&Window> = windows_of(app, &windows);
        if mine.is_empty() {
            return match action {
                AppAction::ToggleFocus | AppAction::Focus => Ok(AppOutcome::Launch),
                _ => Err(AppControlError::NotRunning),
            };
        }
        match action {
            AppAction::Hide => self.hide(&mine).map(|()| AppOutcome::Done),
            AppAction::ToggleFocus if mine.iter().any(|w| w.focused && !w.hidden) => {
                if self.hide_supported() {
                    self.hide(&mine).map(|()| AppOutcome::Done)
                } else {
                    Ok(AppOutcome::AlreadyFocused)
                }
            }
            AppAction::ToggleFocus | AppAction::Focus => {
                let target = most_recent(&mine);
                self.compositor.show_window(&target.id)?;
                Ok(AppOutcome::Done)
            }
            AppAction::Quit => {
                self.close_all(&mine)?;
                Ok(AppOutcome::Done)
            }
            AppAction::ForceQuit => {
                self.kill_all(&mine)?;
                Ok(AppOutcome::Done)
            }
            AppAction::Restart => {
                let pids = pids_of(&mine);
                self.close_all(&mine)?;
                self.wait_until_gone(app, &pids)?;
                Ok(AppOutcome::Relaunch)
            }
        }
    }

    fn hide(&self, windows: &[&Window]) -> Result<(), AppControlError> {
        if !self.hide_supported() {
            return Err(AppControlError::Unsupported(format!(
                "{} cannot hide windows.",
                self.compositor.kind().name()
            )));
        }
        for window in windows.iter().filter(|w| !w.hidden) {
            self.compositor.hide_window(&window.id)?;
        }
        Ok(())
    }

    fn close_all(&self, windows: &[&Window]) -> Result<(), AppControlError> {
        let mut first_error = None;
        for window in windows {
            if let Err(error) = self.compositor.close_window(&window.id) {
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), |e| Err(e.into()))
    }

    fn kill_all(&self, windows: &[&Window]) -> Result<(), AppControlError> {
        let pids = pids_of(windows);
        if pids.is_empty() {
            return Err(AppControlError::Unsupported(
                "The compositor did not report a process for this application.".into(),
            ));
        }
        for pid in pids {
            self.processes
                .kill(pid)
                .map_err(|reason| AppControlError::Signal { pid, reason })?;
        }
        Ok(())
    }

    fn wait_until_gone(&self, app: &AppIdentity, pids: &[u32]) -> Result<(), AppControlError> {
        let started = Instant::now();
        loop {
            let windows_gone = !self.is_running(app)?;
            if windows_gone && !pids.iter().any(|p| self.processes.is_alive(*p)) {
                return Ok(());
            }
            if started.elapsed() >= self.restart.timeout {
                return Err(AppControlError::RestartTimeout);
            }
            thread::sleep(self.restart.interval);
        }
    }
}

fn pids_of(windows: &[&Window]) -> Vec<u32> {
    let mut pids: Vec<u32> = windows.iter().filter_map(|w| w.pid).collect();
    pids.sort_unstable();
    pids.dedup();
    pids
}

fn most_recent<'a>(windows: &[&'a Window]) -> &'a Window {
    // `windows` is never empty here; fall back to the first element.
    windows
        .iter()
        .copied()
        .max_by_key(|w| (!w.hidden, w.focus_rank.unwrap_or(0)))
        .unwrap_or(windows[0])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compositor::WindowId;
    use crate::compositor::{CompositorCapabilities, CompositorKind, LayoutCommand, Output};
    use std::sync::Mutex;

    fn window(id: &str, app_id: &str, pid: u32) -> Window {
        Window {
            id: WindowId(id.into()),
            app_id: Some(app_id.into()),
            title: String::new(),
            pid: Some(pid),
            focused: false,
            floating: false,
            fullscreen: false,
            hidden: false,
            output: None,
            workspace: None,
            frame: None,
            size: None,
            focus_rank: None,
        }
    }

    #[test]
    fn identity_from_an_exec_line_uses_the_program_name() {
        let app = AppIdentity::from_entry("a.desktop", Some(""), "/usr/bin/foo --new-window %U");
        assert_eq!(app.exec_basename.as_deref(), Some("foo"));
        assert_eq!(app.startup_wm_class, None);
        let quoted = AppIdentity::from_entry("a.desktop", Some("Foo"), "\"/opt/bar\" %F");
        assert_eq!(quoted.exec_basename.as_deref(), Some("bar"));
        assert_eq!(quoted.startup_wm_class.as_deref(), Some("Foo"));
        assert_eq!(
            AppIdentity::from_entry("a.desktop", None, "").exec_basename,
            None
        );
        assert_eq!(
            AppIdentity::from_entry("a.desktop", None, "%u").exec_basename,
            None
        );
    }

    fn identity(id: &str, class: Option<&str>, exec: Option<&str>) -> AppIdentity {
        AppIdentity {
            desktop_id: id.into(),
            startup_wm_class: class.map(Into::into),
            exec_basename: exec.map(Into::into),
        }
    }

    struct Fake {
        windows: Mutex<Vec<Window>>,
        hide: bool,
        log: Mutex<Vec<String>>,
        /// Windows disappear after a close request.
        close_removes: bool,
    }

    impl Fake {
        fn new(windows: Vec<Window>, hide: bool) -> Self {
            Self {
                windows: Mutex::new(windows),
                hide,
                log: Mutex::default(),
                close_removes: true,
            }
        }
        fn log(&self) -> Vec<String> {
            self.log.lock().unwrap().clone()
        }
    }

    impl Compositor for Fake {
        fn kind(&self) -> CompositorKind {
            CompositorKind::Sway
        }
        fn capabilities(&self) -> CompositorCapabilities {
            CompositorCapabilities {
                list_windows: true,
                focus: true,
                close: true,
                hide: self.hide,
                exact_geometry: true,
                column_layout: false,
                quit_session: true,
            }
        }
        fn windows(&self) -> Result<Vec<Window>, CompositorError> {
            Ok(self.windows.lock().unwrap().clone())
        }
        fn outputs(&self) -> Result<Vec<Output>, CompositorError> {
            Ok(vec![])
        }
        fn focus_window(&self, id: &WindowId) -> Result<(), CompositorError> {
            self.log.lock().unwrap().push(format!("focus {id}"));
            Ok(())
        }
        fn close_window(&self, id: &WindowId) -> Result<(), CompositorError> {
            self.log.lock().unwrap().push(format!("close {id}"));
            if self.close_removes {
                self.windows.lock().unwrap().retain(|w| &w.id != id);
            }
            Ok(())
        }
        fn hide_window(&self, id: &WindowId) -> Result<(), CompositorError> {
            if !self.hide {
                return Err(CompositorError::Unsupported {
                    backend: "fake",
                    what: "hide",
                });
            }
            self.log.lock().unwrap().push(format!("hide {id}"));
            Ok(())
        }
        fn show_window(&self, id: &WindowId) -> Result<(), CompositorError> {
            self.log.lock().unwrap().push(format!("show {id}"));
            Ok(())
        }
        fn quit_session(&self) -> Result<(), CompositorError> {
            Ok(())
        }
        fn layout(&self, _: &LayoutCommand) -> Result<(), CompositorError> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct Procs {
        killed: Mutex<Vec<u32>>,
        alive: Mutex<Vec<u32>>,
        fail: bool,
    }

    impl ProcessControl for Procs {
        fn kill(&self, pid: u32) -> Result<(), String> {
            if self.fail {
                return Err("EPERM".into());
            }
            self.killed.lock().unwrap().push(pid);
            Ok(())
        }
        fn is_alive(&self, pid: u32) -> bool {
            self.alive.lock().unwrap().contains(&pid)
        }
    }

    fn quick() -> RestartPolicy {
        RestartPolicy {
            timeout: Duration::from_millis(60),
            interval: Duration::from_millis(5),
        }
    }

    #[test]
    fn matching_rules() {
        let firefox = identity(
            "org.mozilla.firefox.desktop",
            Some("firefox"),
            Some("firefox"),
        );
        assert!(firefox.matches(&window("1", "firefox", 1)));
        assert!(firefox.matches(&window("1", "Firefox", 1)));
        assert!(firefox.matches(&window("1", "org.mozilla.firefox", 1)));
        assert!(!firefox.matches(&window("1", "firefox-developer-edition", 1)));
        let ghostty = identity("com.mitchellh.ghostty.desktop", None, Some("ghostty"));
        assert!(ghostty.matches(&window("1", "com.mitchellh.ghostty", 1)));
        assert!(ghostty.matches(&window("1", "ghostty", 1)));
        // Generic launchers never identify an app.
        let flatpak = identity("a.b.c.desktop", None, Some("flatpak"));
        assert!(!flatpak.matches(&window("1", "flatpak", 1)));
        let mut anonymous = window("1", "x", 1);
        anonymous.app_id = None;
        assert!(!firefox.matches(&anonymous));
    }

    #[test]
    fn running_ids() {
        let apps = vec![
            identity("a.desktop", Some("A"), None),
            identity("b.desktop", None, None),
        ];
        let windows = vec![window("1", "a", 1)];
        let running = running_desktop_ids(&apps, &windows);
        assert!(running.contains("a.desktop") && !running.contains("b.desktop"));
    }

    #[test]
    fn not_running_launches_or_errors() {
        let compositor = Fake::new(vec![], true);
        let procs = Procs::default();
        let control = AppController::new(&compositor, &procs);
        let app = identity("a.desktop", None, None);
        assert_eq!(
            control
                .perform(AppAction::ToggleFocus, &app, false)
                .unwrap(),
            AppOutcome::Launch
        );
        assert!(matches!(
            control.perform(AppAction::Quit, &app, false),
            Err(AppControlError::NotRunning)
        ));
    }

    #[test]
    fn toggle_focuses_then_hides() {
        let mut focused = window("2", "a", 20);
        focused.focused = true;
        let mut other = window("1", "a", 20);
        other.focus_rank = Some(5);
        let app = identity("a.desktop", None, None);
        let procs = Procs::default();

        let compositor = Fake::new(vec![other.clone(), window("3", "zzz", 3)], true);
        let control = AppController::new(&compositor, &procs);
        control
            .perform(AppAction::ToggleFocus, &app, false)
            .unwrap();
        assert_eq!(compositor.log(), vec!["show 1"]);

        let compositor = Fake::new(vec![other.clone(), focused.clone()], true);
        let control = AppController::new(&compositor, &procs);
        control
            .perform(AppAction::ToggleFocus, &app, false)
            .unwrap();
        assert_eq!(compositor.log(), vec!["hide 1", "hide 2"]);

        // Where hide is unsupported a toggle on the focused app does nothing.
        let compositor = Fake::new(vec![focused], false);
        let control = AppController::new(&compositor, &procs);
        assert_eq!(
            control
                .perform(AppAction::ToggleFocus, &app, false)
                .unwrap(),
            AppOutcome::AlreadyFocused
        );
        assert!(compositor.log().is_empty());
        assert!(matches!(
            control.perform(AppAction::Hide, &app, false),
            Err(AppControlError::Unsupported(_))
        ));
    }

    #[test]
    fn focus_prefers_visible_recent_windows() {
        let mut hidden = window("1", "a", 1);
        hidden.hidden = true;
        hidden.focus_rank = Some(100);
        let mut visible = window("2", "a", 1);
        visible.focus_rank = Some(1);
        let compositor = Fake::new(vec![hidden, visible], true);
        let procs = Procs::default();
        AppController::new(&compositor, &procs)
            .perform(AppAction::Focus, &identity("a.desktop", None, None), false)
            .unwrap();
        assert_eq!(compositor.log(), vec!["show 2"]);
    }

    #[test]
    fn quit_closes_every_window() {
        let compositor = Fake::new(vec![window("1", "a", 1), window("2", "a", 1)], true);
        let procs = Procs::default();
        AppController::new(&compositor, &procs)
            .perform(AppAction::Quit, &identity("a.desktop", None, None), false)
            .unwrap();
        assert_eq!(compositor.log(), vec!["close 1", "close 2"]);
        assert!(procs.killed.lock().unwrap().is_empty());
    }

    #[test]
    fn force_quit_needs_confirmation_and_kills_unique_pids() {
        let compositor = Fake::new(
            vec![
                window("1", "a", 10),
                window("2", "a", 10),
                window("3", "a", 11),
            ],
            true,
        );
        let procs = Procs::default();
        let control = AppController::new(&compositor, &procs);
        let app = identity("a.desktop", None, None);
        assert!(matches!(
            control.perform(AppAction::ForceQuit, &app, false),
            Err(AppControlError::ConfirmationRequired)
        ));
        assert!(procs.killed.lock().unwrap().is_empty());
        control.perform(AppAction::ForceQuit, &app, true).unwrap();
        assert_eq!(*procs.killed.lock().unwrap(), vec![10, 11]);

        let failing = Procs {
            fail: true,
            ..Procs::default()
        };
        let control = AppController::new(&compositor, &failing);
        assert!(matches!(
            control.perform(AppAction::ForceQuit, &app, true),
            Err(AppControlError::Signal { pid: 10, .. })
        ));
    }

    #[test]
    fn restart_waits_for_windows_and_processes() {
        let compositor = Fake::new(vec![window("1", "a", 10)], true);
        let procs = Procs::default();
        let control = AppController::new(&compositor, &procs).with_restart_policy(quick());
        let app = identity("a.desktop", None, None);
        assert_eq!(
            control.perform(AppAction::Restart, &app, false).unwrap(),
            AppOutcome::Relaunch
        );
        assert_eq!(compositor.log(), vec!["close 1"]);

        // The window closes but the process lingers: no relaunch.
        let compositor = Fake::new(vec![window("1", "a", 10)], true);
        let lingering = Procs {
            alive: Mutex::new(vec![10]),
            ..Procs::default()
        };
        let control = AppController::new(&compositor, &lingering).with_restart_policy(quick());
        assert!(matches!(
            control.perform(AppAction::Restart, &app, false),
            Err(AppControlError::RestartTimeout)
        ));

        // The application refuses to close (unsaved-work prompt).
        let mut refusing = Fake::new(vec![window("1", "a", 10)], true);
        refusing.close_removes = false;
        let control = AppController::new(&refusing, &procs).with_restart_policy(quick());
        assert!(matches!(
            control.perform(AppAction::Restart, &app, false),
            Err(AppControlError::RestartTimeout)
        ));
    }

    #[test]
    fn real_process_control_refuses_dangerous_pids() {
        assert!(SystemProcesses.kill(0).is_err());
        assert!(SystemProcesses.kill(1).is_err());
        assert!(SystemProcesses.kill(std::process::id()).is_err());
        assert!(SystemProcesses.is_alive(std::process::id()));
        assert!(!SystemProcesses.is_alive(0));
    }
}
