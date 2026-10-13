//! Compositor IPC (no GTK): list/focus/close/hide windows and run layout
//! commands on niri, Hyprland, Sway, i3, GNOME and EWMH X11 desktops.
//!
//! Native compositor backends use their IPC sockets; GNOME uses the bundled
//! Shell extension and X11 uses EWMH through x11rb.
//! [`connect`] picks the backend from the environment ([`Env`]).

pub(crate) mod gnome;
mod hyprland;
pub(crate) mod ipc;
mod kwin;
mod niri;
mod sway;
mod x11;

pub use gnome::Gnome;
pub use hyprland::Hyprland;
pub use kwin::KWin;
pub use niri::Niri;
pub use sway::Sway;
pub use x11::X11;

use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::Value;

use crate::paths::Env;

/// Per-request socket timeout.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompositorKind {
    Niri,
    Hyprland,
    Sway,
    I3,
    X11,
    Gnome,
    KWin,
}

impl CompositorKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Niri => "niri",
            Self::Hyprland => "Hyprland",
            Self::Sway => "Sway",
            Self::I3 => "i3",
            Self::X11 => "X11",
            Self::Gnome => "GNOME",
            Self::KWin => "KWin",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CompositorError {
    /// No supported compositor IPC could be located.
    #[error("no supported compositor IPC: {0}")]
    Unavailable(String),
    /// The backend cannot do this at all (for example hiding a window on niri).
    #[error("{backend} does not support this: {what}")]
    Unsupported {
        backend: &'static str,
        what: &'static str,
    },
    #[error("compositor IPC error: {0}")]
    Io(String),
    #[error("unexpected compositor reply: {0}")]
    Protocol(String),
    /// The compositor understood the request and refused it.
    #[error("the compositor rejected the request: {0}")]
    Rejected(String),
}

/// Compositor-specific window identifier, kept as text (`"22"` on niri and
/// Sway, `"0x5581..."` on Hyprland).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WindowId(pub String);

