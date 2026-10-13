//! Global shortcuts: XDG GlobalShortcuts portal client and compositor
//! configuration snippets.
//!
//! Bindings are expressed as GTK accelerator strings (`<Alt>space`,
//! `<Control><Shift>k`) and converted to the portal trigger syntax or to the
//! config syntax of niri, Hyprland, Sway and i3.

use std::fmt::Write as _;

use futures_lite::StreamExt;

// ---------------------------------------------------------------------------
// Accelerator parsing
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AcceleratorError {
    #[error("empty accelerator")]
    Empty,
    #[error("unknown modifier <{0}>")]
    UnknownModifier(String),
    #[error("accelerator `{0}` has no key")]
    MissingKey(String),
    #[error("unterminated modifier in `{0}`")]
    Unterminated(String),
}

/// A parsed keyboard shortcut.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Accelerator {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub logo: bool,
    /// XKB keysym name as written in the accelerator (single ASCII letters are lowercased).
    pub key: String,
}

impl Accelerator {
    /// Parses a GTK accelerator such as `<Alt>space` or `<Primary><Shift>P`.
    pub fn parse(text: &str) -> Result<Self, AcceleratorError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(AcceleratorError::Empty);
        }
        let mut acc = Self {
            ctrl: false,
            alt: false,
            shift: false,
            logo: false,
            key: String::new(),
        };
        let mut rest = text;
        while let Some(after) = rest.strip_prefix('<') {
            let Some(end) = after.find('>') else {
                return Err(AcceleratorError::Unterminated(text.to_owned()));
            };
            match after[..end].to_ascii_lowercase().as_str() {
                "control" | "ctrl" | "primary" | "ctl" => acc.ctrl = true,
                "alt" | "mod1" | "meta" => acc.alt = true,
                "shift" => acc.shift = true,
                "super" | "mod4" | "win" | "hyper" => acc.logo = true,
                other => return Err(AcceleratorError::UnknownModifier(other.to_owned())),
            }
            rest = &after[end + 1..];
        }
        let key = rest.trim();
        if key.is_empty() {
            return Err(AcceleratorError::MissingKey(text.to_owned()));
        }
        acc.key = if key.len() == 1 && key.is_ascii() {
            key.to_ascii_lowercase()
        } else {
            key.to_owned()
        };
        Ok(acc)
    }

    /// XDG shortcuts-spec trigger, e.g. `CTRL+ALT+space`.
    pub fn to_portal(&self) -> String {
        let mut out = String::new();
        for (on, name) in [
            (self.ctrl, "CTRL"),
            (self.alt, "ALT"),
            (self.shift, "SHIFT"),
            (self.logo, "LOGO"),
        ] {
            if on {
                out.push_str(name);
                out.push('+');
            }
        }
        out.push_str(&self.key);
        out
    }

    fn joined(&self, names: [&str; 4], sep: &str, key: &str) -> String {
        let mut parts: Vec<&str> = Vec::new();
        for (on, name) in [
            (self.ctrl, names[0]),
            (self.alt, names[1]),
            (self.shift, names[2]),
            (self.logo, names[3]),
        ] {
            if on {
                parts.push(name);
            }
        }
        parts.push(key);
        parts.join(sep)
    }

    /// niri bind key: `Ctrl+Alt+Shift+Super+Key` with a capitalised key name.
    pub fn to_niri(&self) -> String {
        let mut key = self.key.clone();
        if let Some(first) = key.get_mut(0..1) {
            first.make_ascii_uppercase();
        }
        self.joined(["Ctrl", "Alt", "Shift", "Super"], "+", &key)
    }

    /// Sway / i3 `bindsym` key: `Ctrl+Mod1+Shift+Mod4+key`.
    pub fn to_sway(&self) -> String {
        self.joined(["Ctrl", "Mod1", "Shift", "Mod4"], "+", &self.key)
    }

    /// Hyprland `bind = MODS, key, ...` modifier and key parts.
    pub fn to_hyprland_parts(&self) -> (String, String) {
        let mut mods: Vec<&str> = Vec::new();
        for (on, name) in [
            (self.logo, "SUPER"),
            (self.ctrl, "CTRL"),
            (self.alt, "ALT"),
            (self.shift, "SHIFT"),
        ] {
            if on {
                mods.push(name);
            }
        }
        (mods.join(" "), self.key.clone())
    }
}

