//! Model of the application controls: quit, force quit, restart, hide and the focus toggle of
//! application shortcuts. Port of the decision logic of `ApplicationControlService.swift`;
//! the platform layer performs the actual operations.

use std::time::Duration;

use crate::result_action::ResultActionId;

/// An operation on a (running) application.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ApplicationAction {
    /// Ask the application to quit (it may prompt about unsaved work).
    Quit,
    /// Terminate the application without asking it.
    ForceQuit,
    /// Quit, then launch again once it has terminated.
    Restart,
    /// Application shortcuts: launch, focus or hide.
    ToggleFocus,
    /// Hide all windows.
    Hide,
}

impl ApplicationAction {
    /// The operation behind a result action, if it is one.
    pub fn from_result_action(action: ResultActionId) -> Option<Self> {
        Some(match action {
            ResultActionId::QuitApplication => Self::Quit,
            ResultActionId::ForceQuitApplication => Self::ForceQuit,
            ResultActionId::RestartApplication => Self::Restart,
            ResultActionId::HideApplication => Self::Hide,
            _ => return None,
        })
    }

    /// Whether the user must confirm first (unsaved changes may be lost).
    pub fn needs_confirmation(self) -> bool {
        self == Self::ForceQuit
    }
}

/// Why an operation failed. Messages are user-presentable English.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ApplicationFailure {
    /// The application is not running.
    #[error("This application is not running.")]
    NotRunning,
    /// The application declined the request.
    #[error("The application declined the request. Save your work and try again.")]
    Rejected,
    /// A restart did not see the application quit in time.
    #[error("The application did not quit. Restart was cancelled to protect unsaved work.")]
    RestartTimeout,
}

/// How often a restart checks that the application has terminated.
pub const RESTART_POLL_INTERVAL: Duration = Duration::from_millis(100);
/// How many checks a restart makes (ten seconds) before giving up. The replacement is never
/// launched while an unsaved-document prompt may still be pending.
pub const RESTART_POLL_ATTEMPTS: u32 = 100;

/// What an application shortcut does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToggleFocusStep {
    /// The application is not running: launch it.
    Launch,
    /// It is running and frontmost: hide it.
    Hide,
    /// It is running in the background: bring it to the front.
    Activate,
}

/// The step an application shortcut takes. Launching from search keeps opening the app
/// normally; only shortcuts toggle.
pub fn plan_toggle_focus(running: bool, frontmost: bool) -> ToggleFocusStep {
    match (running, frontmost) {
        (false, _) => ToggleFocusStep::Launch,
        (true, true) => ToggleFocusStep::Hide,
        (true, false) => ToggleFocusStep::Activate,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_launch_activate_or_hide() {
        assert_eq!(plan_toggle_focus(false, false), ToggleFocusStep::Launch);
        assert_eq!(plan_toggle_focus(true, false), ToggleFocusStep::Activate);
        assert_eq!(plan_toggle_focus(true, true), ToggleFocusStep::Hide);
    }

    #[test]
    fn result_actions_map_to_operations_and_only_force_quit_confirms() {
        assert_eq!(
            ApplicationAction::from_result_action(ResultActionId::HideApplication),
            Some(ApplicationAction::Hide)
        );
        assert_eq!(
            ApplicationAction::from_result_action(ResultActionId::Open),
            None
        );
        assert!(ApplicationAction::ForceQuit.needs_confirmation());
        assert!(!ApplicationAction::Quit.needs_confirmation());
        assert!(!ApplicationAction::Restart.needs_confirmation());
    }
}
