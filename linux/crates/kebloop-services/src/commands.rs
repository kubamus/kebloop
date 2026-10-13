//! Launcher command registry. The string ids are persisted (shortcut bindings, pins) and must
//! stay stable across releases.

use crate::l10n::Localizer;
pub use crate::window_management::WindowAction;

/// Built-in launcher commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum LauncherCommand {
    /// Lock the screen (macOS only).
    Lock,
    /// Put the computer to sleep (macOS only).
    Sleep,
    /// Restart the computer (macOS only).
    Restart,
    /// Log out (macOS only).
    LogOut,
    /// Mute or unmute the output (macOS only).
    ToggleMute,
    /// Switch between light and dark appearance (macOS only).
    ToggleAppearance,
    /// Window: left half.
    LeftHalf,
    /// Window: right half.
    RightHalf,
    /// Window: maximize.
    MaximizeWindow,
    /// Window: center.
    CenterWindow,
    /// Window: restore the saved frame.
    RestoreWindow,
    /// Window: move to the next display.
    NextDisplay,
    /// Accepted calculator results.
    CalculationHistory,
    /// Window: top half.
    TopHalf,
    /// Window: bottom half.
    BottomHalf,
    /// Window: top-left quarter.
    TopLeftQuarter,
    /// Window: top-right quarter.
    TopRightQuarter,
    /// Window: bottom-left quarter.
    BottomLeftQuarter,
    /// Window: bottom-right quarter.
    BottomRightQuarter,
    /// Window: left third.
    LeftThird,
    /// Window: center third.
    CenterThird,
    /// Window: right third.
    RightThird,
    /// Window: left two thirds.
    LeftTwoThirds,
    /// Window: right two thirds.
    RightTwoThirds,
    /// Window: move to the previous display.
    PreviousDisplay,
    /// Snippet browser.
    Snippets,
    /// Script commands.
    Scripts,
    /// Extensions.
    Extensions,
    /// Clipboard history.
    Clipboard,
    /// Quicklinks.
    Quicklinks,
    /// Emoji and symbols.
    Emoji,
    /// Settings window.
    Settings,
    /// Shut down the computer.
    Shutdown,
    /// Browse all commands.
    Browse,
    /// Help.
    Help,
    /// Onboarding.
    Onboarding,
    /// Profile switcher.
    Profiles,
    /// Web search.
    WebSearch,
    /// Apple Shortcuts (macOS only; kept for configuration compatibility).
    AppleShortcuts,
}

impl LauncherCommand {
    /// Every command in presentation order.
    pub const ALL: [LauncherCommand; 39] = [
        Self::Lock,
        Self::Sleep,
        Self::Restart,
        Self::LogOut,
        Self::ToggleMute,
        Self::ToggleAppearance,
        Self::LeftHalf,
        Self::RightHalf,
        Self::MaximizeWindow,
        Self::CenterWindow,
        Self::RestoreWindow,
        Self::NextDisplay,
        Self::CalculationHistory,
        Self::TopHalf,
        Self::BottomHalf,
        Self::TopLeftQuarter,
        Self::TopRightQuarter,
        Self::BottomLeftQuarter,
        Self::BottomRightQuarter,
        Self::LeftThird,
        Self::CenterThird,
        Self::RightThird,
        Self::LeftTwoThirds,
        Self::RightTwoThirds,
        Self::PreviousDisplay,
        Self::Snippets,
        Self::Scripts,
        Self::Extensions,
        Self::Clipboard,
        Self::Quicklinks,
        Self::Emoji,
        Self::Settings,
        Self::Shutdown,
        Self::Browse,
        Self::Help,
        Self::Onboarding,
        Self::Profiles,
        Self::WebSearch,
        Self::AppleShortcuts,
    ];