/// Converts a GTK accelerator to the portal trigger syntax.
pub fn gtk_accelerator_to_portal(accelerator: &str) -> Result<String, AcceleratorError> {
    Accelerator::parse(accelerator).map(|a| a.to_portal())
}

// ---------------------------------------------------------------------------
// Compositor snippets
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnippetTarget {
    Niri,
    Hyprland,
    Sway,
    I3,
}

impl SnippetTarget {
    pub const ALL: [Self; 4] = [Self::Niri, Self::Hyprland, Self::Sway, Self::I3];

    pub fn name(self) -> &'static str {
        match self {
            Self::Niri => "niri",
            Self::Hyprland => "Hyprland",
            Self::Sway => "Sway",
            Self::I3 => "i3",
        }
    }

    /// Where the line belongs.
    pub fn config_hint(self) -> &'static str {
        match self {
            Self::Niri => "~/.config/niri/config.kdl, inside the binds { } block",
            Self::Hyprland => "~/.config/hypr/hyprland.conf",
            Self::Sway => "~/.config/sway/config",
            Self::I3 => "~/.config/i3/config",
        }
    }
}

/// A ready-to-paste configuration line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snippet {
    pub target: SnippetTarget,
    pub config_hint: &'static str,
    pub text: String,
}

/// Splits a command line into words honouring single and double quotes.
pub fn split_command(command: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut started = false;
    let mut chars = command.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some('"'), '\\') => {
                if let Some(n) = chars.next() {
                    current.push(n);
                }
            }
            (Some(_), c) => current.push(c),
            (None, '\'' | '"') => {
                quote = Some(c);
                started = true;
            }
            (None, c) if c.is_whitespace() => {
                if started || !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            (None, c) => current.push(c),
        }
    }
    if started || !current.is_empty() {
        words.push(current);
    }
    words
}

fn kdl_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Snippet for one compositor.
pub fn snippet_for(
    target: SnippetTarget,
    binding: &str,
    command: &str,
) -> Result<Snippet, AcceleratorError> {
    let acc = Accelerator::parse(binding)?;
    let command = command.trim();
    let text = match target {
        SnippetTarget::Niri => {
            let mut args = String::new();
            for word in split_command(command) {
                let _ = write!(args, " {}", kdl_string(&word));
            }
            format!("{} {{ spawn{args}; }}", acc.to_niri())
        }
        SnippetTarget::Hyprland => {
            let (mods, key) = acc.to_hyprland_parts();
            format!("bind = {mods}, {key}, exec, {command}")
        }
        SnippetTarget::Sway => format!("bindsym {} exec {command}", acc.to_sway()),
        SnippetTarget::I3 => format!("bindsym {} exec --no-startup-id {command}", acc.to_sway()),
    };
    Ok(Snippet {
        target,
        config_hint: target.config_hint(),
        text,
    })
}

/// Snippets for niri, Hyprland, Sway and i3.
pub fn compositor_snippets(binding: &str, command: &str) -> Result<Vec<Snippet>, AcceleratorError> {
    SnippetTarget::ALL
        .into_iter()
        .map(|t| snippet_for(t, binding, command))
        .collect()
}

// ---------------------------------------------------------------------------
// Portal client
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum ShortcutsError {
    /// The desktop has no GlobalShortcuts portal (use compositor bindings).
    #[error("global shortcuts portal unsupported: {0}")]
    Unsupported(String),
    #[error("global shortcuts portal error: {0}")]
    Portal(String),
    #[error(transparent)]
    Accelerator(#[from] AcceleratorError),
}

/// Result of probing the portal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortalSupport {
    Available { version: u32 },
    Unavailable(String),
}

