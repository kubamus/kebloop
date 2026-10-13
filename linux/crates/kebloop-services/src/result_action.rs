//! Result actions (the Ctrl+K menu): stable identifiers, titles and which actions a kind of
//! result offers. Port of `Sources/Kebloop/ResultAction.swift`, plus the pinned-result
//! accessibility label of the launcher rows.
//!
//! The GTK front end keeps its own richer action type (desktop and extension actions); this
//! module is the shared model of the stable part.

use crate::l10n::Localizer;

/// Stable identifiers shared by launcher menus and command integrations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResultActionId {
    /// The primary action.
    Open,
    /// Remove every accepted calculation.
    ClearCalculationHistory,
    /// Copy without pasting.
    Copy,
    /// Paste without formatting.
    PastePlainText,
    /// Ask a running application to quit.
    QuitApplication,
    /// Force a running application to quit (after confirmation).
    ForceQuitApplication,
    /// Quit and relaunch a running application.
    RestartApplication,
    /// Hide a running application.
    HideApplication,
    /// Show in the file manager.
    Reveal,
    /// Copy the path.
    CopyPath,
    /// Choose an application to open with.
    OpenWith,
    /// Edit a quicklink.
    EditQuicklink,
    /// Pin to favorites.
    Pin,
    /// Remove from favorites.
    Unpin,
    /// Run with extra input.
    RunWithInput,
    /// Delete a clipboard history entry.
    RemoveClipboardItem,
}

impl ResultActionId {
    /// Every action, in declaration order.
    pub const ALL: [ResultActionId; 16] = [
        Self::Open,
        Self::ClearCalculationHistory,
        Self::Copy,
        Self::PastePlainText,
        Self::QuitApplication,
        Self::ForceQuitApplication,
        Self::RestartApplication,
        Self::HideApplication,
        Self::Reveal,
        Self::CopyPath,
        Self::OpenWith,
        Self::EditQuicklink,
        Self::Pin,
        Self::Unpin,
        Self::RunWithInput,
        Self::RemoveClipboardItem,
    ];

    /// The Swift raw value (`open`, `pastePlainText`, ...).
    pub fn id(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::ClearCalculationHistory => "clearCalculationHistory",
            Self::Copy => "copy",
            Self::PastePlainText => "pastePlainText",
            Self::QuitApplication => "quitApplication",
            Self::ForceQuitApplication => "forceQuitApplication",
            Self::RestartApplication => "restartApplication",
            Self::HideApplication => "hideApplication",
            Self::Reveal => "reveal",
            Self::CopyPath => "copyPath",
            Self::OpenWith => "openWith",
            Self::EditQuicklink => "editQuicklink",
            Self::Pin => "pin",
            Self::Unpin => "unpin",
            Self::RunWithInput => "runWithInput",
            Self::RemoveClipboardItem => "removeClipboardItem",
        }
    }

    /// Looks an action up by its stable id.
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|action| action.id() == id)
    }

    /// Localization key (the English title). `Reveal` keeps the macOS wording; the Linux front
    /// end shows "Open Containing Folder" instead.
    pub fn title_key(self) -> &'static str {
        match self {
            Self::ClearCalculationHistory => "Clear Calculator History…",
            Self::Copy => "Copy",
            Self::PastePlainText => "Paste as Plain Text",
            Self::QuitApplication => "Quit Application",
            Self::ForceQuitApplication => "Force Quit Application…",
            Self::RestartApplication => "Restart Application",
            Self::HideApplication => "Hide Application",
            Self::RemoveClipboardItem => "Remove Item",
            Self::Open => "Open",
            Self::RunWithInput => "Run with Input…",
            Self::Reveal => "Reveal in Finder",
            Self::CopyPath => "Copy Path",
            Self::OpenWith => "Open With…",
            Self::EditQuicklink => "Edit Quicklink…",
            Self::Pin => "Pin to Favorites",
            Self::Unpin => "Unpin from Favorites",
        }
    }

    /// Localized title.
    pub fn title(self, localizer: &Localizer) -> String {
        localizer.tr(self.title_key())
    }
}

/// What kind of result a menu is opened for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResultKind {
    /// An application.
    Application,
    /// A file or folder.
    File,
    /// A quicklink.
    Quicklink,
    /// A built-in command.
    Command,
    /// An Apple Shortcut (macOS only).
    AppleShortcut,
    /// A clipboard history entry.
    Clipboard,
    /// Anything else.
    Other,
}

/// Facts about a result that decide its actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActionFacts {
    /// The result is in the favorites (or the clipboard item is pinned).
    pub pinned: bool,
    /// The application is running.
    pub running: bool,
    /// The clipboard entry has a plain-text representation.
    pub has_plain_text: bool,
}

impl Default for ActionFacts {
    fn default() -> Self {
        Self {
            pinned: false,
            running: false,
            has_plain_text: true,
        }
    }
}

/// The actions offered for a `kind` of result, primary action first.
pub fn available(kind: ResultKind, facts: ActionFacts) -> Vec<ResultActionId> {
    use ResultActionId as A;
    let pin = if facts.pinned { A::Unpin } else { A::Pin };
    if kind == ResultKind::Clipboard {
        let mut actions = vec![A::Open, A::Copy];
        if facts.has_plain_text {
            actions.push(A::PastePlainText);
        }
        actions.extend([pin, A::RemoveClipboardItem]);
        return actions;
    }
    let mut actions = vec![A::Open];
    match kind {
        ResultKind::File => actions.extend([A::Reveal, A::CopyPath, A::OpenWith]),
        ResultKind::Application => {
            if facts.running {
                actions.extend([
                    A::HideApplication,
                    A::QuitApplication,
                    A::ForceQuitApplication,
                    A::RestartApplication,
                ]);
            }
            actions.extend([A::Reveal, A::CopyPath]);
        }
        ResultKind::Quicklink => actions.push(A::EditQuicklink),
        ResultKind::AppleShortcut => actions.push(A::RunWithInput),
        ResultKind::Clipboard | ResultKind::Command | ResultKind::Other => {}
    }
    if kind != ResultKind::Other && kind != ResultKind::AppleShortcut {
        actions.push(pin);
    }
    actions
}

