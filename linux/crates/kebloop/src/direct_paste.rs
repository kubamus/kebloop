//! Direct paste (Swift: `PasteService`): after a copy the launcher hides and Kebloop sends the
//! paste chord to the window that was focused before the launcher opened. Everything here is the
//! GTK-free part: the setting, the decision whether and how to paste, the terminal chord and
//! the feedback texts. On every failure the copy stays on the clipboard and the user is told.

use std::collections::BTreeMap;

use kebloop_platform::compositor::Window;
use kebloop_platform::paste::{PasteChord, PasteError, PasteTarget, PasteTool};
use serde_json::Value;

use crate::i18n::{tr, trf};
use kebloop_services::l10n::FormatArg;

/// `settings.extra` key of the direct-paste switch.
pub const KEY: &str = "directPasteEnabled";
/// The key of earlier versions (snippet insertion only, off by default); still read.
pub const LEGACY_KEY: &str = "snippetPasteEnabled";

/// Whether direct paste is switched on. An explicit choice wins (the current key, then the
/// legacy one); without one it is on when a paste tool is installed, because Swift pastes by
/// default.
pub fn enabled(extra: &BTreeMap<String, Value>, tool_installed: bool) -> bool {
    [KEY, LEGACY_KEY]
        .iter()
        .find_map(|key| extra.get(*key).and_then(Value::as_bool))
        .unwrap_or(tool_installed)
}

/// Store the choice under the current key and drop the legacy one.
pub fn set_enabled(extra: &mut BTreeMap<String, Value>, on: bool) {
    extra.insert(KEY.to_owned(), Value::Bool(on));
    extra.remove(LEGACY_KEY);
}

/// Application ids of terminal emulators, which paste with Ctrl+Shift+V.
const TERMINALS: &[&str] = &[
    "alacritty",
    "com.mitchellh.ghostty",
    "com.raggesilver.blackbox",
    "contour",
    "cool-retro-term",
    "footclient",
    "foot",
    "ghostty",
    "gnome-terminal",
    "gnome-terminal-server",
    "kgx",
    "kitty",
    "konsole",
    "lxterminal",
    "org.gnome.console",
    "org.gnome.terminal",
    "org.kde.konsole",
    "org.wezfurlong.wezterm",
    "qterminal",
    "rio",
    "sakura",
    "st",
    "st-256color",
    "terminator",
    "tilix",
    "urxvt",
    "wezterm",
    "xfce4-terminal",
    "xterm",
];

/// The chord that pastes into a window of application `app_id`.
pub fn chord_for(app_id: Option<&str>) -> PasteChord {
    let id = app_id.unwrap_or_default().to_lowercase();
    if TERMINALS.contains(&id.as_str()) {
        PasteChord::CtrlShiftV
    } else {
        PasteChord::CtrlV
    }
}

/// Why nothing was pasted before any key was sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// Neither `wtype` nor `ydotool` is installed.
    NoTool,
    /// The user switched direct paste off.
    Disabled,
    /// The compositor is reachable but no window was focused before the launcher opened.
    NoTarget,
}

/// What to do after the copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// The tool that sends the keys.
    pub tool: PasteTool,
    /// The chord to send.
    pub chord: PasteChord,
    /// The window that must have focus (`None`: no compositor IPC, paste after a short delay).
    pub target: Option<PasteTarget>,
}

/// Decide whether and how to paste. `previous` is the window focused before the launcher opened
/// (never a Kebloop window); `compositor_known` tells whether the compositor answers at all.
pub fn plan(
    enabled: bool,
    tool: Option<PasteTool>,
    compositor_known: bool,
    previous: Option<&Window>,
) -> Result<Plan, Refusal> {
    let Some(tool) = tool else {
        return Err(Refusal::NoTool);
    };
    if !enabled {
        return Err(Refusal::Disabled);
    }
    match (compositor_known, previous) {
        (true, None) => Err(Refusal::NoTarget),
        (_, window) => Ok(Plan {
            tool,
            chord: chord_for(window.and_then(|w| w.app_id.as_deref())),
            target: window.map(|w| PasteTarget {
                window: w.id.clone(),
            }),
        }),
    }
}

/// The text of the "Direct Paste" alert for a refusal.
pub fn refusal_text(refusal: &Refusal) -> String {
    tr(match refusal {
        Refusal::NoTool => "Copied. Install wtype or ydotool to paste directly.",
        Refusal::Disabled => "Copied. Direct paste is turned off in Settings.",
        Refusal::NoTarget => "Copied. The previous window is no longer available.",
    })
}