/// Reads `org.freedesktop.portal.GlobalShortcuts.version` from the portal
/// service without creating a session (no dialogs).
pub fn detect_portal_support() -> PortalSupport {
    use zbus::blocking::{Connection, Proxy};
    let conn = match Connection::session() {
        Ok(c) => c,
        Err(e) => return PortalSupport::Unavailable(format!("no session bus: {e}")),
    };
    let proxy = match Proxy::new(
        &conn,
        "org.freedesktop.portal.Desktop",
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.DBus.Properties",
    ) {
        Ok(p) => p,
        Err(e) => return PortalSupport::Unavailable(format!("portal proxy: {e}")),
    };
    match proxy.call::<_, _, zbus::zvariant::OwnedValue>(
        "Get",
        &("org.freedesktop.portal.GlobalShortcuts", "version"),
    ) {
        Ok(v) => match u32::try_from(v) {
            Ok(version) => PortalSupport::Available { version },
            Err(e) => PortalSupport::Unavailable(format!("unexpected version type: {e}")),
        },
        Err(e) => PortalSupport::Unavailable(format!(
            "the desktop portal does not implement GlobalShortcuts ({e})"
        )),
    }
}

/// A shortcut to register.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShortcutSpec {
    pub id: String,
    pub description: String,
    /// Portal trigger syntax (see [`ShortcutSpec::with_accelerator`]).
    pub preferred_trigger: Option<String>,
}

