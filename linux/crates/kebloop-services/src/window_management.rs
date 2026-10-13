//! Window-management model: the arrangement actions, their geometry and the suggested
//! shortcuts. Port of the platform-neutral parts of `WindowManagementService.swift` and
//! `WindowManagementSettingsView.swift`.
//!
//! Arranging another application's window needs compositor support that Wayland does not
//! offer generically, so nothing here moves a window: the platform layer decides whether the
//! commands are available (see [`crate::commands::CommandSupport`]) and applies the frames
//! computed here. Coordinates are in any consistent top-left-origin pixel space of the usable
//! (visible) area of a display.

use crate::accelerator::Accelerator;
use crate::commands::LauncherCommand;
use crate::config::PortableSettings;
use crate::shortcuts::{ShortcutOwner, assign_command_shortcut};

/// A window arrangement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WindowAction {
    /// Left 1/2.
    LeftHalf,
    /// Right 1/2.
    RightHalf,
    /// Top 1/2.
    TopHalf,
    /// Bottom 1/2.
    BottomHalf,
    /// Top-left 1/4.
    TopLeftQuarter,
    /// Top-right 1/4.
    TopRightQuarter,
    /// Bottom-left 1/4.
    BottomLeftQuarter,
    /// Bottom-right 1/4.
    BottomRightQuarter,
    /// Left 1/3.
    LeftThird,
    /// Middle 1/3.
    CenterThird,
    /// Right 1/3.
    RightThird,
    /// Left 2/3.
    LeftTwoThirds,
    /// Right 2/3.
    RightTwoThirds,
    /// Fill the display's usable area.
    Maximize,
    /// Center, keeping the size (shrunk to fit).
    Center,
    /// Return to the frame saved before the first arrangement.
    Restore,
    /// Move to the previous display.
    PreviousDisplay,
    /// Move to the next display.
    NextDisplay,
}

impl WindowAction {
    /// Every action, in the order of the Swift `CaseIterable`.
    pub const ALL: [WindowAction; 18] = [
        Self::LeftHalf,
        Self::RightHalf,
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
        Self::Maximize,
        Self::Center,
        Self::Restore,
        Self::PreviousDisplay,
        Self::NextDisplay,
    ];

    /// The stable launcher command id that performs this action (`leftHalf`, `maximizeWindow`...).
    pub fn command_id(self) -> &'static str {
        LauncherCommand::for_window_action(self).map_or("", LauncherCommand::id)
    }
}

/// An axis-aligned rectangle (`x`, `y` is the minimum corner).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    /// Left edge.
    pub x: f64,
    /// Top edge.
    pub y: f64,
    /// Width.
    pub width: f64,
    /// Height.
    pub height: f64,
}

impl Rect {
    /// A rectangle from its minimum corner and size.
    pub const fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// The empty rectangle at the origin.
    pub const ZERO: Rect = Rect::new(0.0, 0.0, 0.0, 0.0);

    /// Left edge.
    pub fn min_x(&self) -> f64 {
        self.x
    }

    /// Right edge.
    pub fn max_x(&self) -> f64 {
        self.x + self.width
    }

    /// Top edge.
    pub fn min_y(&self) -> f64 {
        self.y
    }

    /// Bottom edge.
    pub fn max_y(&self) -> f64 {
        self.y + self.height
    }

    /// Horizontal midpoint.
    pub fn mid_x(&self) -> f64 {
        self.x + self.width / 2.0
    }

    /// Vertical midpoint.
    pub fn mid_y(&self) -> f64 {
        self.y + self.height / 2.0
    }

    /// Area of the overlap with `other` (zero when they do not overlap).
    pub fn intersection_area(&self, other: &Rect) -> f64 {
        let width = self.max_x().min(other.max_x()) - self.min_x().max(other.min_x());
        let height = self.max_y().min(other.max_y()) - self.min_y().max(other.min_y());
        if width > 0.0 && height > 0.0 {
            width * height
        } else {
            0.0
        }
    }
}

