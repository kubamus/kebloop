//! Key handling decisions, free of GTK so they can be unit tested. The window adapts GDK key
//! events into [`Key`] + [`Mods`] and executes the returned [`KeyAction`].

/// Keys the launcher cares about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    /// Escape.
    Escape,
    /// Return / keypad Enter.
    Return,
    /// Tab.
    Tab,
    /// Cursor up.
    Up,
    /// Cursor down.
    Down,
    /// Cursor left.
    Left,
    /// Cursor right.
    Right,
    /// Page up.
    PageUp,
    /// Page down.
    PageDown,
    /// Home.
    Home,
    /// End.
    End,
    /// Backspace.
    BackSpace,
    /// A printable key, lower-cased.
    Char(char),
}

/// Modifier state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Mods {
    /// Control.
    pub ctrl: bool,
    /// Shift.
    pub shift: bool,
    /// Alt.
    pub alt: bool,
    /// Super / logo.
    pub logo: bool,
}

impl Mods {
    /// No modifier at all.
    pub const NONE: Mods = Mods {
        ctrl: false,
        shift: false,
        alt: false,
        logo: false,
    };
    /// Control only.
    pub const CTRL: Mods = Mods {
        ctrl: true,
        ..Mods::NONE
    };
    /// Control and Shift.
    pub const CTRL_SHIFT: Mods = Mods {
        ctrl: true,
        shift: true,
        ..Mods::NONE
    };
    /// Alt only.
    pub const ALT: Mods = Mods {
        alt: true,
        ..Mods::NONE
    };

    fn is_none(self) -> bool {
        self == Self::NONE
    }
}

/// What the visible view is, as far as keys are concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewKind {
    /// Root search.
    Root,
    /// A result list other than root.
    List,
    /// The clipboard history list (has remove shortcuts).
    Clipboard,
    /// The emoji grid.
    Emoji,
    /// A page with its own widgets (snippet form, preview).
    Form,
}

/// Facts the decision depends on.
#[derive(Debug, Clone, Copy)]
pub struct KeyState {
    /// The visible view.
    pub view: ViewKind,
    /// The action menu is open.
    pub actions_open: bool,
    /// The search text is empty (whitespace only counts as empty).
    pub query_empty: bool,
    /// There is at least one result or action target.
    pub has_results: bool,
    /// The input method is composing text (preedit string showing).
    pub preedit: bool,
}

/// What to do with a key press.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    /// Let the focused widget handle it.
    Propagate,
    /// Hide the launcher.
    Hide,
    /// Return to root search.
    Back,
    /// Hide every Kebloop window (Ctrl+Q).
    HideUi,
    /// Open settings.
    OpenSettings,
    /// Move the selection by this many rows.
    Move(isize),
    /// Move by a page (sign gives the direction).
    Page(isize),
    /// Select the first result.
    First,
    /// Select the last result.
    Last,
    /// Run the selected result.
    Activate,
    /// Open or close the action menu.
    ToggleActions,
    /// Hand the query to the AI provider.
    AskAi,
    /// Remove the selected clipboard entry.
    RemoveClipboardItem,
    /// Ask to clear the whole clipboard history.
    ClearClipboard,
    /// Copy the selected clipboard entry or emoji without pasting (Ctrl+Return).
    CopySelected,
    /// Paste the selected clipboard entry as plain text (Ctrl+Shift+Return).
    PastePlainSelected,
    /// Run the n-th result (0-based).
    QuickSelect(usize),
    /// Move the action menu selection.
    ActionsMove(isize),
    /// Run the selected action.
    ActionsActivate,
    /// Close the action menu.
    ActionsClose,
    /// Move within the emoji grid by columns/rows: (dx, dy).
    GridMove(isize, isize),
    /// Swallow the key.
    Consume,
}