impl fmt::Display for WindowId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Logical-pixel rectangle (y grows downwards).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    pub const fn right(self) -> i32 {
        self.x + self.w
    }

    pub const fn bottom(self) -> i32 {
        self.y + self.h
    }

    /// Area of the intersection (0 when disjoint).
    pub fn intersection_area(self, other: Self) -> i64 {
        let w = i64::from(self.right().min(other.right()) - self.x.max(other.x));
        let h = i64::from(self.bottom().min(other.bottom()) - self.y.max(other.y));
        if w > 0 && h > 0 { w * h } else { 0 }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Window {
    pub id: WindowId,
    /// Wayland `app_id` or X11 class.
    pub app_id: Option<String>,
    pub title: String,
    pub pid: Option<u32>,
    /// Currently has keyboard focus.
    pub focused: bool,
    pub floating: bool,
    /// Always `false` on niri, whose IPC does not report it.
    pub fullscreen: bool,
    /// Parked where the user cannot see it (Sway scratchpad, Hyprland special workspace).
    pub hidden: bool,
    pub output: Option<String>,
    pub workspace: Option<String>,
    /// Absolute layout coordinates (Hyprland, Sway, i3); `None` on niri.
    pub frame: Option<Rect>,
    /// Window size when known.
    pub size: Option<(i32, i32)>,
    /// Larger means focused more recently (niri focus timestamp, Hyprland focus history).
    pub focus_rank: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Output {
    pub name: String,
    /// Logical frame in the global layout.
    pub frame: Rect,
    /// Area available to windows: excludes panels/bars where the compositor
    /// reports them (Hyprland `reserved`, Sway/i3 workspace rect). niri does
    /// not report layer-shell exclusive zones, so there it equals `frame`.
    pub usable: Rect,
    pub focused: bool,
}

/// Width/height change in the units niri accepts.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SizeChange {
    /// Logical pixels.
    Fixed(i32),
    /// Percent of the working area (0 to 100).
    Percent(f64),
}

/// Floating-window position change (niri).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PositionChange {
    Fixed(f64),
    /// Percent of the working area.
    Percent(f64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cycle {
    Next,
    Previous,
}

/// Semantic layout operations; each backend implements the ones it can.
#[derive(Debug, Clone, PartialEq)]
pub enum LayoutCommand {
    /// Make the window floating and give it this exact frame (Hyprland, Sway, i3).
    SetFrame { id: WindowId, frame: Rect },
    /// Return a window to the tiling layout (Hyprland, Sway, i3).
    SetTiled { id: WindowId },
    /// Change the width of the window's column (niri, tiled).
    SetColumnWidth { id: WindowId, change: SizeChange },
    /// Resize a window (niri; any of the two may be left alone).
    SetWindowSize {
        id: WindowId,
        width: Option<SizeChange>,
        height: Option<SizeChange>,
    },
    /// Move a floating window (niri).
    MoveFloating {
        id: WindowId,
        x: PositionChange,
        y: PositionChange,
    },
    /// Center the window's column (niri, tiled).
    CenterColumn { id: WindowId },
    /// Center a floating window (niri).
    CenterWindow { id: WindowId },
    /// Move to the next/previous output (niri).
    MoveToOutput { id: WindowId, cycle: Cycle },
}

/// What a backend can do; the UI uses it to hide unsupported commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompositorCapabilities {
    pub list_windows: bool,
    pub focus: bool,
    pub close: bool,
    pub hide: bool,
    /// Exact floating geometry ([`LayoutCommand::SetFrame`]).
    pub exact_geometry: bool,
    /// niri-style column/proportion layout commands.
    pub column_layout: bool,
    pub quit_session: bool,
}

pub trait Compositor: Send + Sync {
    fn kind(&self) -> CompositorKind;
    fn capabilities(&self) -> CompositorCapabilities;
    fn windows(&self) -> Result<Vec<Window>, CompositorError>;
    fn focused_window(&self) -> Result<Option<Window>, CompositorError> {
        Ok(self.windows()?.into_iter().find(|w| w.focused))
    }
    fn outputs(&self) -> Result<Vec<Output>, CompositorError>;
    fn focus_window(&self, id: &WindowId) -> Result<(), CompositorError>;
    /// Asks the window to close (like its close button).
    fn close_window(&self, id: &WindowId) -> Result<(), CompositorError>;
    /// Parks the window out of sight (the macOS "Hide" equivalent).
    fn hide_window(&self, id: &WindowId) -> Result<(), CompositorError>;
    /// Brings a hidden window back and focuses it.
    fn show_window(&self, id: &WindowId) -> Result<(), CompositorError> {
        self.focus_window(id)
    }
    /// Asks the compositor to exit (ends the graphical session).
    fn quit_session(&self) -> Result<(), CompositorError>;
    fn layout(&self, command: &LayoutCommand) -> Result<(), CompositorError>;
}

impl Compositor for Box<dyn Compositor> {
    fn kind(&self) -> CompositorKind {
        (**self).kind()
    }
    fn capabilities(&self) -> CompositorCapabilities {
        (**self).capabilities()
    }
    fn windows(&self) -> Result<Vec<Window>, CompositorError> {
        (**self).windows()
    }
    fn focused_window(&self) -> Result<Option<Window>, CompositorError> {
        (**self).focused_window()
    }
    fn outputs(&self) -> Result<Vec<Output>, CompositorError> {
        (**self).outputs()
    }
    fn focus_window(&self, id: &WindowId) -> Result<(), CompositorError> {
        (**self).focus_window(id)
    }
    fn close_window(&self, id: &WindowId) -> Result<(), CompositorError> {
        (**self).close_window(id)
    }
    fn hide_window(&self, id: &WindowId) -> Result<(), CompositorError> {
        (**self).hide_window(id)
    }
    fn show_window(&self, id: &WindowId) -> Result<(), CompositorError> {
        (**self).show_window(id)
    }
    fn quit_session(&self) -> Result<(), CompositorError> {
        (**self).quit_session()
    }
    fn layout(&self, command: &LayoutCommand) -> Result<(), CompositorError> {
        (**self).layout(command)
    }
}

/// Which backend the environment points at, without connecting.
pub fn detect_kind(env: &Env) -> Option<CompositorKind> {
    if env.get("NIRI_SOCKET").is_some() {
        Some(CompositorKind::Niri)
    } else if env.get("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
        Some(CompositorKind::Hyprland)
    } else if env.get("SWAYSOCK").is_some() {
        Some(CompositorKind::Sway)
    } else if env.get("I3SOCK").is_some() {
        Some(CompositorKind::I3)
    } else if env.get("DISPLAY").is_some()
        && env.get("WAYLAND_DISPLAY").is_none()
        && env.get("XDG_SESSION_TYPE") != Some("wayland")
    {
        Some(CompositorKind::X11)
    } else if env.get("XDG_CURRENT_DESKTOP").is_some_and(|value| {
        value
            .split(':')
            .any(|part| part.eq_ignore_ascii_case("GNOME"))
    }) {
        Some(CompositorKind::Gnome)
    } else if env.get("XDG_CURRENT_DESKTOP").is_some_and(|value| {
        value
            .split(':')
            .any(|part| part.eq_ignore_ascii_case("KDE"))
    }) {
        Some(CompositorKind::KWin)
    } else {
        None
    }
}

/// Hyprland command socket for this environment.
pub fn hyprland_socket_path(env: &Env) -> Option<PathBuf> {
    let signature = env.get("HYPRLAND_INSTANCE_SIGNATURE")?;
    let runtime = env.get_abs_path("XDG_RUNTIME_DIR")?;
    let primary = runtime.join("hypr").join(signature).join(".socket.sock");
    if primary.exists() {
        return Some(primary);
    }
    let legacy = PathBuf::from("/tmp/hypr")
        .join(signature)
        .join(".socket.sock");
    Some(if legacy.exists() { legacy } else { primary })
}

/// Creates the backend for the environment; `Unavailable(reason)` otherwise.
pub fn connect(env: &Env) -> Result<Box<dyn Compositor>, CompositorError> {
    let kind = detect_kind(env).ok_or_else(|| {
        CompositorError::Unavailable(
            "no supported compositor socket, GNOME desktop, or native X11 display".into(),
        )
    })?;
    let unset = |name: &str| CompositorError::Unavailable(format!("{name} is not usable"));
    Ok(match kind {
        CompositorKind::KWin => Box::new(KWin::new()?),
        CompositorKind::Gnome => Box::new(Gnome::new()?),
        CompositorKind::X11 => Box::new(X11::new(
            env.get("DISPLAY").ok_or_else(|| unset("DISPLAY"))?,
        )?),
        CompositorKind::Niri => Box::new(Niri::new(
            env.get("NIRI_SOCKET")
                .map(PathBuf::from)
                .ok_or_else(|| unset("NIRI_SOCKET"))?,
        )),
        CompositorKind::Hyprland => Box::new(Hyprland::new(
            hyprland_socket_path(env).ok_or_else(|| unset("XDG_RUNTIME_DIR"))?,
        )),
        CompositorKind::Sway => Box::new(Sway::new(
            CompositorKind::Sway,
            env.get("SWAYSOCK")
                .map(PathBuf::from)
                .ok_or_else(|| unset("SWAYSOCK"))?,
        )),
        CompositorKind::I3 => Box::new(Sway::new(
            CompositorKind::I3,
            env.get("I3SOCK")
                .map(PathBuf::from)
                .ok_or_else(|| unset("I3SOCK"))?,
        )),
    })
}

/// Capabilities without opening a socket (for settings screens).
pub fn capabilities_for(kind: CompositorKind) -> CompositorCapabilities {
    match kind {
        CompositorKind::KWin => kwin::CAPABILITIES,
        CompositorKind::Gnome => gnome::CAPABILITIES,
        CompositorKind::X11 => x11::CAPABILITIES,
        CompositorKind::Niri => niri::CAPABILITIES,
        CompositorKind::Hyprland => hyprland::CAPABILITIES,
        CompositorKind::Sway | CompositorKind::I3 => sway::CAPABILITIES,
    }
}

// ---- JSON helpers shared by the backends ----

pub(crate) fn json_i32(value: &Value) -> Option<i32> {
    let number = value.as_f64()?;
    if number.is_finite() && number.abs() < f64::from(i32::MAX) {
        // Rounded and range-checked above.
        #[allow(clippy::cast_possible_truncation)]
        Some(number.round() as i32)
    } else {
        None
    }
}

pub(crate) fn json_str(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

pub(crate) fn json_pid(value: &Value) -> Option<u32> {
    value
        .get("pid")
        .and_then(Value::as_u64)
        .and_then(|p| u32::try_from(p).ok())
        .filter(|p| *p > 0)
}

pub(crate) fn parse_json(text: &str) -> Result<Value, CompositorError> {
    serde_json::from_str(text).map_err(|e| CompositorError::Protocol(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detection_order_and_errors() {
        assert_eq!(detect_kind(&Env::default()), None);
        assert!(matches!(
            connect(&Env::default()),
            Err(CompositorError::Unavailable(_))
        ));
        let env = Env::from_pairs([("SWAYSOCK", "/x"), ("I3SOCK", "/y")]);
        assert_eq!(detect_kind(&env), Some(CompositorKind::Sway));
        let env = Env::from_pairs([("I3SOCK", "/y")]);
        assert_eq!(
            connect(&env).map(|c| c.kind()).ok(),
            Some(CompositorKind::I3)
        );
        let env = Env::from_pairs([("NIRI_SOCKET", "/n"), ("SWAYSOCK", "/x")]);
        assert_eq!(detect_kind(&env), Some(CompositorKind::Niri));
        // Hyprland without a runtime dir cannot be located.
        let env = Env::from_pairs([("HYPRLAND_INSTANCE_SIGNATURE", "abc")]);
        assert!(matches!(
            connect(&env),
            Err(CompositorError::Unavailable(_))
        ));
    }

    #[test]
    fn x11_does_not_mistake_xwayland_for_native_x11() {
        assert_eq!(
            detect_kind(&Env::from_pairs([("DISPLAY", ":0")])),
            Some(CompositorKind::X11)
        );
        assert_eq!(
            detect_kind(&Env::from_pairs([
                ("DISPLAY", ":0"),
                ("WAYLAND_DISPLAY", "wayland-0")
            ])),
            None
        );
        assert_eq!(
            detect_kind(&Env::from_pairs([
                ("DISPLAY", ":0"),
                ("XDG_SESSION_TYPE", "wayland")
            ])),
            None
        );
    }

    #[test]
    fn hyprland_socket_prefers_runtime_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        let env = Env::from_pairs([
            ("HYPRLAND_INSTANCE_SIGNATURE", "sig"),
            ("XDG_RUNTIME_DIR", dir.path().to_str().unwrap()),
        ]);
        assert_eq!(
            hyprland_socket_path(&env),
            Some(dir.path().join("hypr/sig/.socket.sock"))
        );
    }

    #[test]
    fn rect_intersection() {
        let a = Rect::new(0, 0, 100, 100);
        assert_eq!(a.intersection_area(Rect::new(50, 50, 100, 100)), 2500);
        assert_eq!(a.intersection_area(Rect::new(100, 0, 10, 10)), 0);
    }
}