/// The frame `action` gives a window on `screen`. `original` is only used by
/// [`WindowAction::Center`], which keeps its size (shrunk to fit), and by the actions that
/// leave the frame alone but clamp it ([`WindowAction::Restore`] and the display moves).
pub fn arranged_frame(action: WindowAction, original: Rect, screen: Rect) -> Rect {
    let third_x = screen.min_x() + screen.width / 3.0;
    let two_thirds_x = screen.min_x() + screen.width * 2.0 / 3.0;
    match action {
        WindowAction::LeftHalf => Rect::new(
            screen.min_x(),
            screen.min_y(),
            screen.width / 2.0,
            screen.height,
        ),
        WindowAction::RightHalf => Rect::new(
            screen.mid_x(),
            screen.min_y(),
            screen.width / 2.0,
            screen.height,
        ),
        WindowAction::TopHalf => Rect::new(
            screen.min_x(),
            screen.min_y(),
            screen.width,
            screen.height / 2.0,
        ),
        WindowAction::BottomHalf => Rect::new(
            screen.min_x(),
            screen.mid_y(),
            screen.width,
            screen.height / 2.0,
        ),
        WindowAction::TopLeftQuarter => Rect::new(
            screen.min_x(),
            screen.min_y(),
            screen.width / 2.0,
            screen.height / 2.0,
        ),
        WindowAction::TopRightQuarter => Rect::new(
            screen.mid_x(),
            screen.min_y(),
            screen.width / 2.0,
            screen.height / 2.0,
        ),
        WindowAction::BottomLeftQuarter => Rect::new(
            screen.min_x(),
            screen.mid_y(),
            screen.width / 2.0,
            screen.height / 2.0,
        ),
        WindowAction::BottomRightQuarter => Rect::new(
            screen.mid_x(),
            screen.mid_y(),
            screen.width / 2.0,
            screen.height / 2.0,
        ),
        WindowAction::LeftThird => Rect::new(
            screen.min_x(),
            screen.min_y(),
            third_x - screen.min_x(),
            screen.height,
        ),
        WindowAction::CenterThird => Rect::new(
            third_x,
            screen.min_y(),
            two_thirds_x - third_x,
            screen.height,
        ),
        WindowAction::RightThird => Rect::new(
            two_thirds_x,
            screen.min_y(),
            screen.max_x() - two_thirds_x,
            screen.height,
        ),
        WindowAction::LeftTwoThirds => Rect::new(
            screen.min_x(),
            screen.min_y(),
            two_thirds_x - screen.min_x(),
            screen.height,
        ),
        WindowAction::RightTwoThirds => Rect::new(
            third_x,
            screen.min_y(),
            screen.max_x() - third_x,
            screen.height,
        ),
        WindowAction::Maximize => screen,
        WindowAction::Center => {
            let width = original.width.min(screen.width);
            let height = original.height.min(screen.height);
            Rect::new(
                screen.mid_x() - width / 2.0,
                screen.mid_y() - height / 2.0,
                width,
                height,
            )
        }
        WindowAction::Restore | WindowAction::PreviousDisplay | WindowAction::NextDisplay => {
            clamp(original, screen)
        }
    }
}

/// `frame` moved (and shrunk, if needed) to lie inside `screen`.
pub fn clamp(frame: Rect, screen: Rect) -> Rect {
    let width = frame.width.min(screen.width);
    let height = frame.height.min(screen.height);
    Rect::new(
        frame
            .min_x()
            .max(screen.min_x())
            .min(screen.max_x() - width),
        frame
            .min_y()
            .max(screen.min_y())
            .min(screen.max_y() - height),
        width,
        height,
    )
}

/// `frame` carried from display `source` to display `destination`, keeping its relative
/// position and size, clamped to the destination.
pub fn move_between(frame: Rect, source: Rect, destination: Rect) -> Rect {
    clamp(
        Rect::new(
            destination.min_x()
                + (frame.min_x() - source.min_x()) / source.width * destination.width,
            destination.min_y()
                + (frame.min_y() - source.min_y()) / source.height * destination.height,
            frame.width,
            frame.height,
        ),
        destination,
    )
}

