//! Session detection: display server, compositor and layer-shell likelihood.

use crate::paths::Env;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayServer {
    Wayland,
    X11,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Compositor {
    Niri,
    Hyprland,
    Sway,
    Kde,
    Gnome,
    /// Another desktop, carrying the first `XDG_CURRENT_DESKTOP` component.
    Other(String),
    Unknown,
}

impl Compositor {
    /// Human-readable name.
    pub fn name(&self) -> &str {
        match self {
            Self::Niri => "niri",
            Self::Hyprland => "Hyprland",
            Self::Sway => "sway",
            Self::Kde => "KDE Plasma",
            Self::Gnome => "GNOME",
            Self::Other(s) => s,
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionInfo {
    pub display_server: DisplayServer,
    pub compositor: Compositor,
    /// Components of `XDG_CURRENT_DESKTOP`, as given.
    pub current_desktop: Vec<String>,
    /// Whether `wlr-layer-shell` is likely offered (Wayland and not Mutter).
    pub layer_shell_likely: bool,
}

impl SessionInfo {
    pub fn detect() -> Self {
        Self::detect_with(&Env::from_process())
    }

    pub fn detect_with(env: &Env) -> Self {
        let wayland = env.get("WAYLAND_DISPLAY").is_some()
            || env
                .get("XDG_SESSION_TYPE")
                .is_some_and(|t| t.eq_ignore_ascii_case("wayland"));
        let display_server = if wayland {
            DisplayServer::Wayland
        } else if env.get("DISPLAY").is_some()
            || env
                .get("XDG_SESSION_TYPE")
                .is_some_and(|t| t.eq_ignore_ascii_case("x11"))
        {
            DisplayServer::X11
        } else {
            DisplayServer::Unknown
        };

        let current_desktop: Vec<String> = env
            .get("XDG_CURRENT_DESKTOP")
            .map(|v| {
                v.split(':')
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let has = |name: &str| current_desktop.iter().any(|d| d.eq_ignore_ascii_case(name));

        let compositor = if env.get("NIRI_SOCKET").is_some() || has("niri") {
            Compositor::Niri
        } else if env.get("HYPRLAND_INSTANCE_SIGNATURE").is_some() || has("Hyprland") {
            Compositor::Hyprland
        } else if env.get("SWAYSOCK").is_some() || has("sway") {
            Compositor::Sway
        } else if has("KDE") {
            Compositor::Kde
        } else if has("GNOME") || has("ubuntu:GNOME") {
            Compositor::Gnome
        } else if let Some(first) = current_desktop.first() {
            Compositor::Other(first.clone())
        } else {
            Compositor::Unknown
        };

        let layer_shell_likely = display_server == DisplayServer::Wayland
            && !matches!(compositor, Compositor::Gnome | Compositor::Unknown);

        Self {
            display_server,
            compositor,
            current_desktop,
            layer_shell_likely,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn niri_wayland() {
        let env = Env::from_pairs([
            ("WAYLAND_DISPLAY", "wayland-1"),
            ("XDG_CURRENT_DESKTOP", "niri"),
            ("NIRI_SOCKET", "/run/x"),
        ]);
        let s = SessionInfo::detect_with(&env);
        assert_eq!(s.display_server, DisplayServer::Wayland);
        assert_eq!(s.compositor, Compositor::Niri);
        assert!(s.layer_shell_likely);
    }

    #[test]
    fn gnome_has_no_layer_shell() {
        let env = Env::from_pairs([
            ("WAYLAND_DISPLAY", "wayland-0"),
            ("XDG_CURRENT_DESKTOP", "ubuntu:GNOME"),
        ]);
        let s = SessionInfo::detect_with(&env);
        assert_eq!(s.compositor, Compositor::Gnome);
        assert!(!s.layer_shell_likely);
    }

    #[test]
    fn x11_kde_and_sway_by_socket() {
        let kde = SessionInfo::detect_with(&Env::from_pairs([
            ("DISPLAY", ":0"),
            ("XDG_CURRENT_DESKTOP", "KDE"),
        ]));
        assert_eq!(kde.display_server, DisplayServer::X11);
        assert_eq!(kde.compositor, Compositor::Kde);
        assert!(!kde.layer_shell_likely);
        let sway = SessionInfo::detect_with(&Env::from_pairs([
            ("WAYLAND_DISPLAY", "w"),
            ("SWAYSOCK", "/s"),
        ]));
        assert_eq!(sway.compositor, Compositor::Sway);
        let hypr = SessionInfo::detect_with(&Env::from_pairs([
            ("WAYLAND_DISPLAY", "w"),
            ("HYPRLAND_INSTANCE_SIGNATURE", "abc"),
        ]));
        assert_eq!(hypr.compositor, Compositor::Hyprland);
    }

    #[test]
    fn empty_env_is_unknown() {
        let s = SessionInfo::detect_with(&Env::default());
        assert_eq!(s.display_server, DisplayServer::Unknown);
        assert_eq!(s.compositor, Compositor::Unknown);
        assert!(!s.layer_shell_likely);
    }

    #[test]
    fn other_desktop() {
        let s = SessionInfo::detect_with(&Env::from_pairs([
            ("WAYLAND_DISPLAY", "w"),
            ("XDG_CURRENT_DESKTOP", "COSMIC"),
        ]));
        assert_eq!(s.compositor, Compositor::Other("COSMIC".into()));
        assert!(s.layer_shell_likely);
    }
}
