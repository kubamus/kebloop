//! Window arrangement (port of `WindowManagementService`).
//!
//! Action ids are identical to the Swift `LauncherCommand` raw values. The
//! geometry backends (Hyprland, Sway, i3) float the window if needed and set an
//! exact frame inside the output's usable area (bars excluded), remembering the
//! frame from before the first arrangement for *Restore*. niri is a scrolling
//! tiler without screen regions, so tiled windows get the closest column
//! operation and floating windows get proportional sizes and positions;
//! [`capability_table`] tells the UI exactly what each backend does.
//!
//! Full-screen and hidden windows are skipped as on macOS. niri's IPC does not
//! report full-screen state, so that check cannot apply there.

use std::sync::{Arc, Mutex};

use crate::compositor::{
    Compositor, CompositorError, CompositorKind, Cycle, LayoutCommand, Output, PositionChange,
    Rect, SizeChange, Window, WindowId,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WindowAction {
    LeftHalf,
    RightHalf,
    TopHalf,
    BottomHalf,
    TopLeftQuarter,
    TopRightQuarter,
    BottomLeftQuarter,
    BottomRightQuarter,
    LeftThird,
    CenterThird,
    RightThird,
    LeftTwoThirds,
    RightTwoThirds,
    Maximize,
    Center,
    Restore,
    PreviousDisplay,
    NextDisplay,
}

impl WindowAction {
    pub const ALL: [Self; 18] = [
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

    /// The persisted command id; identical to the Swift `LauncherCommand` raw value.
    pub fn id(self) -> &'static str {
        match self {
            Self::LeftHalf => "leftHalf",
            Self::RightHalf => "rightHalf",
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
            Self::Maximize => "maximizeWindow",
            Self::Center => "centerWindow",
            Self::Restore => "restoreWindow",
            Self::PreviousDisplay => "previousDisplay",
            Self::NextDisplay => "nextDisplay",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.id() == id)
    }

    fn is_display(self) -> bool {
        matches!(self, Self::PreviousDisplay | Self::NextDisplay)
    }
}

/// How well an action maps to a backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionSupport {
    /// Behaves like the macOS action.
    Full,
    /// Works, but differently from macOS (the note says how).
    Approximate(&'static str),
    /// Only for floating windows (the window must be floating at call time).
    FloatingOnly,
    /// Not possible; hide the command.
    Unsupported(&'static str),
}

impl ActionSupport {
    /// Whether the UI should offer the command.
    pub fn is_offered(self) -> bool {
        !matches!(self, Self::Unsupported(_))
    }
}

const NIRI_COLUMN: &str = "sets the column width; niri has no fixed screen regions";
const NIRI_FLOATING: &str = "needs a floating window on niri";

/// Support of `action` on `kind`.
pub fn support(kind: CompositorKind, action: WindowAction) -> ActionSupport {
    use WindowAction as A;
    match kind {
        CompositorKind::Hyprland
        | CompositorKind::Sway
        | CompositorKind::I3
        | CompositorKind::X11
        | CompositorKind::KWin
        | CompositorKind::Gnome => ActionSupport::Full,
        CompositorKind::Niri => match action {
            A::TopHalf
            | A::BottomHalf
            | A::TopLeftQuarter
            | A::TopRightQuarter
            | A::BottomLeftQuarter
            | A::BottomRightQuarter => ActionSupport::FloatingOnly,
            A::LeftHalf
            | A::RightHalf
            | A::LeftThird
            | A::CenterThird
            | A::RightThird
            | A::LeftTwoThirds
            | A::RightTwoThirds
            | A::Maximize => ActionSupport::Approximate(NIRI_COLUMN),
            A::Restore => ActionSupport::Approximate("restores the size only"),
            A::Center | A::PreviousDisplay | A::NextDisplay => ActionSupport::Full,
        },
    }
}

/// Every action with its support on `kind`, in settings order.
pub fn capability_table(kind: CompositorKind) -> Vec<(WindowAction, ActionSupport)> {
    WindowAction::ALL
        .into_iter()
        .map(|a| (a, support(kind, a)))
        .collect()
}

#[derive(Debug, thiserror::Error)]
pub enum WindowManagementError {
    #[error("The previous application has no focused window.")]
    NoWindow,
    #[error("This window cannot be arranged. {0}")]
    CannotArrange(String),
    #[error("{0}")]
    Unsupported(String),
    #[error("There is no saved frame for this window.")]
    NoRestore,
    #[error("Connect another display to move this window.")]
    NoOtherDisplay,
    #[error(transparent)]
    Compositor(CompositorError),
}

impl From<CompositorError> for WindowManagementError {
    fn from(error: CompositorError) -> Self {
        match error {
            CompositorError::Unsupported { backend, what } => {
                Self::Unsupported(format!("{backend}: {what}"))
            }
            other => Self::Compositor(other),
        }
    }
}

#[derive(Debug, Clone)]
struct Saved {
    id: WindowId,
    floating: bool,
    frame: Option<Rect>,
    size: Option<(i32, i32)>,
}

const MAX_SAVED: usize = 64;

pub struct WindowManager {
    compositor: Arc<dyn Compositor>,
    saved: Mutex<Vec<Saved>>,
}

impl WindowManager {
    pub fn new(compositor: Arc<dyn Compositor>) -> Self {
        Self {
            compositor,
            saved: Mutex::new(Vec::new()),
        }
    }

    pub fn kind(&self) -> CompositorKind {
        self.compositor.kind()
    }

    /// Capability table of the connected backend.
    pub fn capability_table(&self) -> Vec<(WindowAction, ActionSupport)> {
        capability_table(self.kind())
    }

    /// Arranges `target` (the window that was focused before the launcher
    /// opened) or, without one, the currently focused window.
    pub fn perform(
        &self,
        action: WindowAction,
        target: Option<&WindowId>,
    ) -> Result<(), WindowManagementError> {
        let kind = self.kind();
        if let ActionSupport::Unsupported(reason) = support(kind, action) {
            return Err(WindowManagementError::Unsupported(reason.to_owned()));
        }
        let windows = self.compositor.windows()?;
        let window = match target {
            Some(id) => windows.into_iter().find(|w| &w.id == id),
            None => windows.into_iter().find(|w| w.focused),
        }
        .ok_or(WindowManagementError::NoWindow)?;
        if window.fullscreen {
            return Err(WindowManagementError::CannotArrange(
                "Full-screen windows are excluded.".into(),
            ));
        }
        if window.hidden {
            return Err(WindowManagementError::CannotArrange(
                "Hidden windows are excluded.".into(),
            ));
        }
        let outputs = self.compositor.outputs()?;
        if kind == CompositorKind::Niri {
            self.perform_niri(action, &window, &outputs)
        } else {
            self.perform_geometry(action, &window, &outputs)
        }
    }

    fn saved_index(&self, saved: &[Saved], id: &WindowId) -> Option<usize> {
        saved.iter().position(|s| &s.id == id)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Saved>> {
        self.saved
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn remember(&self, window: &Window) {
        let mut saved = self.lock();
        if self.saved_index(&saved, &window.id).is_some() {
            return;
        }
        if saved.len() >= MAX_SAVED {
            saved.remove(0);
        }
        saved.push(Saved {
            id: window.id.clone(),
            floating: window.floating,
            frame: window.frame,
            size: window.size,
        });
    }

    fn take_saved(&self, id: &WindowId) -> Option<Saved> {
        let saved = self.lock();
        self.saved_index(&saved, id).map(|i| saved[i].clone())
    }

    fn forget(&self, id: &WindowId) {
        self.lock().retain(|s| &s.id != id);
    }

    // ---- Hyprland, Sway, i3 ----

    fn perform_geometry(
        &self,
        action: WindowAction,
        window: &Window,
        outputs: &[Output],
    ) -> Result<(), WindowManagementError> {
        let original = window.frame.ok_or_else(|| {
            WindowManagementError::CannotArrange("The window frame is unknown.".into())
        })?;
        let areas: Vec<Rect> = outputs.iter().map(|o| o.usable).collect();
        let current = screen_index(original, &areas).ok_or(WindowManagementError::NoWindow)?;
        let saved = self.take_saved(&window.id);
        let destination = if action == WindowAction::Restore {
            let saved_frame = saved
                .as_ref()
                .and_then(|s| s.frame)
                .ok_or(WindowManagementError::NoRestore)?;
            let target = screen_index(saved_frame, &areas).unwrap_or(current);
            clamp(saved_frame, areas[target])
        } else if action.is_display() {
            if areas.len() < 2 {
                return Err(WindowManagementError::NoOtherDisplay);
            }
            let step = if action == WindowAction::NextDisplay {
                1
            } else {
                areas.len() - 1
            };
            let next = (current + step) % areas.len();
            move_between(original, areas[current], areas[next])
        } else {
            arranged_frame(action, original, areas[current])
        };
        let set = LayoutCommand::SetFrame {
            id: window.id.clone(),
            frame: destination,
        };
        let result = self.compositor.layout(&set).and_then(|()| {
            // A window that was tiled before returns to the tiling layout on restore.
            match &saved {
                Some(s) if action == WindowAction::Restore && !s.floating => {
                    self.compositor.layout(&LayoutCommand::SetTiled {
                        id: window.id.clone(),
                    })
                }
                _ => Ok(()),
            }
        });
        if let Err(error) = result {
            // Best effort: put the window back where it was.
            let _ = self.compositor.layout(&LayoutCommand::SetFrame {
                id: window.id.clone(),
                frame: original,
            });
            if !window.floating {
                let _ = self.compositor.layout(&LayoutCommand::SetTiled {
                    id: window.id.clone(),
                });
            }
            return Err(error.into());
        }
        if action == WindowAction::Restore {
            self.forget(&window.id);
        } else {
            self.remember(window);
        }
        Ok(())
    }

    // ---- niri ----

    fn perform_niri(
        &self,
        action: WindowAction,
        window: &Window,
        outputs: &[Output],
    ) -> Result<(), WindowManagementError> {
        use WindowAction as A;
        let id = window.id.clone();
        let commands: Vec<LayoutCommand> = if action.is_display() {
            if outputs.len() < 2 {
                return Err(WindowManagementError::NoOtherDisplay);
            }
            let cycle = if action == A::NextDisplay {
                Cycle::Next
            } else {
                Cycle::Previous
            };
            vec![LayoutCommand::MoveToOutput { id, cycle }]
        } else if action == A::Restore {
            let (width, height) = self
                .take_saved(&window.id)
                .and_then(|s| s.size)
                .ok_or(WindowManagementError::NoRestore)?;
            if window.floating {
                vec![LayoutCommand::SetWindowSize {
                    id,
                    width: Some(SizeChange::Fixed(width)),
                    height: Some(SizeChange::Fixed(height)),
                }]
            } else {
                vec![LayoutCommand::SetColumnWidth {
                    id,
                    change: SizeChange::Fixed(width),
                }]
            }
        } else if window.floating {
            if action == A::Center {
                vec![LayoutCommand::CenterWindow { id }]
            } else {
                let (x, y, w, h) = floating_percent(action)
                    .ok_or_else(|| WindowManagementError::Unsupported(NIRI_FLOATING.to_owned()))?;
                vec![
                    LayoutCommand::SetWindowSize {
                        id: id.clone(),
                        width: Some(SizeChange::Percent(w)),
                        height: Some(SizeChange::Percent(h)),
                    },
                    LayoutCommand::MoveFloating {
                        id,
                        x: PositionChange::Percent(x),
                        y: PositionChange::Percent(y),
                    },
                ]
            }
        } else {
            let column = |percent: f64| LayoutCommand::SetColumnWidth {
                id: id.clone(),
                change: SizeChange::Percent(percent),
            };
            match action {
                A::LeftHalf | A::RightHalf => vec![column(50.0)],
                A::LeftThird | A::RightThird => vec![column(THIRD)],
                A::CenterThird => vec![column(THIRD), LayoutCommand::CenterColumn { id }],
                A::LeftTwoThirds | A::RightTwoThirds => vec![column(TWO_THIRDS)],
                A::Maximize => vec![column(100.0)],
                A::Center => vec![LayoutCommand::CenterColumn { id }],
                _ => {
                    return Err(WindowManagementError::Unsupported(
                        "Only floating windows can be placed in a screen region on niri.".into(),
                    ));
                }
            }
        };
        let restoring = action == A::Restore;
        if !restoring {
            self.remember(window);
        }
        for command in &commands {
            self.compositor.layout(command)?;
        }
        if restoring {
            self.forget(&window.id);
        }
        Ok(())
    }
}

const THIRD: f64 = 100.0 / 3.0;
const TWO_THIRDS: f64 = 200.0 / 3.0;

/// `(x, y, width, height)` in percent of the working area (niri floating windows).
fn floating_percent(action: WindowAction) -> Option<(f64, f64, f64, f64)> {
    use WindowAction as A;
    Some(match action {
        A::LeftHalf => (0.0, 0.0, 50.0, 100.0),
        A::RightHalf => (50.0, 0.0, 50.0, 100.0),
        A::TopHalf => (0.0, 0.0, 100.0, 50.0),
        A::BottomHalf => (0.0, 50.0, 100.0, 50.0),
        A::TopLeftQuarter => (0.0, 0.0, 50.0, 50.0),
        A::TopRightQuarter => (50.0, 0.0, 50.0, 50.0),
        A::BottomLeftQuarter => (0.0, 50.0, 50.0, 50.0),
        A::BottomRightQuarter => (50.0, 50.0, 50.0, 50.0),
        A::LeftThird => (0.0, 0.0, THIRD, 100.0),
        A::CenterThird => (THIRD, 0.0, THIRD, 100.0),
        A::RightThird => (TWO_THIRDS, 0.0, THIRD, 100.0),
        A::LeftTwoThirds => (0.0, 0.0, TWO_THIRDS, 100.0),
        A::RightTwoThirds => (THIRD, 0.0, TWO_THIRDS, 100.0),
        A::Maximize => (0.0, 0.0, 100.0, 100.0),
        A::Center | A::Restore | A::PreviousDisplay | A::NextDisplay => return None,
    })
}

/// Index of the area the frame overlaps most.
pub fn screen_index(frame: Rect, screens: &[Rect]) -> Option<usize> {
    (0..screens.len()).max_by_key(|&i| frame.intersection_area(screens[i]))
}

/// Frame for `action` inside `screen`; the halves/thirds tile `screen` exactly.
pub fn arranged_frame(action: WindowAction, original: Rect, screen: Rect) -> Rect {
    use WindowAction as A;
    let (x, y, w, h) = (screen.x, screen.y, screen.w, screen.h);
    let half_w = w / 2;
    let half_h = h / 2;
    let third_x = x + w / 3;
    let two_thirds_x = x + w * 2 / 3;
    match action {
        A::LeftHalf => Rect::new(x, y, half_w, h),
        A::RightHalf => Rect::new(x + half_w, y, w - half_w, h),
        A::TopHalf => Rect::new(x, y, w, half_h),
        A::BottomHalf => Rect::new(x, y + half_h, w, h - half_h),
        A::TopLeftQuarter => Rect::new(x, y, half_w, half_h),
        A::TopRightQuarter => Rect::new(x + half_w, y, w - half_w, half_h),
        A::BottomLeftQuarter => Rect::new(x, y + half_h, half_w, h - half_h),
        A::BottomRightQuarter => Rect::new(x + half_w, y + half_h, w - half_w, h - half_h),
        A::LeftThird => Rect::new(x, y, third_x - x, h),
        A::CenterThird => Rect::new(third_x, y, two_thirds_x - third_x, h),
        A::RightThird => Rect::new(two_thirds_x, y, screen.right() - two_thirds_x, h),
        A::LeftTwoThirds => Rect::new(x, y, two_thirds_x - x, h),
        A::RightTwoThirds => Rect::new(third_x, y, screen.right() - third_x, h),
        A::Maximize => screen,
        A::Center => {
            let width = original.w.min(w);
            let height = original.h.min(h);
            Rect::new(x + (w - width) / 2, y + (h - height) / 2, width, height)
        }
        A::Restore | A::PreviousDisplay | A::NextDisplay => clamp(original, screen),
    }
}

/// Moves `frame` inside `screen`, shrinking it if it is larger.
pub fn clamp(frame: Rect, screen: Rect) -> Rect {
    let w = frame.w.min(screen.w);
    let h = frame.h.min(screen.h);
    Rect::new(
        frame.x.clamp(screen.x, screen.right() - w),
        frame.y.clamp(screen.y, screen.bottom() - h),
        w,
        h,
    )
}

/// Keeps the relative position when moving between displays, then clamps.
pub fn move_between(frame: Rect, from: Rect, to: Rect) -> Rect {
    let scale = |offset: i32, source: i32, target: i32| -> i32 {
        if source == 0 {
            return 0;
        }
        i32::try_from(i64::from(offset) * i64::from(target) / i64::from(source)).unwrap_or(0)
    };
    clamp(
        Rect::new(
            to.x + scale(frame.x - from.x, from.w, to.w),
            to.y + scale(frame.y - from.y, from.h, to.h),
            frame.w,
            frame.h,
        ),
        to,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compositor::ipc::fake;
    use crate::compositor::{Hyprland, Niri, Sway};
    use serde_json::{Value, json};
    use tempfile::TempDir;

    #[test]
    fn ids_match_swift_launcher_commands() {
        let ids: Vec<&str> = WindowAction::ALL.iter().map(|a| a.id()).collect();
        assert_eq!(
            ids,
            [
                "leftHalf",
                "rightHalf",
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
                "maximizeWindow",
                "centerWindow",
                "restoreWindow",
                "previousDisplay",
                "nextDisplay"
            ]
        );
        for action in WindowAction::ALL {
            assert_eq!(WindowAction::from_id(action.id()), Some(action));
        }
        assert_eq!(WindowAction::from_id("shutdown"), None);
    }

    #[test]
    fn frames_tile_the_screen_exactly() {
        let s = Rect::new(10, 20, 1001, 701);
        let o = Rect::new(0, 0, 10, 10);
        use WindowAction as A;
        let f = |a| arranged_frame(a, o, s);
        assert_eq!(f(A::LeftHalf).right(), f(A::RightHalf).x);
        assert_eq!(f(A::RightHalf).right(), s.right());
        assert_eq!(f(A::TopHalf).bottom(), f(A::BottomHalf).y);
        assert_eq!(f(A::BottomHalf).bottom(), s.bottom());
        assert_eq!(f(A::LeftThird).right(), f(A::CenterThird).x);
        assert_eq!(f(A::CenterThird).right(), f(A::RightThird).x);
        assert_eq!(f(A::RightThird).right(), s.right());
        assert_eq!(f(A::LeftTwoThirds).right(), f(A::RightThird).x);
        assert_eq!(f(A::RightTwoThirds).x, f(A::CenterThird).x);
        assert_eq!(f(A::BottomRightQuarter).right(), s.right());
        assert_eq!(f(A::BottomRightQuarter).bottom(), s.bottom());
        assert_eq!(f(A::Maximize), s);
        // Center keeps the size, clamped to the screen.
        assert_eq!(
            arranged_frame(
                A::Center,
                Rect::new(0, 0, 200, 100),
                Rect::new(0, 0, 1000, 600)
            ),
            Rect::new(400, 250, 200, 100)
        );
        assert_eq!(
            arranged_frame(
                A::Center,
                Rect::new(0, 0, 5000, 100),
                Rect::new(0, 0, 1000, 600)
            ),
            Rect::new(0, 250, 1000, 100)
        );
    }

    #[test]
    fn clamp_and_move_between_displays() {
        let a = Rect::new(0, 0, 1000, 800);
        let b = Rect::new(1000, 0, 2000, 1600);
        assert_eq!(
            clamp(Rect::new(900, 700, 300, 300), a),
            Rect::new(700, 500, 300, 300)
        );
        assert_eq!(clamp(Rect::new(-5, -5, 3000, 3000), a), a);
        // Relative position is preserved: (500, 400) of 1000x800 -> half of 2000x1600.
        assert_eq!(
            move_between(Rect::new(500, 400, 100, 100), a, b),
            Rect::new(2000, 800, 100, 100)
        );
        assert_eq!(screen_index(Rect::new(900, 0, 300, 100), &[a, b]), Some(1));
        assert_eq!(screen_index(Rect::new(0, 0, 1, 1), &[]), None);
    }

    #[test]
    fn capability_tables() {
        use WindowAction as A;
        for kind in [
            CompositorKind::Hyprland,
            CompositorKind::Sway,
            CompositorKind::I3,
        ] {
            assert!(
                capability_table(kind)
                    .iter()
                    .all(|(_, s)| *s == ActionSupport::Full)
            );
        }
        assert_eq!(capability_table(CompositorKind::Niri).len(), 18);
        assert_eq!(
            support(CompositorKind::Niri, A::TopHalf),
            ActionSupport::FloatingOnly
        );
        assert!(matches!(
            support(CompositorKind::Niri, A::LeftHalf),
            ActionSupport::Approximate(_)
        ));
        assert_eq!(
            support(CompositorKind::Niri, A::Center),
            ActionSupport::Full
        );
        assert!(support(CompositorKind::Niri, A::TopHalf).is_offered());
        assert!(!ActionSupport::Unsupported("x").is_offered());
    }

    // ---- fake compositors ----

    fn manager(compositor: impl Compositor + 'static) -> WindowManager {
        WindowManager::new(Arc::new(compositor))
    }

    fn niri_window(id: u64, floating: bool, focused: bool, size: (i32, i32)) -> Value {
        json!({"id": id, "title": "t", "app_id": "app", "pid": 7, "workspace_id": 1,
               "is_focused": focused, "is_floating": floating,
               "layout": {"window_size": [size.0, size.1]},
               "focus_timestamp": {"secs": 1, "nanos": 0}})
    }

    fn niri_output(name: &str, x: i32) -> Value {
        json!({"name": name, "logical": {"x": x, "y": 0, "width": 1920, "height": 1200}})
    }

    fn niri_server(dir: &TempDir, windows: Vec<Value>, outputs: Vec<Value>) -> (Niri, fake::Log) {
        let path = fake::socket_path(dir.path());
        let log = fake::line_server(&path, move |request| {
            let value: Value = serde_json::from_str(request).unwrap_or(Value::Null);
            match value.as_str() {
                Some("Windows") => json!({"Ok": {"Windows": windows}}).to_string(),
                Some("Workspaces") => json!({"Ok": {"Workspaces": []}}).to_string(),
                Some("Outputs") => {
                    let map: serde_json::Map<String, Value> = outputs
                        .iter()
                        .filter_map(|o| Some((o.get("name")?.as_str()?.to_owned(), o.clone())))
                        .collect();
                    json!({"Ok": {"Outputs": map}}).to_string()
                }
                Some("FocusedOutput") => json!({"Ok": {"FocusedOutput": null}}).to_string(),
                _ if value.get("Action").is_some() => r#"{"Ok":"Handled"}"#.into(),
                _ => r#"{"Err":"unexpected"}"#.into(),
            }
        });
        (Niri::new(path), log)
    }

    fn niri_actions(log: &fake::Log) -> Vec<Value> {
        fake::log_of(log)
            .iter()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .filter_map(|v| v.get("Action").cloned())
            .collect()
    }

    #[test]
    fn niri_tiled_actions() {
        use WindowAction as A;
        let cases: Vec<(WindowAction, Vec<Value>)> = vec![
            (
                A::LeftHalf,
                vec![json!({"SetColumnWidth": {"change": {"SetProportion": 50.0}}})],
            ),
            (
                A::RightHalf,
                vec![json!({"SetColumnWidth": {"change": {"SetProportion": 50.0}}})],
            ),
            (
                A::LeftThird,
                vec![json!({"SetColumnWidth": {"change": {"SetProportion": THIRD}}})],
            ),
            (
                A::RightTwoThirds,
                vec![json!({"SetColumnWidth": {"change": {"SetProportion": TWO_THIRDS}}})],
            ),
            (
                A::Maximize,
                vec![json!({"SetColumnWidth": {"change": {"SetProportion": 100.0}}})],
            ),
            (A::Center, vec![json!({"CenterColumn": {}})]),
            (
                A::CenterThird,
                vec![
                    json!({"SetColumnWidth": {"change": {"SetProportion": THIRD}}}),
                    json!({"CenterColumn": {}}),
                ],
            ),
        ];
        for (action, commands) in cases {
            let dir = TempDir::new().unwrap();
            let (niri, log) = niri_server(
                &dir,
                vec![niri_window(9, false, true, (948, 1184))],
                vec![niri_output("eDP-1", 0)],
            );
            manager(niri).perform(action, None).unwrap();
            // Every id-less niri action is preceded by FocusWindow of the target.
            let mut expected = Vec::new();
            for command in commands {
                if command.get("FocusWindow").is_none() {
                    expected.push(json!({"FocusWindow": {"id": 9}}));
                }
                expected.push(command);
            }
            assert_eq!(niri_actions(&log), expected, "{action:?}");
        }
    }

    #[test]
    fn niri_tiled_quarters_are_unsupported_and_send_nothing() {
        let dir = TempDir::new().unwrap();
        let (niri, log) = niri_server(
            &dir,
            vec![niri_window(9, false, true, (948, 1184))],
            vec![niri_output("eDP-1", 0)],
        );
        let wm = manager(niri);
        for action in [WindowAction::TopHalf, WindowAction::BottomRightQuarter] {
            assert!(matches!(
                wm.perform(action, None),
                Err(WindowManagementError::Unsupported(_))
            ));
        }
        assert!(niri_actions(&log).is_empty());
    }

    #[test]
    fn niri_floating_regions() {
        let dir = TempDir::new().unwrap();
        let (niri, log) = niri_server(
            &dir,
            vec![niri_window(9, true, true, (800, 600))],
            vec![niri_output("eDP-1", 0)],
        );
        let wm = manager(niri);
        wm.perform(WindowAction::BottomRightQuarter, None).unwrap();
        wm.perform(WindowAction::Center, None).unwrap();
        assert_eq!(
            niri_actions(&log),
            vec![
                json!({"SetWindowWidth": {"id": 9, "change": {"SetProportion": 50.0}}}),
                json!({"SetWindowHeight": {"id": 9, "change": {"SetProportion": 50.0}}}),
                json!({"MoveFloatingWindow": {"id": 9,
                    "x": {"SetProportion": 50.0}, "y": {"SetProportion": 50.0}}}),
                json!({"CenterWindow": {"id": 9}}),
            ]
        );
    }

    #[test]
    fn niri_restore_returns_the_saved_size() {
        let dir = TempDir::new().unwrap();
        let (niri, log) = niri_server(
            &dir,
            vec![niri_window(9, false, true, (948, 1184))],
            vec![niri_output("eDP-1", 0)],
        );
        let wm = manager(niri);
        assert!(matches!(
            wm.perform(WindowAction::Restore, None),
            Err(WindowManagementError::NoRestore)
        ));
        wm.perform(WindowAction::Maximize, None).unwrap();
        // The saved size is the one from before the first arrangement.
        wm.perform(WindowAction::LeftHalf, None).unwrap();
        wm.perform(WindowAction::Restore, None).unwrap();
        let sent = niri_actions(&log);
        assert_eq!(
            sent.last().unwrap(),
            &json!({"SetColumnWidth": {"change": {"SetFixed": 948}}})
        );
        assert!(matches!(
            wm.perform(WindowAction::Restore, None),
            Err(WindowManagementError::NoRestore)
        ));
    }

    #[test]
    fn niri_displays() {
        let dir = TempDir::new().unwrap();
        let (niri, log) = niri_server(
            &dir,
            vec![niri_window(9, false, true, (948, 1184))],
            vec![niri_output("eDP-1", 0), niri_output("DP-2", 1920)],
        );
        let wm = manager(niri);
        wm.perform(WindowAction::NextDisplay, None).unwrap();
        wm.perform(WindowAction::PreviousDisplay, Some(&WindowId("9".into())))
            .unwrap();
        assert_eq!(
            niri_actions(&log),
            vec![
                json!({"FocusWindow": {"id": 9}}),
                json!({"MoveWindowToMonitorNext": {}}),
                json!({"FocusWindow": {"id": 9}}),
                json!({"MoveWindowToMonitorPrevious": {}}),
            ]
        );
        let dir = TempDir::new().unwrap();
        let (niri, _) = niri_server(
            &dir,
            vec![niri_window(9, false, true, (1, 1))],
            vec![niri_output("eDP-1", 0)],
        );
        assert!(matches!(
            manager(niri).perform(WindowAction::NextDisplay, None),
            Err(WindowManagementError::NoOtherDisplay)
        ));
    }

    #[test]
    fn no_window_and_explicit_targets() {
        let dir = TempDir::new().unwrap();
        let (niri, log) = niri_server(
            &dir,
            vec![niri_window(9, false, false, (100, 100))],
            vec![niri_output("eDP-1", 0)],
        );
        let wm = manager(niri);
        assert!(matches!(
            wm.perform(WindowAction::Center, None),
            Err(WindowManagementError::NoWindow)
        ));
        assert!(matches!(
            wm.perform(WindowAction::Center, Some(&WindowId("77".into()))),
            Err(WindowManagementError::NoWindow)
        ));
        wm.perform(WindowAction::Center, Some(&WindowId("9".into())))
            .unwrap();
        assert_eq!(niri_actions(&log).len(), 2);
    }

    fn hypr_client(
        address: &str,
        floating: bool,
        fullscreen: i32,
        at: (i32, i32),
        size: (i32, i32),
    ) -> Value {
        json!({"address": address, "mapped": true, "at": [at.0, at.1], "size": [size.0, size.1],
               "workspace": {"id": 1, "name": "1"}, "floating": floating, "monitor": 0,
               "class": "app", "title": "t", "pid": 5, "fullscreen": fullscreen, "focusHistoryID": 0})
    }

    fn hypr_monitors(second: bool) -> Value {
        let mut list = vec![
            json!({"id": 0, "name": "DP-1", "width": 1920, "height": 1080, "x": 0, "y": 0,
            "scale": 1.0, "transform": 0, "reserved": [0, 30, 0, 0], "focused": true,
            "activeWorkspace": {"id": 1, "name": "1"}}),
        ];
        if second {
            list.push(
                json!({"id": 1, "name": "DP-2", "width": 2560, "height": 1440, "x": 1920, "y": 0,
                "scale": 1.0, "transform": 0, "reserved": [0, 0, 0, 0], "focused": false,
                "activeWorkspace": {"id": 2, "name": "2"}}),
            );
        }
        Value::Array(list)
    }

    fn hypr_server(dir: &TempDir, client: Value, second: bool) -> (Hyprland, fake::Log) {
        let path = fake::socket_path(dir.path());
        let address = client["address"].as_str().unwrap_or_default().to_owned();
        let log = fake::eof_server(&path, move |request| match request {
            "j/clients" => json!([client]).to_string(),
            "j/monitors" => hypr_monitors(second).to_string(),
            "j/activewindow" => json!({"address": address}).to_string(),
            r if r.starts_with("/dispatch ") => "ok".into(),
            _ => "bad".into(),
        });
        (Hyprland::new(path), log)
    }

    fn dispatches(log: &fake::Log) -> Vec<String> {
        fake::log_of(log)
            .into_iter()
            .filter(|l| l.starts_with("/dispatch"))
            .collect()
    }

    #[test]
    fn hyprland_floats_and_places_in_the_usable_area() {
        let dir = TempDir::new().unwrap();
        let (hypr, log) = hypr_server(
            &dir,
            hypr_client("0xa", false, 0, (10, 40), (800, 600)),
            false,
        );
        let wm = manager(hypr);
        wm.perform(WindowAction::RightHalf, None).unwrap();
        assert_eq!(
            dispatches(&log),
            vec![
                "/dispatch setfloating address:0xa",
                "/dispatch resizewindowpixel exact 960 1050,address:0xa",
                "/dispatch movewindowpixel exact 960 30,address:0xa",
            ]
        );
    }

    #[test]
    fn hyprland_restore_returns_to_the_original_frame_and_tiling() {
        let dir = TempDir::new().unwrap();
        let (hypr, log) = hypr_server(
            &dir,
            hypr_client("0xa", false, 0, (10, 40), (800, 600)),
            false,
        );
        let wm = manager(hypr);
        wm.perform(WindowAction::Maximize, None).unwrap();
        wm.perform(WindowAction::LeftHalf, None).unwrap();
        wm.perform(WindowAction::Restore, None).unwrap();
        let sent = dispatches(&log);
        assert_eq!(
            &sent[sent.len() - 4..],
            [
                "/dispatch setfloating address:0xa",
                "/dispatch resizewindowpixel exact 800 600,address:0xa",
                "/dispatch movewindowpixel exact 10 40,address:0xa",
                "/dispatch settiled address:0xa",
            ]
        );
        assert!(matches!(
            wm.perform(WindowAction::Restore, None),
            Err(WindowManagementError::NoRestore)
        ));
    }

    #[test]
    fn hyprland_display_moves_keep_relative_position() {
        let dir = TempDir::new().unwrap();
        let (hypr, log) = hypr_server(
            &dir,
            hypr_client("0xa", true, 0, (1000, 500), (800, 600)),
            true,
        );
        manager(hypr)
            .perform(WindowAction::NextDisplay, None)
            .unwrap();
        // Offsets inside the usable area (0,30,1920,1050) scale to DP-2 (1920,0,2560,1440).
        assert_eq!(
            dispatches(&log),
            vec![
                "/dispatch setfloating address:0xa",
                "/dispatch resizewindowpixel exact 800 600,address:0xa",
                "/dispatch movewindowpixel exact 3253 644,address:0xa",
            ]
        );
        // A window larger than the destination is shrunk and kept on it.
        let dir = TempDir::new().unwrap();
        let (hypr, log) = hypr_server(
            &dir,
            hypr_client("0xa", true, 0, (0, 0), (3000, 2000)),
            true,
        );
        manager(hypr)
            .perform(WindowAction::NextDisplay, None)
            .unwrap();
        assert!(dispatches(&log)[1].contains("exact 2560 1440"));
        // One display only: error and nothing sent.
        let dir = TempDir::new().unwrap();
        let (hypr, log) = hypr_server(&dir, hypr_client("0xa", true, 0, (0, 0), (100, 100)), false);
        assert!(matches!(
            manager(hypr).perform(WindowAction::NextDisplay, None),
            Err(WindowManagementError::NoOtherDisplay)
        ));
        assert!(dispatches(&log).is_empty());
    }

    #[test]
    fn fullscreen_windows_are_skipped() {
        let dir = TempDir::new().unwrap();
        let (hypr, log) = hypr_server(
            &dir,
            hypr_client("0xa", false, 2, (0, 0), (1920, 1080)),
            false,
        );
        assert!(matches!(
            manager(hypr).perform(WindowAction::LeftHalf, None),
            Err(WindowManagementError::CannotArrange(_))
        ));
        assert!(dispatches(&log).is_empty());
    }

    #[test]
    fn sway_uses_the_workspace_rect_as_usable_area() {
        let dir = TempDir::new().unwrap();
        let path = fake::socket_path(dir.path());
        let tree = json!({"type": "root", "nodes": [
            {"type": "output", "name": "DP-1", "nodes": [
                {"type": "workspace", "name": "1", "nodes": [
                    {"id": 10, "type": "con", "name": "w", "app_id": "app", "pid": 3, "focused": true,
                     "fullscreen_mode": 0, "nodes": [], "floating_nodes": [],
                     "rect": {"x": 0, "y": 30, "width": 1920, "height": 1050}}],
                 "floating_nodes": []}],
             "floating_nodes": []}],
            "floating_nodes": []});
        let log = fake::i3_server(&path, move |kind, _| {
            match kind {
            4 => tree.to_string(),
            3 => r#"[{"name":"DP-1","active":true,"focused":true,"rect":{"x":0,"y":0,"width":1920,"height":1080}}]"#.into(),
            1 => r#"[{"name":"1","visible":true,"output":"DP-1","rect":{"x":0,"y":30,"width":1920,"height":1050}}]"#.into(),
            _ => r#"[{"success":true}]"#.into(),
        }
        });
        let wm = manager(Sway::new(CompositorKind::Sway, path));
        wm.perform(WindowAction::TopLeftQuarter, None).unwrap();
        wm.perform(WindowAction::Restore, None).unwrap();
        let commands: Vec<String> = fake::log_of(&log)
            .into_iter()
            .filter_map(|l| l.strip_prefix("0 ").map(str::to_owned))
            .collect();
        assert_eq!(
            commands,
            vec![
                "[con_id=10] floating enable, resize set width 960 px height 525 px, \
                 move absolute position 0 px 30 px",
                "[con_id=10] floating enable, resize set width 1920 px height 1050 px, \
                 move absolute position 0 px 30 px",
                "[con_id=10] floating disable",
            ]
        );
    }
}