/// The actions of the calculator-history browser: the primary action and clearing.
pub const CALCULATION_HISTORY_ACTIONS: [ResultActionId; 2] = [
    ResultActionId::Open,
    ResultActionId::ClearCalculationHistory,
];

/// The accessibility label of a launcher row: `title, subtitle, action`, followed by
/// "Pinned to Favorites" for pinned results, so the pin badge is not visual-only.
pub fn row_accessibility_label(
    title: &str,
    subtitle: &str,
    action: &str,
    pinned: bool,
    localizer: &Localizer,
) -> String {
    let mut label = format!("{title}, {subtitle}, {action}");
    if pinned {
        label.push_str(", ");
        label.push_str(&localizer.tr(PINNED_BADGE_KEY));
    }
    label
}

/// Localization key of the pin badge's tooltip and accessibility text.
pub const PINNED_BADGE_KEY: &str = "Pinned to Favorites";

#[cfg(test)]
mod tests {
    use super::ResultActionId as A;
    use super::*;

    fn facts(pinned: bool, running: bool, has_plain_text: bool) -> ActionFacts {
        ActionFacts {
            pinned,
            running,
            has_plain_text,
        }
    }

    #[test]
    fn ids_round_trip_and_are_unique() {
        let ids: std::collections::HashSet<_> = A::ALL.iter().map(|a| a.id()).collect();
        assert_eq!(ids.len(), A::ALL.len());
        for action in A::ALL {
            assert_eq!(A::from_id(action.id()), Some(action));
            assert!(!action.title_key().is_empty());
        }
        assert_eq!(A::from_id("nonsense"), None);
    }

    #[test]
    fn clipboard_actions_include_copy_plain_paste_and_remove_for_pinned_and_unpinned_items() {
        assert_eq!(
            available(ResultKind::Clipboard, ActionFacts::default()),
            [
                A::Open,
                A::Copy,
                A::PastePlainText,
                A::Pin,
                A::RemoveClipboardItem
            ]
        );
        assert_eq!(
            available(ResultKind::Clipboard, facts(true, false, true)),
            [
                A::Open,
                A::Copy,
                A::PastePlainText,
                A::Unpin,
                A::RemoveClipboardItem
            ]
        );
        for kind in [
            ResultKind::File,
            ResultKind::Application,
            ResultKind::Quicklink,
            ResultKind::Command,
            ResultKind::AppleShortcut,
            ResultKind::Other,
        ] {
            assert!(!available(kind, ActionFacts::default()).contains(&A::RemoveClipboardItem));
        }
    }

    #[test]
    fn files_and_applications_offer_reveal_and_pin() {
        assert_eq!(
            available(ResultKind::File, ActionFacts::default()),
            [A::Open, A::Reveal, A::CopyPath, A::OpenWith, A::Pin]
        );
        assert_eq!(
            available(ResultKind::Application, ActionFacts::default()),
            [A::Open, A::Reveal, A::CopyPath, A::Pin]
        );
    }

    #[test]
    fn app_controls_only_appear_for_running_applications_and_plain_paste_requires_text() {
        assert_eq!(
            available(ResultKind::Application, facts(false, true, true)),
            [
                A::Open,
                A::HideApplication,
                A::QuitApplication,
                A::ForceQuitApplication,
                A::RestartApplication,
                A::Reveal,
                A::CopyPath,
                A::Pin
            ]
        );
        assert!(
            !available(ResultKind::Clipboard, facts(false, false, false))
                .contains(&A::PastePlainText)
        );
    }

    #[test]
    fn editable_quicklinks_and_supported_pins() {
        assert_eq!(
            available(ResultKind::Quicklink, facts(true, false, true)),
            [A::Open, A::EditQuicklink, A::Unpin]
        );
        assert_eq!(
            available(ResultKind::Command, ActionFacts::default()),
            [A::Open, A::Pin]
        );
        assert_eq!(
            available(ResultKind::Other, ActionFacts::default()),
            [A::Open]
        );
        assert_eq!(
            available(ResultKind::AppleShortcut, ActionFacts::default()),
            [A::Open, A::RunWithInput]
        );
    }

    #[test]
    fn removing_a_clipboard_item_by_keyboard_navigation_finds_its_row_by_identifier() {
        // MilestoneIntegrationTests no longer assumes a fixed row: the remove action is located
        // in the menu, because Copy and Paste as Plain Text precede Pin.
        for pinned in [false, true] {
            let actions = available(ResultKind::Clipboard, facts(pinned, false, true));
            assert_eq!(
                actions.iter().position(|a| *a == A::RemoveClipboardItem),
                Some(4)
            );
        }
        let without_text = available(ResultKind::Clipboard, facts(false, false, false));
        assert_eq!(
            without_text
                .iter()
                .position(|a| *a == A::RemoveClipboardItem),
            Some(3)
        );
    }

    #[test]
    fn history_browser_offers_open_and_clear_only() {
        assert_eq!(
            CALCULATION_HISTORY_ACTIONS,
            [A::Open, A::ClearCalculationHistory]
        );
    }
}