/// The text of the alert for a failed paste; `None` when the paste was cancelled on purpose
/// (the launcher was opened again), which needs no feedback.
pub fn failure_text(error: &PasteError) -> Option<String> {
    Some(match error {
        PasteError::NoTool => refusal_text(&Refusal::NoTool),
        PasteError::Failed(program, reason) => trf(
            "Copied. Could not send the paste keys with %@: %@",
            &[
                FormatArg::Str((*program).to_owned()),
                FormatArg::Str(reason.clone()),
            ],
        ),
        PasteError::FocusChanged => {
            tr("Copied. Paste cancelled because the active window changed.")
        }
        PasteError::FocusTimeout => {
            tr("Copied. The previous window did not regain focus, so nothing was pasted.")
        }
        PasteError::FocusUnverifiable(reason) => trf(
            "Copied. Could not verify which window has focus: %@",
            &[FormatArg::Str(reason.clone())],
        ),
        PasteError::ClipboardChanged => tr("Paste cancelled because the clipboard changed."),
        PasteError::Cancelled => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use kebloop_platform::compositor::WindowId;

    fn window(id: &str, app_id: Option<&str>) -> Window {
        Window {
            id: WindowId(id.into()),
            app_id: app_id.map(Into::into),
            title: String::new(),
            pid: Some(7),
            focused: true,
            floating: false,
            fullscreen: false,
            hidden: false,
            output: None,
            workspace: None,
            frame: None,
            size: None,
            focus_rank: None,
        }
    }

    #[test]
    fn the_setting_defaults_to_the_tool_and_honours_explicit_choices() {
        let mut extra = BTreeMap::new();
        assert!(enabled(&extra, true));
        assert!(!enabled(&extra, false));
        extra.insert(LEGACY_KEY.to_owned(), Value::Bool(false));
        assert!(!enabled(&extra, true), "an explicit legacy choice is kept");
        extra.insert(LEGACY_KEY.to_owned(), Value::Bool(true));
        assert!(enabled(&extra, false));
        extra.insert(KEY.to_owned(), Value::Bool(false));
        assert!(
            !enabled(&extra, true),
            "the current key wins over the legacy one"
        );
        extra.insert(KEY.to_owned(), Value::String("yes".into()));
        extra.remove(LEGACY_KEY);
        assert!(enabled(&extra, true), "non-boolean values are ignored");
    }

    #[test]
    fn saving_migrates_to_the_single_key() {
        let mut extra = BTreeMap::new();
        extra.insert(LEGACY_KEY.to_owned(), Value::Bool(true));
        extra.insert("other".to_owned(), Value::Bool(true));
        set_enabled(&mut extra, false);
        assert_eq!(extra.get(KEY), Some(&Value::Bool(false)));
        assert!(!extra.contains_key(LEGACY_KEY));
        assert!(extra.contains_key("other"));
    }

    #[test]
    fn terminals_paste_with_shift() {
        assert_eq!(chord_for(Some("foot")), PasteChord::CtrlShiftV);
        assert_eq!(chord_for(Some("Alacritty")), PasteChord::CtrlShiftV);
        assert_eq!(chord_for(Some("org.gnome.Console")), PasteChord::CtrlShiftV);
        assert_eq!(chord_for(Some("firefox")), PasteChord::CtrlV);
        assert_eq!(chord_for(None), PasteChord::CtrlV);
    }

    #[test]
    fn plans_refuse_before_sending_anything() {
        let firefox = window("3", Some("firefox"));
        assert_eq!(
            plan(true, None, true, Some(&firefox)),
            Err(Refusal::NoTool),
            "a missing tool is reported even though the setting is on"
        );
        assert_eq!(
            plan(false, Some(PasteTool::Wtype), true, Some(&firefox)),
            Err(Refusal::Disabled)
        );
        assert_eq!(
            plan(true, Some(PasteTool::Wtype), true, None),
            Err(Refusal::NoTarget)
        );
        let planned = plan(true, Some(PasteTool::Ydotool), true, Some(&firefox));
        assert_eq!(
            planned,
            Ok(Plan {
                tool: PasteTool::Ydotool,
                chord: PasteChord::CtrlV,
                target: Some(PasteTarget {
                    window: WindowId("3".into())
                }),
            })
        );
    }

    #[test]
    fn without_compositor_ipc_the_paste_is_blind() {
        let planned = plan(true, Some(PasteTool::Wtype), false, None);
        assert_eq!(
            planned,
            Ok(Plan {
                tool: PasteTool::Wtype,
                chord: PasteChord::CtrlV,
                target: None,
            })
        );
    }

    #[test]
    fn terminal_targets_use_the_shifted_chord() {
        let terminal = window("9", Some("kitty"));
        let planned = plan(true, Some(PasteTool::Wtype), true, Some(&terminal));
        assert_eq!(planned.map(|p| p.chord), Ok(PasteChord::CtrlShiftV));
    }

    #[test]
    fn feedback_exists_for_every_failure_except_cancellation() {
        assert!(failure_text(&PasteError::Cancelled).is_none());
        for error in [
            PasteError::NoTool,
            PasteError::Failed("wtype", "no seat".into()),
            PasteError::FocusChanged,
            PasteError::FocusTimeout,
            PasteError::FocusUnverifiable("x".into()),
            PasteError::ClipboardChanged,
        ] {
            assert!(failure_text(&error).is_some_and(|t| !t.is_empty()));
        }
        for refusal in [Refusal::NoTool, Refusal::Disabled, Refusal::NoTarget] {
            assert!(refusal_text(&refusal).starts_with("Copied."));
        }
    }
}
