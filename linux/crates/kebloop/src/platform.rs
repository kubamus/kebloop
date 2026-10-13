//! GTK-free decisions about what the platform layer can do on this machine: which launcher
//! commands to offer (system actions, window management), which controls a running application
//! gets, and how installed applications are recognised among compositor windows.

use std::collections::HashSet;

use kebloop_platform::app_control::AppIdentity;
use kebloop_platform::apps::DesktopApp;
use kebloop_platform::compositor::{CompositorCapabilities, CompositorKind};
use kebloop_platform::system_actions::SystemAction;
use kebloop_platform::window_management::{ActionSupport, WindowAction, support};
use kebloop_services::commands::LauncherCommand;

/// The system action behind a launcher command, if it is one.
pub fn system_action_of(command: LauncherCommand) -> Option<SystemAction> {
    command
        .system_action()
        .and_then(|_| SystemAction::from_id(command.id()))
}

/// The window arrangement behind a launcher command, if it is one.
pub fn window_action_of(command: LauncherCommand) -> Option<WindowAction> {
    command
        .window_action()
        .and_then(|_| WindowAction::from_id(command.id()))
}

/// How a window command differs from its macOS behaviour on this compositor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowNote {
    /// Works, but not exactly like macOS (the text says how).
    Approximate(&'static str),
    /// Only works for floating windows.
    FloatingOnly,
}

/// Which built-in commands this machine can run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommandAvailability {
    shutdown: bool,
    system: HashSet<SystemAction>,
    window_kind: Option<CompositorKind>,
}

impl CommandAvailability {
    /// `system` are the system actions whose probe succeeded; `window_kind` is the compositor
    /// whose window commands are offered (`None` when there is no supported compositor IPC).
    pub fn new(
        shutdown: bool,
        system: impl IntoIterator<Item = SystemAction>,
        window_kind: Option<CompositorKind>,
    ) -> Self {
        Self {
            shutdown,
            system: system.into_iter().collect(),
            window_kind,
        }
    }

    /// Whether Shut Down may be offered.
    pub fn shutdown(&self) -> bool {
        self.shutdown
    }

    /// The compositor whose window commands are offered.
    pub fn window_kind(&self) -> Option<CompositorKind> {
        self.window_kind
    }

    /// Whether `command` is offered right now.
    pub fn offers(&self, command: LauncherCommand) -> bool {
        if command == LauncherCommand::Shutdown {
            return self.shutdown;
        }
        if let Some(action) = system_action_of(command) {
            return self.system.contains(&action);
        }
        if let Some(action) = window_action_of(command) {
            return self
                .window_kind
                .is_some_and(|kind| support(kind, action).is_offered());
        }
        command.is_available()
    }

    /// Every offered command in presentation order.
    pub fn commands(&self) -> impl Iterator<Item = LauncherCommand> + '_ {
        LauncherCommand::ALL
            .into_iter()
            .filter(|command| self.offers(*command))
    }

    /// How a window command differs from macOS here (`None` for full support or other commands).
    pub fn window_note(&self, command: LauncherCommand) -> Option<WindowNote> {
        let kind = self.window_kind?;
        match support(kind, window_action_of(command)?) {
            ActionSupport::Approximate(note) => Some(WindowNote::Approximate(note)),
            ActionSupport::FloatingOnly => Some(WindowNote::FloatingOnly),
            ActionSupport::Full | ActionSupport::Unsupported(_) => None,
        }
    }
}

/// What the compositor lets Kebloop do to a running application.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AppControls {
    /// Hide (park) all windows of the application.
    pub hide: bool,
    /// Ask the application to quit (close its windows); also used by Restart.
    pub close: bool,
    /// Windows can be listed, so a running application is recognised and force-quit by pid.
    pub list: bool,
}

impl AppControls {
    /// The controls for a compositor with `capabilities` (`None`: no compositor IPC).
    pub fn of(capabilities: Option<CompositorCapabilities>) -> Self {
        capabilities.map_or_else(Self::default, |caps| Self {
            hide: caps.hide && caps.list_windows,
            close: caps.close && caps.list_windows,
            list: caps.list_windows,
        })
    }
}