    /// Stable persisted identifier.
    pub fn id(self) -> &'static str {
        match self {
            Self::Lock => "lock",
            Self::Sleep => "sleep",
            Self::Restart => "restart",
            Self::LogOut => "logOut",
            Self::ToggleMute => "toggleMute",
            Self::ToggleAppearance => "toggleAppearance",
            Self::LeftHalf => "leftHalf",
            Self::RightHalf => "rightHalf",
            Self::MaximizeWindow => "maximizeWindow",
            Self::CenterWindow => "centerWindow",
            Self::RestoreWindow => "restoreWindow",
            Self::NextDisplay => "nextDisplay",
            Self::CalculationHistory => "calculationHistory",
            Self::TopHalf => "topHalf",
            Self::BottomHalf => "bottomHalf",
            Self::TopLeftQuarter => "topLeftQuarter",
            Self::TopRightQuarter => "topRightQuarter",
            Self::BottomLeftQuarter => "bottomLeftQuarter",
            Self::BottomRightQuarter => "bottomRightQuarter",
            Self::LeftThird => "leftThird",
            Self::CenterThird => "centerThird",
            Self::RightThird => "rightThird",
            Self::LeftTwoThirds => "leftTwoThirds",
            Self::RightTwoThirds => "rightTwoThirds",
            Self::PreviousDisplay => "previousDisplay",
            Self::Snippets => "snippets",
            Self::Scripts => "scripts",
            Self::Extensions => "extensions",
            Self::Clipboard => "clipboard",
            Self::Quicklinks => "quicklinks",
            Self::Emoji => "emoji",
            Self::Settings => "settings",
            Self::Shutdown => "shutdown",
            Self::Browse => "browse",
            Self::Help => "help",
            Self::Onboarding => "onboarding",
            Self::Profiles => "profiles",
            Self::WebSearch => "webSearch",
            Self::AppleShortcuts => "appleShortcuts",
        }
    }

    /// Look a command up by its persisted id.
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.id() == id)
    }

    /// Localization key (the English title) for the command title.
    pub fn title_key(self) -> &'static str {
        match self {
            Self::Lock => "Lock Screen",
            Self::Sleep => "Sleep",
            Self::Restart => "Restart…",
            Self::LogOut => "Log Out…",
            Self::ToggleMute => "Toggle Mute",
            Self::ToggleAppearance => "Toggle Appearance",
            Self::LeftHalf => "Left Half",
            Self::RightHalf => "Right Half",
            Self::MaximizeWindow => "Maximize Window",
            Self::CenterWindow => "Center Window",
            Self::RestoreWindow => "Restore Window",
            Self::NextDisplay => "Move to Next Display",
            Self::CalculationHistory => "Calculator History",
            Self::TopHalf => "Top Half",
            Self::BottomHalf => "Bottom Half",
            Self::TopLeftQuarter => "Top Left Quarter",
            Self::TopRightQuarter => "Top Right Quarter",
            Self::BottomLeftQuarter => "Bottom Left Quarter",
            Self::BottomRightQuarter => "Bottom Right Quarter",
            Self::LeftThird => "Left Third",
            Self::CenterThird => "Center Third",
            Self::RightThird => "Right Third",
            Self::LeftTwoThirds => "Left Two Thirds",
            Self::RightTwoThirds => "Right Two Thirds",
            Self::PreviousDisplay => "Move to Previous Display",
            Self::Snippets => "Snippets",
            Self::Scripts => "Script Commands",
            Self::Extensions => "Extensions",
            Self::Clipboard => "Clipboard History",
            Self::Quicklinks => "Quicklinks",
            Self::Emoji => "Emoji & Symbols",
            Self::Settings => "Open Settings",
            Self::Shutdown => "Shut Down…",
            Self::Browse => "Browse Commands",
            Self::Help => "Help",
            Self::Onboarding => "Welcome to Kebloop",
            Self::Profiles => "Profiles",
            Self::WebSearch => "Web Search",
            Self::AppleShortcuts => "Apple Shortcuts",
        }
    }

    /// Localized title.
    pub fn title(self, localizer: &Localizer) -> String {
        localizer.tr(self.title_key())
    }

    /// macOS SF Symbol name (kept for parity and documentation).
    pub fn symbol(self) -> &'static str {
        match self {
            Self::Lock => "lock",
            Self::Sleep => "moon.zzz",
            Self::Restart => "restart",
            Self::LogOut => "rectangle.portrait.and.arrow.right",
            Self::ToggleMute => "speaker.slash",
            Self::ToggleAppearance => "circle.lefthalf.filled",
            Self::LeftHalf => "rectangle.lefthalf.filled",
            Self::RightHalf => "rectangle.righthalf.filled",
            Self::MaximizeWindow => "arrow.up.left.and.arrow.down.right",
            Self::CenterWindow => "rectangle.center.inset.filled",
            Self::RestoreWindow => "arrow.uturn.backward",
            Self::NextDisplay => "display.2",
            Self::CalculationHistory => "clock.arrow.circlepath",
            Self::TopHalf => "rectangle.tophalf.filled",
            Self::BottomHalf => "rectangle.bottomhalf.filled",
            Self::TopLeftQuarter => "rectangle.split.2x2",
            Self::TopRightQuarter => "rectangle.split.2x2",
            Self::BottomLeftQuarter => "rectangle.split.2x2",
            Self::BottomRightQuarter => "rectangle.split.2x2",
            Self::LeftThird => "rectangle.split.3x1",
            Self::CenterThird => "rectangle.split.3x1",
            Self::RightThird => "rectangle.split.3x1",
            Self::LeftTwoThirds => "rectangle.split.3x1",
            Self::RightTwoThirds => "rectangle.split.3x1",
            Self::PreviousDisplay => "display.2",
            Self::Snippets => "text.quote",
            Self::Scripts => "terminal",
            Self::Extensions => "puzzlepiece.extension",
            Self::Clipboard => "doc.on.clipboard",
            Self::Quicklinks => "link",
            Self::Emoji => "face.smiling",
            Self::Settings => "gearshape",
            Self::Shutdown => "power",
            Self::Browse => "square.grid.2x2",
            Self::Help => "questionmark.circle",
            Self::Onboarding => "sparkles",
            Self::Profiles => "person.crop.circle",
            Self::WebSearch => "globe",
            Self::AppleShortcuts => "square.stack.3d.up",
        }
    }

    /// Freedesktop icon-theme name used by the GTK front-end.
    pub fn icon_name(self) -> &'static str {
        match self {
            Self::Lock => "system-lock-screen-symbolic",
            Self::Sleep => "weather-clear-night-symbolic",
            Self::Restart => "system-reboot-symbolic",
            Self::LogOut => "system-log-out-symbolic",
            Self::ToggleMute => "audio-volume-muted-symbolic",
            Self::ToggleAppearance => "preferences-desktop-appearance-symbolic",
            Self::LeftHalf => "view-dual-symbolic",
            Self::RightHalf => "view-dual-symbolic",
            Self::MaximizeWindow => "view-fullscreen-symbolic",
            Self::CenterWindow => "view-restore-symbolic",
            Self::RestoreWindow => "edit-undo-symbolic",
            Self::NextDisplay => "video-display-symbolic",
            Self::CalculationHistory => "document-open-recent-symbolic",
            Self::TopHalf => "view-dual-symbolic",
            Self::BottomHalf => "view-dual-symbolic",
            Self::TopLeftQuarter => "view-grid-symbolic",
            Self::TopRightQuarter => "view-grid-symbolic",
            Self::BottomLeftQuarter => "view-grid-symbolic",
            Self::BottomRightQuarter => "view-grid-symbolic",
            Self::LeftThird => "view-continuous-symbolic",
            Self::CenterThird => "view-continuous-symbolic",
            Self::RightThird => "view-continuous-symbolic",
            Self::LeftTwoThirds => "view-continuous-symbolic",
            Self::RightTwoThirds => "view-continuous-symbolic",
            Self::PreviousDisplay => "video-display-symbolic",
            Self::Snippets => "insert-text-symbolic",
            Self::Scripts => "utilities-terminal-symbolic",
            Self::Extensions => "application-x-addon-symbolic",
            Self::Clipboard => "edit-paste-symbolic",
            Self::Quicklinks => "insert-link-symbolic",
            Self::Emoji => "face-smile-symbolic",
            Self::Settings => "preferences-system-symbolic",
            Self::Shutdown => "system-shutdown-symbolic",
            Self::Browse => "view-grid-symbolic",
            Self::Help => "help-browser-symbolic",
            Self::Onboarding => "starred-symbolic",
            Self::Profiles => "avatar-default-symbolic",
            Self::WebSearch => "web-browser-symbolic",
            Self::AppleShortcuts => "view-grid-symbolic",
        }
    }

    /// English plus Polish search aliases (lower-case).
    pub fn aliases(self) -> &'static [&'static str] {
        match self {
            Self::Lock => &["lock screen", "lock", "zablokuj"],
            Self::Sleep => &["sleep", "uśpij"],
            Self::Restart => &["restart", "reboot", "uruchom ponownie"],
            Self::LogOut => &["log out", "logout", "wyloguj"],
            Self::ToggleMute => &["mute", "unmute", "wycisz"],
            Self::ToggleAppearance => &["dark mode", "light mode", "appearance", "wygląd"],
            Self::LeftHalf => &["left half", "left window", "lewa połowa"],
            Self::RightHalf => &["right half", "right window", "prawa połowa"],
            Self::MaximizeWindow => &["maximize", "window maximize", "maksymalizuj"],
            Self::CenterWindow => &["center window", "center", "wyśrodkuj"],
            Self::RestoreWindow => &["restore window", "restore", "przywróć okno"],
            Self::NextDisplay => &["next display", "move display", "następny ekran"],
            Self::CalculationHistory => &[
                "calculator history",
                "calculation history",
                "historia obliczeń",
            ],
            Self::TopHalf => &["top half", "window top half"],
            Self::BottomHalf => &["bottom half", "window bottom half"],
            Self::TopLeftQuarter => &["top left quarter", "window top left quarter"],
            Self::TopRightQuarter => &["top right quarter", "window top right quarter"],
            Self::BottomLeftQuarter => &["bottom left quarter", "window bottom left quarter"],
            Self::BottomRightQuarter => &["bottom right quarter", "window bottom right quarter"],
            Self::LeftThird => &["left third", "window left third"],
            Self::CenterThird => &["center third", "window center third"],
            Self::RightThird => &["right third", "window right third"],
            Self::LeftTwoThirds => &["left two thirds", "window left two thirds"],
            Self::RightTwoThirds => &["right two thirds", "window right two thirds"],
            Self::PreviousDisplay => &[
                "move to previous display",
                "window move to previous display",
            ],
            Self::Snippets => &["snippets", "text", "fragmenty", "tekst"],
            Self::Scripts => &["scripts", "script commands", "skrypty"],
            Self::Extensions => &["extensions", "rozszerzenia"],
            Self::Clipboard => &["schowek", "historia", "clipboard", "history", "paste"],
            Self::Quicklinks => &["linki", "zakładki", "quicklinks", "links", "bookmarks"],
            Self::Emoji => &[
                "emotikony",
                "symbole",
                "emoji",
                "symbols",
                "emoji & symbols",
            ],
            Self::Settings => &[
                "ustawienia",
                "preferencje",
                "settings",
                "preferences",
                "open settings",
            ],
            Self::Shutdown => &[
                "wyłącz",
                "zamknij komputer",
                "shutdown",
                "shut down",
                "power off",
                "turn off",
            ],
            Self::Browse => &[
                "polecenia",
                "przeglądaj",
                "browse",
                "commands",
                "all commands",
            ],
            Self::Help => &["pomoc", "poradnik", "help", "guide"],
            Self::Onboarding => &["witaj", "samouczek", "welcome", "onboarding", "tutorial"],
            Self::Profiles => &["profile", "przełącz profil", "profiles", "switch profile"],
            Self::WebSearch => &["szukaj w sieci", "wyszukiwarka", "web search", "search web"],
            Self::AppleShortcuts => &[
                "skróty",
                "automatyzacja",
                "apple shortcuts",
                "shortcuts",
                "automation",
            ],
        }
    }

    /// `true` for commands whose upstream implementation is macOS-only (AppleScript, Carbon,
    /// Accessibility): Apple Shortcuts, the system actions and the window-management actions.
    /// They keep their stable ids and titles so imported configurations (shortcut bindings,
    /// pins) stay valid, but have no Linux implementation in this crate.
    pub fn is_macos_only(self) -> bool {
        matches!(self, Self::AppleShortcuts)
            || self.system_action().is_some()
            || self.window_action().is_some()
    }

    /// Whether the command can run on Linux with no optional integrations: everything except the
    /// macOS-only commands. `appleShortcuts` is kept only so macOS configurations import
    /// cleanly; it is hidden in the launcher.
    pub fn is_available(self) -> bool {
        self.is_available_with(CommandSupport::default())
    }

    /// Availability when the platform layer provides the optional integrations in `support`.
    pub fn is_available_with(self, support: CommandSupport) -> bool {
        if self == Self::AppleShortcuts {
            return false;
        }
        if self.system_action().is_some() {
            return support.system_actions;
        }
        if self.window_action().is_some() {
            return support.window_management;
        }
        true
    }

    /// Commands that appear in the launcher on Linux.
    pub fn available() -> impl Iterator<Item = LauncherCommand> {
        Self::ALL.into_iter().filter(|c| c.is_available())
    }

    /// Commands that appear when the platform layer provides `support`.
    pub fn available_with(support: CommandSupport) -> impl Iterator<Item = LauncherCommand> {
        Self::ALL
            .into_iter()
            .filter(move |c| c.is_available_with(support))
    }

    /// The system action this command performs, if it is one.
    pub fn system_action(self) -> Option<SystemAction> {
        Some(match self {
            Self::Lock => SystemAction::Lock,
            Self::Sleep => SystemAction::Sleep,
            Self::Restart => SystemAction::Restart,
            Self::LogOut => SystemAction::LogOut,
            Self::ToggleMute => SystemAction::ToggleMute,
            Self::ToggleAppearance => SystemAction::ToggleAppearance,
            _ => return None,
        })
    }

    /// The window arrangement this command performs, if it is one.
    pub fn window_action(self) -> Option<WindowAction> {
        Some(match self {
            Self::LeftHalf => WindowAction::LeftHalf,
            Self::RightHalf => WindowAction::RightHalf,
            Self::MaximizeWindow => WindowAction::Maximize,
            Self::CenterWindow => WindowAction::Center,
            Self::RestoreWindow => WindowAction::Restore,
            Self::NextDisplay => WindowAction::NextDisplay,
            Self::TopHalf => WindowAction::TopHalf,
            Self::BottomHalf => WindowAction::BottomHalf,
            Self::TopLeftQuarter => WindowAction::TopLeftQuarter,
            Self::TopRightQuarter => WindowAction::TopRightQuarter,
            Self::BottomLeftQuarter => WindowAction::BottomLeftQuarter,
            Self::BottomRightQuarter => WindowAction::BottomRightQuarter,
            Self::LeftThird => WindowAction::LeftThird,
            Self::CenterThird => WindowAction::CenterThird,
            Self::RightThird => WindowAction::RightThird,
            Self::LeftTwoThirds => WindowAction::LeftTwoThirds,
            Self::RightTwoThirds => WindowAction::RightTwoThirds,
            Self::PreviousDisplay => WindowAction::PreviousDisplay,
            _ => return None,
        })
    }

    /// The command that performs `action`.
    pub fn for_window_action(action: WindowAction) -> Option<LauncherCommand> {
        Self::ALL
            .into_iter()
            .find(|command| command.window_action() == Some(action))
    }
}