/// Decide what a key press means.
pub fn decide(key: Key, mods: Mods, state: KeyState) -> KeyAction {
    let ctrl_only = mods == Mods::CTRL;
    let plain = mods.is_none();

    // Global chords first.
    if ctrl_only && key == Key::Char('q') {
        return KeyAction::HideUi;
    }
    if ctrl_only && key == Key::Char(',') {
        return KeyAction::OpenSettings;
    }

    // While an input method composes text, Return commits and Escape cancels the composition;
    // navigation keys pick candidates. None of them may drive the launcher.
    if state.preedit
        && matches!(
            key,
            Key::Return
                | Key::Escape
                | Key::Tab
                | Key::Up
                | Key::Down
                | Key::PageUp
                | Key::PageDown
        )
    {
        return KeyAction::Propagate;
    }

    if state.actions_open {
        return match key {
            Key::Escape if plain => KeyAction::ActionsClose,
            Key::Char('k') if ctrl_only => KeyAction::ActionsClose,
            Key::Up if plain => KeyAction::ActionsMove(-1),
            Key::Down if plain => KeyAction::ActionsMove(1),
            Key::Return if plain || mods == Mods::CTRL => KeyAction::ActionsActivate,
            Key::Tab => KeyAction::Consume,
            _ => KeyAction::Propagate,
        };
    }

    if key == Key::Escape && plain {
        return if state.view == ViewKind::Root {
            KeyAction::Hide
        } else {
            KeyAction::Back
        };
    }
    if state.view == ViewKind::Form {
        return KeyAction::Propagate;
    }
    if key == Key::Char('k') && ctrl_only {
        return if state.has_results {
            KeyAction::ToggleActions
        } else {
            KeyAction::Consume
        };
    }
    if state.view == ViewKind::Clipboard && key == Key::BackSpace {
        if mods == Mods::CTRL_SHIFT {
            return KeyAction::ClearClipboard;
        }
        if ctrl_only {
            return KeyAction::RemoveClipboardItem;
        }
    }
    if state.view == ViewKind::Clipboard && key == Key::Return && mods.ctrl && state.has_results {
        return if mods.shift {
            KeyAction::PastePlainSelected
        } else {
            KeyAction::CopySelected
        };
    }
    if key == Key::Tab && plain {
        return if state.view == ViewKind::Root && !state.query_empty {
            KeyAction::AskAi
        } else {
            KeyAction::Consume
        };
    }
    if state.view == ViewKind::Emoji {
        let horizontal_ok = state.query_empty || mods == Mods::ALT;
        return match key {
            Key::Left if horizontal_ok && (plain || mods == Mods::ALT) => {
                KeyAction::GridMove(-1, 0)
            }
            Key::Right if horizontal_ok && (plain || mods == Mods::ALT) => {
                KeyAction::GridMove(1, 0)
            }
            Key::Up if plain => KeyAction::GridMove(0, -1),
            Key::Down if plain => KeyAction::GridMove(0, 1),
            Key::PageUp if plain => KeyAction::Page(-1),
            Key::PageDown if plain => KeyAction::Page(1),
            Key::Return if mods.ctrl => KeyAction::CopySelected,
            Key::Return if mods.is_none() || mods.shift => KeyAction::Activate,
            _ => KeyAction::Propagate,
        };
    }
    match key {
        Key::Up if plain => KeyAction::Move(-1),
        Key::Down if plain => KeyAction::Move(1),
        Key::PageUp if plain => KeyAction::Page(-1),
        Key::PageDown if plain => KeyAction::Page(1),
        Key::Home if plain && state.query_empty => KeyAction::First,
        Key::End if plain && state.query_empty => KeyAction::Last,
        Key::Home if ctrl_only => KeyAction::First,
        Key::End if ctrl_only => KeyAction::Last,
        Key::Return if plain || mods.shift || mods == Mods::CTRL => KeyAction::Activate,
        Key::Char(c) if ctrl_only && ('1'..='9').contains(&c) => {
            KeyAction::QuickSelect(c as usize - '1' as usize)
        }
        _ => KeyAction::Propagate,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(view: ViewKind) -> KeyState {
        KeyState {
            view,
            actions_open: false,
            query_empty: false,
            has_results: true,
            preedit: false,
        }
    }

    #[test]
    fn composing_text_keeps_return_and_escape_for_the_input_method() {
        let mut s = state(ViewKind::Root);
        s.preedit = true;
        for key in [Key::Return, Key::Escape, Key::Down, Key::Tab] {
            assert_eq!(decide(key, Mods::NONE, s), KeyAction::Propagate);
        }
        // Global chords still work.
        assert_eq!(
            decide(Key::Char(','), Mods::CTRL, s),
            KeyAction::OpenSettings
        );
        s.preedit = false;
        assert_eq!(decide(Key::Return, Mods::NONE, s), KeyAction::Activate);
        assert_eq!(decide(Key::Escape, Mods::NONE, s), KeyAction::Hide);
    }

    #[test]
    fn escape_hides_from_root_and_goes_back_elsewhere() {
        assert_eq!(
            decide(Key::Escape, Mods::NONE, state(ViewKind::Root)),
            KeyAction::Hide
        );
        for view in [
            ViewKind::List,
            ViewKind::Clipboard,
            ViewKind::Emoji,
            ViewKind::Form,
        ] {
            assert_eq!(
                decide(Key::Escape, Mods::NONE, state(view)),
                KeyAction::Back
            );
        }
    }

    #[test]
    fn arrows_move_the_selection_and_return_activates() {
        let s = state(ViewKind::Root);
        assert_eq!(decide(Key::Down, Mods::NONE, s), KeyAction::Move(1));
        assert_eq!(decide(Key::Up, Mods::NONE, s), KeyAction::Move(-1));
        assert_eq!(decide(Key::PageDown, Mods::NONE, s), KeyAction::Page(1));
        assert_eq!(decide(Key::Return, Mods::NONE, s), KeyAction::Activate);
    }

    #[test]
    fn home_and_end_move_the_text_cursor_unless_the_query_is_empty() {
        let mut s = state(ViewKind::Root);
        assert_eq!(decide(Key::Home, Mods::NONE, s), KeyAction::Propagate);
        assert_eq!(decide(Key::Home, Mods::CTRL, s), KeyAction::First);
        s.query_empty = true;
        assert_eq!(decide(Key::Home, Mods::NONE, s), KeyAction::First);
        assert_eq!(decide(Key::End, Mods::NONE, s), KeyAction::Last);
    }

    #[test]
    fn tab_hands_off_to_ai_only_for_a_non_blank_root_query() {
        let mut s = state(ViewKind::Root);
        assert_eq!(decide(Key::Tab, Mods::NONE, s), KeyAction::AskAi);
        s.query_empty = true;
        assert_eq!(decide(Key::Tab, Mods::NONE, s), KeyAction::Consume);
        s.query_empty = false;
        assert_eq!(
            decide(Key::Tab, Mods::NONE, state(ViewKind::List)),
            KeyAction::Consume
        );
        assert_eq!(decide(Key::Tab, Mods::CTRL, s), KeyAction::Propagate);
    }

    #[test]
    fn ctrl_k_toggles_the_action_menu_only_with_results() {
        let mut s = state(ViewKind::Root);
        assert_eq!(
            decide(Key::Char('k'), Mods::CTRL, s),
            KeyAction::ToggleActions
        );
        s.has_results = false;
        assert_eq!(decide(Key::Char('k'), Mods::CTRL, s), KeyAction::Consume);
    }

    #[test]
    fn open_action_menu_captures_navigation_keys() {
        let mut s = state(ViewKind::Root);
        s.actions_open = true;
        assert_eq!(decide(Key::Down, Mods::NONE, s), KeyAction::ActionsMove(1));
        assert_eq!(decide(Key::Up, Mods::NONE, s), KeyAction::ActionsMove(-1));
        assert_eq!(
            decide(Key::Return, Mods::NONE, s),
            KeyAction::ActionsActivate
        );
        assert_eq!(decide(Key::Escape, Mods::NONE, s), KeyAction::ActionsClose);
        assert_eq!(
            decide(Key::Char('k'), Mods::CTRL, s),
            KeyAction::ActionsClose
        );
        assert_eq!(decide(Key::Char('a'), Mods::NONE, s), KeyAction::Propagate);
    }

    #[test]
    fn clipboard_remove_shortcuts() {
        let s = state(ViewKind::Clipboard);
        assert_eq!(
            decide(Key::BackSpace, Mods::CTRL, s),
            KeyAction::RemoveClipboardItem
        );
        assert_eq!(
            decide(Key::BackSpace, Mods::CTRL_SHIFT, s),
            KeyAction::ClearClipboard
        );
        assert_eq!(decide(Key::BackSpace, Mods::NONE, s), KeyAction::Propagate);
        assert_eq!(
            decide(Key::BackSpace, Mods::CTRL, state(ViewKind::Root)),
            KeyAction::Propagate
        );
    }

    #[test]
    fn clipboard_return_pastes_and_ctrl_return_copies() {
        let s = state(ViewKind::Clipboard);
        assert_eq!(decide(Key::Return, Mods::NONE, s), KeyAction::Activate);
        assert_eq!(decide(Key::Return, Mods::CTRL, s), KeyAction::CopySelected);
        assert_eq!(
            decide(Key::Return, Mods::CTRL_SHIFT, s),
            KeyAction::PastePlainSelected
        );
        let mut empty = s;
        empty.has_results = false;
        assert_eq!(decide(Key::Return, Mods::CTRL, empty), KeyAction::Activate);
        // Other lists keep Ctrl+Return as a plain activation.
        assert_eq!(
            decide(Key::Return, Mods::CTRL, state(ViewKind::Root)),
            KeyAction::Activate
        );
    }

    #[test]
    fn emoji_return_pastes_and_ctrl_return_copies() {
        let s = state(ViewKind::Emoji);
        assert_eq!(decide(Key::Return, Mods::NONE, s), KeyAction::Activate);
        assert_eq!(decide(Key::Return, Mods::CTRL, s), KeyAction::CopySelected);
        assert_eq!(
            decide(Key::Return, Mods::CTRL_SHIFT, s),
            KeyAction::CopySelected
        );
    }

    #[test]
    fn global_chords_work_everywhere() {
        for view in [
            ViewKind::Root,
            ViewKind::List,
            ViewKind::Emoji,
            ViewKind::Form,
        ] {
            assert_eq!(
                decide(Key::Char('q'), Mods::CTRL, state(view)),
                KeyAction::HideUi
            );
            assert_eq!(
                decide(Key::Char(','), Mods::CTRL, state(view)),
                KeyAction::OpenSettings
            );
        }
    }

    #[test]
    fn quick_select_uses_ctrl_digits() {
        let s = state(ViewKind::Root);
        assert_eq!(
            decide(Key::Char('1'), Mods::CTRL, s),
            KeyAction::QuickSelect(0)
        );
        assert_eq!(
            decide(Key::Char('9'), Mods::CTRL, s),
            KeyAction::QuickSelect(8)
        );
        assert_eq!(decide(Key::Char('1'), Mods::NONE, s), KeyAction::Propagate);
    }

    #[test]
    fn emoji_grid_arrows() {
        let mut s = state(ViewKind::Emoji);
        s.query_empty = true;
        assert_eq!(decide(Key::Left, Mods::NONE, s), KeyAction::GridMove(-1, 0));
        assert_eq!(decide(Key::Down, Mods::NONE, s), KeyAction::GridMove(0, 1));
        s.query_empty = false;
        assert_eq!(decide(Key::Left, Mods::NONE, s), KeyAction::Propagate);
        assert_eq!(decide(Key::Left, Mods::ALT, s), KeyAction::GridMove(-1, 0));
        assert_eq!(decide(Key::Return, Mods::NONE, s), KeyAction::Activate);
    }

    #[test]
    fn forms_leave_typing_alone() {
        let s = state(ViewKind::Form);
        assert_eq!(decide(Key::Down, Mods::NONE, s), KeyAction::Propagate);
        assert_eq!(decide(Key::Return, Mods::NONE, s), KeyAction::Propagate);
        assert_eq!(decide(Key::Tab, Mods::NONE, s), KeyAction::Propagate);
    }
}