/// How a desktop entry is recognised among windows.
pub fn identity_of(app: &DesktopApp) -> AppIdentity {
    AppIdentity::from_entry(&app.id, app.startup_wm_class.as_deref(), &app.exec)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_platform_specific_is_offered_without_backends() {
        let none = CommandAvailability::default();
        assert!(!none.offers(LauncherCommand::Shutdown));
        assert!(!none.offers(LauncherCommand::Lock));
        assert!(!none.offers(LauncherCommand::LeftHalf));
        assert!(!none.offers(LauncherCommand::AppleShortcuts));
        assert!(none.offers(LauncherCommand::Emoji));
        assert!(none.offers(LauncherCommand::CalculationHistory));
        assert_eq!(
            none.commands().collect::<Vec<_>>(),
            LauncherCommand::available()
                .filter(|c| *c != LauncherCommand::Shutdown)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn system_commands_follow_their_probe() {
        let some =
            CommandAvailability::new(true, [SystemAction::Lock, SystemAction::ToggleMute], None);
        assert!(some.offers(LauncherCommand::Shutdown));
        assert!(some.offers(LauncherCommand::Lock));
        assert!(some.offers(LauncherCommand::ToggleMute));
        assert!(!some.offers(LauncherCommand::Sleep));
        assert!(!some.offers(LauncherCommand::Restart));
        assert!(!some.offers(LauncherCommand::LogOut));
        assert!(!some.offers(LauncherCommand::ToggleAppearance));
    }

    #[test]
    fn window_commands_need_a_compositor_and_carry_their_notes() {
        let niri = CommandAvailability::new(false, [], Some(CompositorKind::Niri));
        assert!(niri.offers(LauncherCommand::LeftHalf));
        assert!(niri.offers(LauncherCommand::NextDisplay));
        assert_eq!(
            niri.window_note(LauncherCommand::TopHalf),
            Some(WindowNote::FloatingOnly)
        );
        assert!(matches!(
            niri.window_note(LauncherCommand::LeftHalf),
            Some(WindowNote::Approximate(_))
        ));
        assert_eq!(niri.window_note(LauncherCommand::CenterWindow), None);
        assert_eq!(niri.window_note(LauncherCommand::Emoji), None);
        let sway = CommandAvailability::new(false, [], Some(CompositorKind::Sway));
        assert!(sway.offers(LauncherCommand::TopLeftQuarter));
        assert_eq!(sway.window_note(LauncherCommand::TopLeftQuarter), None);
        assert!(!CommandAvailability::default().offers(LauncherCommand::CenterWindow));
    }

    #[test]
    fn every_window_and_system_command_maps_to_a_platform_action() {
        for command in LauncherCommand::ALL {
            assert_eq!(
                command.window_action().is_some(),
                window_action_of(command).is_some()
            );
            assert_eq!(
                command.system_action().is_some(),
                system_action_of(command).is_some()
            );
        }
    }

    #[test]
    fn app_controls_follow_the_capabilities() {
        assert_eq!(AppControls::of(None), AppControls::default());
        let caps = |hide| CompositorCapabilities {
            list_windows: true,
            focus: true,
            close: true,
            hide,
            exact_geometry: false,
            column_layout: false,
            quit_session: true,
        };
        let niri = AppControls::of(Some(caps(false)));
        assert!(!niri.hide && niri.close && niri.list);
        assert!(AppControls::of(Some(caps(true))).hide);
    }

    #[test]
    fn every_window_note_has_english_and_polish_text() {
        use kebloop_services::l10n::{Language, catalog_keys};
        let english = catalog_keys(Language::English);
        let polish = catalog_keys(Language::Polish);
        for kind in [
            CompositorKind::Niri,
            CompositorKind::Hyprland,
            CompositorKind::Sway,
            CompositorKind::I3,
        ] {
            let availability = CommandAvailability::new(false, [], Some(kind));
            for command in LauncherCommand::ALL {
                if let Some(WindowNote::Approximate(text)) = availability.window_note(command) {
                    assert!(english.contains(text), "no English entry for {text:?}");
                    assert!(polish.contains(text), "no Polish entry for {text:?}");
                }
            }
        }
    }
}