/// Index of the display that `frame` overlaps most (the first of equals); `None` without
/// displays.
pub fn screen_index(frame: Rect, screens: &[Rect]) -> Option<usize> {
    let mut best: Option<(usize, f64)> = None;
    for (index, screen) in screens.iter().enumerate() {
        let area = frame.intersection_area(screen);
        if best.is_none_or(|(_, best_area)| area > best_area) {
            best = Some((index, area));
        }
    }
    best.map(|(index, _)| index)
}

/// The display a display-move action lands on: the next or previous one, wrapping around.
/// `None` for other actions and when there is no other display.
pub fn destination_display(action: WindowAction, current: usize, count: usize) -> Option<usize> {
    if count < 2 || current >= count {
        return None;
    }
    match action {
        WindowAction::NextDisplay => Some((current + 1) % count),
        WindowAction::PreviousDisplay => Some((current + count - 1) % count),
        _ => None,
    }
}

/// A suggested global shortcut for a window command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuggestedShortcut {
    /// The launcher command id.
    pub command_id: &'static str,
    /// GTK accelerator (`<Control><Alt>Left`).
    pub accelerator: String,
    /// Display label (`Ctrl+Alt+Left`).
    pub label: String,
    /// The macOS key code of the same chord (for configuration compatibility).
    pub key_code: u32,
    /// The macOS modifier mask of the same chord.
    pub modifiers: u32,
}

/// macOS virtual key codes of the suggested keys.
mod key {
    pub const LEFT: u32 = 123;
    pub const RIGHT: u32 = 124;
    pub const DOWN: u32 = 125;
    pub const UP: u32 = 126;
    pub const RETURN: u32 = 36;
    pub const DELETE: u32 = 51;
    pub const C: u32 = 8;
    pub const D: u32 = 2;
    pub const E: u32 = 14;
    pub const F: u32 = 3;
    pub const G: u32 = 5;
    pub const I: u32 = 34;
    pub const J: u32 = 38;
    pub const K: u32 = 40;
    pub const T: u32 = 17;
    pub const U: u32 = 32;
}

const CARBON_SHIFT: u32 = 0x200;
const CARBON_OPTION: u32 = 0x800;
const CARBON_CONTROL: u32 = 0x1000;

/// The suggested shortcuts: Control–Option (Ctrl+Alt on Linux) with arrows for halves,
/// U/I/J/K for quarters, D/F/G for thirds, E/T for two thirds, Return to maximize, C to
/// center and Delete (Backspace) to restore; Shift added to Left/Right for displays.
pub fn suggested_shortcuts() -> Vec<SuggestedShortcut> {
    let keys: [(WindowAction, u32, &str); 18] = [
        (WindowAction::LeftHalf, key::LEFT, "Left"),
        (WindowAction::RightHalf, key::RIGHT, "Right"),
        (WindowAction::TopHalf, key::UP, "Up"),
        (WindowAction::BottomHalf, key::DOWN, "Down"),
        (WindowAction::TopLeftQuarter, key::U, "U"),
        (WindowAction::TopRightQuarter, key::I, "I"),
        (WindowAction::BottomLeftQuarter, key::J, "J"),
        (WindowAction::BottomRightQuarter, key::K, "K"),
        (WindowAction::LeftThird, key::D, "D"),
        (WindowAction::CenterThird, key::F, "F"),
        (WindowAction::RightThird, key::G, "G"),
        (WindowAction::LeftTwoThirds, key::E, "E"),
        (WindowAction::RightTwoThirds, key::T, "T"),
        (WindowAction::Maximize, key::RETURN, "Return"),
        (WindowAction::Center, key::C, "C"),
        (WindowAction::Restore, key::DELETE, "Backspace"),
        (WindowAction::PreviousDisplay, key::LEFT, "Left"),
        (WindowAction::NextDisplay, key::RIGHT, "Right"),
    ];
    keys.into_iter()
        .filter_map(|(action, key_code, key_label)| {
            let display = matches!(
                action,
                WindowAction::PreviousDisplay | WindowAction::NextDisplay
            );
            let modifiers = CARBON_CONTROL | CARBON_OPTION | if display { CARBON_SHIFT } else { 0 };
            let accelerator = Accelerator::from_carbon(key_code, modifiers)?;
            let label = if display {
                format!("Ctrl+Alt+Shift+{key_label}")
            } else {
                format!("Ctrl+Alt+{key_label}")
            };
            Some(SuggestedShortcut {
                command_id: action.command_id(),
                accelerator: accelerator.to_string(),
                label,
                key_code,
                modifiers,
            })
        })
        .collect()
}