impl ShortcutSpec {
    pub fn new(id: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            description: description.into(),
            preferred_trigger: None,
        }
    }

    /// Sets the preferred trigger from a GTK accelerator such as `<Alt>space`.
    pub fn with_accelerator(mut self, accelerator: &str) -> Result<Self, AcceleratorError> {
        self.preferred_trigger = Some(gtk_accelerator_to_portal(accelerator)?);
        Ok(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutEventKind {
    Activated,
    Deactivated,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShortcutEvent {
    pub id: String,
    pub kind: ShortcutEventKind,
    /// Compositor timestamp in milliseconds.
    pub timestamp_ms: u64,
}

/// A shortcut as bound by the compositor (the user may have changed the trigger).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundShortcut {
    pub id: String,
    pub description: String,
    pub trigger_description: String,
}

/// Running portal session. Dropping it closes the session.
pub struct PortalShortcuts {
    stop: async_channel::Sender<()>,
    /// Shortcuts the portal accepted.
    pub bound: Vec<BoundShortcut>,
}

impl Drop for PortalShortcuts {
    fn drop(&mut self) {
        let _ = self.stop.try_send(());
    }
}

enum Next {
    Activated(Option<ashpd::desktop::global_shortcuts::Activated>),
    Deactivated(Option<ashpd::desktop::global_shortcuts::Deactivated>),
    Stop,
}

/// Creates a GlobalShortcuts session, binds `specs` and forwards activations.
/// Runs on a dedicated thread driven by `futures-lite` (no tokio).
///
/// Returns [`ShortcutsError::Unsupported`] when the portal lacks the interface;
/// callers then show [`compositor_snippets`] instead.
pub fn start_portal_shortcuts(
    specs: Vec<ShortcutSpec>,
) -> Result<(PortalShortcuts, async_channel::Receiver<ShortcutEvent>), ShortcutsError> {
    if let PortalSupport::Unavailable(reason) = detect_portal_support() {
        return Err(ShortcutsError::Unsupported(reason));
    }
    let (events_tx, events_rx) = async_channel::bounded(32);
    let (stop_tx, stop_rx) = async_channel::bounded::<()>(1);
    let (ready_tx, ready_rx) =
        std::sync::mpsc::channel::<Result<Vec<BoundShortcut>, ShortcutsError>>();

    std::thread::Builder::new()
        .name("kebloop-portal-shortcuts".into())
        .spawn(move || {
            futures_lite::future::block_on(portal_main(specs, events_tx, stop_rx, ready_tx));
        })
        .map_err(|e| ShortcutsError::Portal(e.to_string()))?;

    match ready_rx.recv_timeout(std::time::Duration::from_secs(120)) {
        Ok(Ok(bound)) => Ok((
            PortalShortcuts {
                stop: stop_tx,
                bound,
            },
            events_rx,
        )),
        Ok(Err(e)) => Err(e),
        Err(_) => Err(ShortcutsError::Portal(
            "timed out waiting for the portal".into(),
        )),
    }
}

async fn portal_main(
    specs: Vec<ShortcutSpec>,
    events: async_channel::Sender<ShortcutEvent>,
    stop: async_channel::Receiver<()>,
    ready: std::sync::mpsc::Sender<Result<Vec<BoundShortcut>, ShortcutsError>>,
) {
    use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
    use futures_lite::future::or;

    let portal_err = |e: ashpd::Error| ShortcutsError::Portal(e.to_string());
    let setup = async {
        let portal = GlobalShortcuts::new()
            .await
            .map_err(|e| ShortcutsError::Unsupported(e.to_string()))?;
        let activated = portal.receive_activated().await.map_err(portal_err)?;
        let deactivated = portal.receive_deactivated().await.map_err(portal_err)?;
        let session = portal
            .create_session(Default::default())
            .await
            .map_err(portal_err)?;
        let new: Vec<NewShortcut> = specs
            .iter()
            .map(|s| {
                NewShortcut::new(s.id.clone(), s.description.clone())
                    .preferred_trigger(s.preferred_trigger.as_deref())
            })
            .collect();
        let request = portal
            .bind_shortcuts(&session, &new, None, Default::default())
            .await
            .map_err(portal_err)?;
        let response = request.response().map_err(portal_err)?;
        let bound: Vec<BoundShortcut> = response
            .shortcuts()
            .iter()
            .map(|s| BoundShortcut {
                id: s.id().to_owned(),
                description: s.description().to_owned(),
                trigger_description: s.trigger_description().to_owned(),
            })
            .collect();
        Ok::<_, ShortcutsError>((portal, session, activated, deactivated, bound))
    };
    let (portal, session, mut activated, mut deactivated, bound) = match setup.await {
        Ok(v) => v,
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    let _ = ready.send(Ok(bound));

    loop {
        let next = or(
            or(async { Next::Activated(activated.next().await) }, async {
                Next::Deactivated(deactivated.next().await)
            }),
            async {
                let _ = stop.recv().await;
                Next::Stop
            },
        )
        .await;
        let event = match next {
            Next::Activated(Some(a)) => ShortcutEvent {
                id: a.shortcut_id().to_owned(),
                kind: ShortcutEventKind::Activated,
                timestamp_ms: u64::try_from(a.timestamp().as_millis()).unwrap_or(u64::MAX),
            },
            Next::Deactivated(Some(d)) => ShortcutEvent {
                id: d.shortcut_id().to_owned(),
                kind: ShortcutEventKind::Deactivated,
                timestamp_ms: u64::try_from(d.timestamp().as_millis()).unwrap_or(u64::MAX),
            },
            Next::Activated(None) | Next::Deactivated(None) | Next::Stop => break,
        };
        if events.send(event).await.is_err() {
            break;
        }
    }
    let _ = session.close().await;
    drop(portal);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn acc(s: &str) -> Accelerator {
        Accelerator::parse(s).unwrap()
    }

    #[test]
    fn parses_gtk_accelerators() {
        let a = acc("<Alt>space");
        assert!(a.alt && !a.ctrl && !a.shift && !a.logo);
        assert_eq!(a.key, "space");
        let a = acc("<Primary><Shift>K");
        assert!(a.ctrl && a.shift);
        assert_eq!(a.key, "k");
        let a = acc("<Super><Mod1>Return");
        assert!(a.logo && a.alt);
        assert_eq!(a.key, "Return");
        assert_eq!(acc("F12").key, "F12");
    }

    #[test]
    fn rejects_invalid_accelerators() {
        assert_eq!(Accelerator::parse(""), Err(AcceleratorError::Empty));
        assert!(matches!(
            Accelerator::parse("<Alt>"),
            Err(AcceleratorError::MissingKey(_))
        ));
        assert!(matches!(
            Accelerator::parse("<Bogus>x"),
            Err(AcceleratorError::UnknownModifier(_))
        ));
        assert!(matches!(
            Accelerator::parse("<Alt"),
            Err(AcceleratorError::Unterminated(_))
        ));
    }

    #[test]
    fn portal_format() {
        assert_eq!(
            gtk_accelerator_to_portal("<Alt>space").unwrap(),
            "ALT+space"
        );
        assert_eq!(
            gtk_accelerator_to_portal("<Control><Alt>T").unwrap(),
            "CTRL+ALT+t"
        );
        assert_eq!(
            gtk_accelerator_to_portal("<Shift><Super>Return").unwrap(),
            "SHIFT+LOGO+Return"
        );
    }

    #[test]
    fn niri_snippet() {
        let s = snippet_for(SnippetTarget::Niri, "<Alt>space", "kebloop toggle").unwrap();
        assert_eq!(s.text, "Alt+Space { spawn \"kebloop\" \"toggle\"; }");
        let s = snippet_for(
            SnippetTarget::Niri,
            "<Control><Shift>k",
            "kebloop launch 'org.gnome.Nautilus.desktop'",
        )
        .unwrap();
        assert_eq!(
            s.text,
            "Ctrl+Shift+K { spawn \"kebloop\" \"launch\" \"org.gnome.Nautilus.desktop\"; }"
        );
        let s = snippet_for(SnippetTarget::Niri, "<Super>Return", "foot -e \"a b\"").unwrap();
        assert_eq!(s.text, "Super+Return { spawn \"foot\" \"-e\" \"a b\"; }");
    }

    #[test]
    fn hyprland_snippet() {
        let s = snippet_for(SnippetTarget::Hyprland, "<Alt>space", "kebloop toggle").unwrap();
        assert_eq!(s.text, "bind = ALT, space, exec, kebloop toggle");
        let s = snippet_for(
            SnippetTarget::Hyprland,
            "<Control><Shift><Super>p",
            "kebloop toggle",
        )
        .unwrap();
        assert_eq!(s.text, "bind = SUPER CTRL SHIFT, p, exec, kebloop toggle");
        let s = snippet_for(SnippetTarget::Hyprland, "F12", "kebloop toggle").unwrap();
        assert_eq!(s.text, "bind = , F12, exec, kebloop toggle");
    }

    #[test]
    fn sway_and_i3_snippets() {
        let s = snippet_for(SnippetTarget::Sway, "<Alt>space", "kebloop toggle").unwrap();
        assert_eq!(s.text, "bindsym Mod1+space exec kebloop toggle");
        let s = snippet_for(SnippetTarget::I3, "<Control><Super>x", "kebloop toggle").unwrap();
        assert_eq!(
            s.text,
            "bindsym Ctrl+Mod4+x exec --no-startup-id kebloop toggle"
        );
    }

    #[test]
    fn all_snippets_and_errors() {
        let all = compositor_snippets("<Alt>space", "kebloop toggle").unwrap();
        assert_eq!(all.len(), 4);
        assert_eq!(all[0].target, SnippetTarget::Niri);
        assert!(all.iter().all(|s| !s.config_hint.is_empty()));
        assert!(compositor_snippets("<Nope>x", "kebloop toggle").is_err());
    }

    #[test]
    fn command_splitting() {
        assert_eq!(split_command("a  b"), ["a", "b"]);
        assert_eq!(
            split_command("a 'b c' \"d e\" f\\ g"),
            ["a", "b c", "d e", "f\\", "g"]
        );
        assert_eq!(split_command("x \"\""), ["x", ""]);
        assert!(split_command("   ").is_empty());
    }

    #[test]
    fn spec_conversion() {
        let spec = ShortcutSpec::new("toggle", "Toggle Kebloop")
            .with_accelerator("<Alt>space")
            .unwrap();
        assert_eq!(spec.preferred_trigger.as_deref(), Some("ALT+space"));
        assert!(
            ShortcutSpec::new("x", "y")
                .with_accelerator("<Bad>q")
                .is_err()
        );
    }

    /// Read-only: queries the session bus for the portal interface.
    #[test]
    #[ignore = "requires a session bus with xdg-desktop-portal"]
    fn live_portal_detection() {
        let support = detect_portal_support();
        eprintln!("GlobalShortcuts portal: {support:?}");
    }
}