/// Optional integrations the platform layer can provide, which enable the commands that are
/// macOS-only upstream. The default (nothing provided) is what this crate can run on its own.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CommandSupport {
    /// Lock, sleep, restart, log out, mute and appearance can be performed.
    pub system_actions: bool,
    /// Windows of other applications can be arranged.
    pub window_management: bool,
}

/// Actions of the system commands (`lock` ... `toggleAppearance`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SystemAction {
    /// Lock the screen.
    Lock,
    /// Sleep.
    Sleep,
    /// Restart (the system confirms unsaved work).
    Restart,
    /// Log out (the system confirms unsaved work).
    LogOut,
    /// Toggle the output mute state.
    ToggleMute,
    /// Toggle between light and dark appearance.
    ToggleAppearance,
}

/// `true` when `id` names a user command (`snippet:`, `script:`, `extension:`, `apple:`, `link:`).
pub fn is_user_command_id(id: &str) -> bool {
    ["snippet:", "script:", "extension:", "apple:", "link:"]
        .iter()
        .any(|prefix| id.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn ids_are_unique_and_round_trip() {
        let ids: HashSet<_> = LauncherCommand::ALL.iter().map(|c| c.id()).collect();
        assert_eq!(ids.len(), LauncherCommand::ALL.len());
        for command in LauncherCommand::ALL {
            assert_eq!(LauncherCommand::from_id(command.id()), Some(command));
            assert!(!command.title_key().is_empty());
            assert!(!command.symbol().is_empty());
            assert!(!command.icon_name().is_empty());
            assert!(!command.aliases().is_empty());
        }
    }

    #[test]
    fn persisted_ids_are_stable() {
        let ids: Vec<_> = LauncherCommand::ALL.iter().map(|c| c.id()).collect();
        assert_eq!(
            ids,
            [
                "lock",
                "sleep",
                "restart",
                "logOut",
                "toggleMute",
                "toggleAppearance",
                "leftHalf",
                "rightHalf",
                "maximizeWindow",
                "centerWindow",
                "restoreWindow",
                "nextDisplay",
                "calculationHistory",
                "topHalf",
                "bottomHalf",
                "topLeftQuarter",
                "topRightQuarter",
                "bottomLeftQuarter",
                "bottomRightQuarter",
                "leftThird",
                "centerThird",
                "rightThird",
                "leftTwoThirds",
                "rightTwoThirds",
                "previousDisplay",
                "snippets",
                "scripts",
                "extensions",
                "clipboard",
                "quicklinks",
                "emoji",
                "settings",
                "shutdown",
                "browse",
                "help",
                "onboarding",
                "profiles",
                "webSearch",
                "appleShortcuts"
            ]
        );
    }

    #[test]
    fn macos_only_commands_are_unavailable_unless_the_platform_provides_them() {
        assert!(LauncherCommand::AppleShortcuts.is_macos_only());
        assert!(!LauncherCommand::AppleShortcuts.is_available());
        assert_eq!(
            LauncherCommand::available().count(),
            14,
            "the 13 original commands (without Apple Shortcuts) and the calculator history"
        );
        assert!(LauncherCommand::CalculationHistory.is_available());
        let system = LauncherCommand::ALL
            .iter()
            .filter(|c| c.system_action().is_some())
            .count();
        let windows = LauncherCommand::ALL
            .iter()
            .filter(|c| c.window_action().is_some())
            .count();
        assert_eq!((system, windows), (6, 18));
        assert!(LauncherCommand::Lock.is_macos_only());
        assert!(!LauncherCommand::Lock.is_available());
        assert!(!LauncherCommand::LeftHalf.is_available());

        let windows_only = CommandSupport {
            window_management: true,
            ..CommandSupport::default()
        };
        assert!(LauncherCommand::LeftHalf.is_available_with(windows_only));
        assert!(!LauncherCommand::Lock.is_available_with(windows_only));
        assert_eq!(
            LauncherCommand::available_with(windows_only).count(),
            14 + 18
        );
        assert!(
            !LauncherCommand::AppleShortcuts.is_available_with(CommandSupport {
                system_actions: true,
                window_management: true
            })
        );
    }

    #[test]
    fn window_commands_map_one_to_one_onto_window_actions() {
        for action in WindowAction::ALL {
            let command = LauncherCommand::for_window_action(action).expect("a command");
            assert_eq!(command.window_action(), Some(action));
        }
        assert_eq!(WindowAction::ALL.len(), 18);
    }

    #[test]
    fn aliases_keep_english_and_polish_names() {
        assert!(LauncherCommand::Settings.aliases().contains(&"settings"));
        assert!(LauncherCommand::Settings.aliases().contains(&"ustawienia"));
        assert!(LauncherCommand::Clipboard.aliases().contains(&"clipboard"));
        assert!(LauncherCommand::Clipboard.aliases().contains(&"schowek"));
    }
}