/// The result of applying the suggested shortcuts.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PresetOutcome {
    /// Shortcuts that were added.
    pub added: usize,
    /// Command ids whose suggested chord was taken by the launcher, an app or another command.
    pub conflicts: Vec<String>,
}

/// Fills unassigned window commands with the suggested shortcuts. Existing shortcuts are kept
/// and conflicting suggestions are skipped and reported.
pub fn apply_suggested_shortcuts(
    settings: &mut PortableSettings,
    enabled: &dyn Fn(&ShortcutOwner) -> bool,
) -> PresetOutcome {
    let mut outcome = PresetOutcome::default();
    for suggestion in suggested_shortcuts() {
        if settings
            .command_shortcuts
            .iter()
            .any(|binding| binding.id == suggestion.command_id)
        {
            continue;
        }
        match assign_command_shortcut(
            settings,
            suggestion.command_id,
            &suggestion.accelerator,
            &suggestion.label,
            enabled,
        ) {
            Ok(()) => outcome.added += 1,
            Err(_) => outcome.conflicts.push(suggestion.command_id.to_owned()),
        }
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORIGIN: Rect = Rect::ZERO;

    fn frame(action: WindowAction, screen: Rect) -> Rect {
        arranged_frame(action, ORIGIN, screen)
    }

    #[test]
    fn halves_use_the_visible_frame_and_cover_odd_widths() {
        let screen = Rect::new(-801.0, 25.0, 801.0, 700.0);
        let left = frame(WindowAction::LeftHalf, screen);
        let right = frame(WindowAction::RightHalf, screen);
        assert_eq!(left.max_x(), right.min_x());
        assert_eq!(left.min_x(), screen.min_x());
        assert_eq!(right.max_x(), screen.max_x());
        assert_eq!(right.height, 700.0);
    }

    #[test]
    fn vertical_halves_and_quarters_tile_an_odd_visible_frame() {
        let screen = Rect::new(-801.0, -101.0, 801.0, 701.0);
        let top = frame(WindowAction::TopHalf, screen);
        let bottom = frame(WindowAction::BottomHalf, screen);
        assert_eq!(top.min_y(), screen.min_y());
        assert_eq!(top.max_y(), bottom.min_y());
        assert_eq!(bottom.max_y(), screen.max_y());

        let top_left = frame(WindowAction::TopLeftQuarter, screen);
        let top_right = frame(WindowAction::TopRightQuarter, screen);
        let bottom_left = frame(WindowAction::BottomLeftQuarter, screen);
        let bottom_right = frame(WindowAction::BottomRightQuarter, screen);
        assert_eq!(top_left.max_x(), top_right.min_x());
        assert_eq!(bottom_left.max_x(), bottom_right.min_x());
        assert_eq!(top_left.max_y(), bottom_left.min_y());
        assert_eq!(top_right.max_y(), bottom_right.min_y());
        assert_eq!(top_left.min_x(), screen.min_x());
        assert_eq!(top_right.max_x(), screen.max_x());
        assert_eq!(bottom_left.min_y(), top.min_y() + screen.height / 2.0);
        assert_eq!(bottom_right.max_y(), screen.max_y());
    }

    #[test]
    fn thirds_and_two_thirds_share_exact_boundaries() {
        let screen = Rect::new(15.0, 30.0, 1001.0, 700.0);
        let left = frame(WindowAction::LeftThird, screen);
        let center = frame(WindowAction::CenterThird, screen);
        let right = frame(WindowAction::RightThird, screen);
        assert_eq!(left.min_x(), screen.min_x());
        assert_eq!(left.max_x(), center.min_x());
        assert_eq!(center.max_x(), right.min_x());
        assert_eq!(right.max_x(), screen.max_x());

        let left_two = frame(WindowAction::LeftTwoThirds, screen);
        let right_two = frame(WindowAction::RightTwoThirds, screen);
        assert_eq!(left_two.min_x(), screen.min_x());
        assert_eq!(left_two.max_x(), right.min_x());
        assert_eq!(right_two.min_x(), left.min_x() + screen.width / 3.0);
        assert_eq!(right_two.max_x(), screen.max_x());
        assert_eq!(left_two.height, screen.height);
        assert_eq!(right_two.height, screen.height);
    }

    #[test]
    fn move_preserves_size_and_clamps_to_a_smaller_display() {
        let source = Rect::new(0.0, 0.0, 1600.0, 1000.0);
        let target = Rect::new(-800.0, -600.0, 800.0, 600.0);
        let moved = move_between(Rect::new(1200.0, 800.0, 1000.0, 900.0), source, target);
        assert_eq!(moved, target);
    }

    #[test]
    fn selects_the_display_with_the_greatest_intersection_and_centers_oversize_windows() {
        let screens = [
            Rect::new(0.0, 0.0, 1000.0, 800.0),
            Rect::new(1000.0, 0.0, 1000.0, 800.0),
        ];
        assert_eq!(
            screen_index(Rect::new(900.0, 100.0, 500.0, 400.0), &screens),
            Some(1)
        );
        assert_eq!(screen_index(ORIGIN, &[]), None);
        assert_eq!(
            arranged_frame(
                WindowAction::Center,
                Rect::new(0.0, 0.0, 1500.0, 900.0),
                screens[1]
            ),
            screens[1]
        );
    }

    #[test]
    fn display_moves_wrap_and_need_two_displays() {
        assert_eq!(
            destination_display(WindowAction::NextDisplay, 1, 2),
            Some(0)
        );
        assert_eq!(
            destination_display(WindowAction::PreviousDisplay, 0, 3),
            Some(2)
        );
        assert_eq!(destination_display(WindowAction::NextDisplay, 0, 1), None);
        assert_eq!(destination_display(WindowAction::LeftHalf, 0, 3), None);
    }

    #[test]
    fn suggested_shortcuts_cover_every_action_with_unique_chords() {
        let suggestions = suggested_shortcuts();
        assert_eq!(suggestions.len(), WindowAction::ALL.len());
        let ids: std::collections::HashSet<_> = suggestions.iter().map(|s| s.command_id).collect();
        let window_commands: std::collections::HashSet<_> = LauncherCommand::ALL
            .iter()
            .filter(|c| c.window_action().is_some())
            .map(|c| c.id())
            .collect();
        assert_eq!(ids, window_commands);
        let chords: std::collections::HashSet<_> = suggestions
            .iter()
            .map(|s| format!("{}:{}", s.key_code, s.modifiers))
            .collect();
        assert_eq!(chords.len(), suggestions.len());
        let accelerators: std::collections::HashSet<_> =
            suggestions.iter().map(|s| s.accelerator.clone()).collect();
        assert_eq!(accelerators.len(), suggestions.len());
        assert!(suggestions.iter().all(|s| {
            s.modifiers & (CARBON_CONTROL | CARBON_OPTION) == (CARBON_CONTROL | CARBON_OPTION)
                && s.accelerator.contains("<Control><Alt>")
        }));
        let left = suggestions
            .iter()
            .find(|s| s.command_id == "leftHalf")
            .map(|s| (s.accelerator.as_str(), s.label.as_str()));
        assert_eq!(left, Some(("<Control><Alt>Left", "Ctrl+Alt+Left")));
        let next = suggestions
            .iter()
            .find(|s| s.command_id == "nextDisplay")
            .map(|s| s.accelerator.as_str());
        assert_eq!(next, Some("<Shift><Control><Alt>Right"));
    }
}
