//! GTK-accelerator-style key representation (`<Alt>space`) and conversion from macOS Carbon codes.

use std::fmt;

/// Shift modifier bit.
pub const SHIFT: u8 = 1;
/// Control modifier bit.
pub const CONTROL: u8 = 2;
/// Alt (macOS Option) modifier bit.
pub const ALT: u8 = 4;
/// Super (macOS Command) modifier bit.
pub const SUPER: u8 = 8;

const CARBON_CMD: u32 = 0x100;
const CARBON_SHIFT: u32 = 0x200;
const CARBON_OPTION: u32 = 0x800;
const CARBON_CONTROL: u32 = 0x1000;

/// Carbon virtual key codes (ANSI layout) paired with GDK key names.
const KEYS: &[(u32, &str)] = &[
    (0, "a"),
    (1, "s"),
    (2, "d"),
    (3, "f"),
    (4, "h"),
    (5, "g"),
    (6, "z"),
    (7, "x"),
    (8, "c"),
    (9, "v"),
    (11, "b"),
    (12, "q"),
    (13, "w"),
    (14, "e"),
    (15, "r"),
    (16, "y"),
    (17, "t"),
    (18, "1"),
    (19, "2"),
    (20, "3"),
    (21, "4"),
    (22, "6"),
    (23, "5"),
    (24, "equal"),
    (25, "9"),
    (26, "7"),
    (27, "minus"),
    (28, "8"),
    (29, "0"),
    (30, "bracketright"),
    (31, "o"),
    (32, "u"),
    (33, "bracketleft"),
    (34, "i"),
    (35, "p"),
    (36, "Return"),
    (37, "l"),
    (38, "j"),
    (39, "apostrophe"),
    (40, "k"),
    (41, "semicolon"),
    (42, "backslash"),
    (43, "comma"),
    (44, "slash"),
    (45, "n"),
    (46, "m"),
    (47, "period"),
    (48, "Tab"),
    (49, "space"),
    (50, "grave"),
    (51, "BackSpace"),
    (53, "Escape"),
    (64, "F17"),
    (79, "F18"),
    (80, "F19"),
    (90, "F20"),
    (96, "F5"),
    (97, "F6"),
    (98, "F7"),
    (99, "F3"),
    (100, "F8"),
    (101, "F9"),
    (103, "F11"),
    (105, "F13"),
    (106, "F16"),
    (107, "F14"),
    (109, "F10"),
    (111, "F12"),
    (113, "F15"),
    (115, "Home"),
    (116, "Page_Up"),
    (117, "Delete"),
    (118, "F4"),
    (119, "End"),
    (120, "F2"),
    (121, "Page_Down"),
    (122, "F1"),
    (123, "Left"),
    (124, "Right"),
    (125, "Down"),
    (126, "Up"),
];

/// Key names that are valid on Linux but have no Carbon counterpart in [`KEYS`].
const EXTRA_KEYS: &[&str] = &[
    "Insert", "F21", "F22", "F23", "F24", "KP_Enter", "Print", "Menu",
];

/// Errors from parsing an accelerator string.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AcceleratorError {
    /// The string was empty or had no key.
    #[error("The shortcut has no key.")]
    MissingKey,
    /// An unknown `<Modifier>` was used.
    #[error("Unknown shortcut modifier: {0}")]
    UnknownModifier(String),
    /// The key name contains unsupported characters.
    #[error("Invalid shortcut key: {0}")]
    InvalidKey(String),
}

/// A parsed accelerator: modifier set plus a canonical key name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Accelerator {
    modifiers: u8,
    key: String,
}

impl Accelerator {
    /// Parse GTK accelerator syntax such as `<Control><Alt>space` (case-insensitive modifiers).
    pub fn parse(text: &str) -> Result<Self, AcceleratorError> {
        let mut rest = text.trim();
        let mut modifiers = 0u8;
        while let Some(stripped) = rest.strip_prefix('<') {
            let Some(end) = stripped.find('>') else {
                return Err(AcceleratorError::InvalidKey(text.to_string()));
            };
            let name = &stripped[..end];
            modifiers |= match name.to_ascii_lowercase().as_str() {
                "shift" => SHIFT,
                "control" | "ctrl" | "primary" => CONTROL,
                "alt" | "mod1" => ALT,
                "super" | "mod4" => SUPER,
                _ => return Err(AcceleratorError::UnknownModifier(name.to_string())),
            };
            rest = &stripped[end + 1..];
        }
        if rest.is_empty() {
            return Err(AcceleratorError::MissingKey);
        }
        Ok(Self {
            modifiers,
            key: canonical_key(rest)?,
        })
    }

    /// Build from raw parts (key names are canonicalised).
    pub fn new(modifiers: u8, key: &str) -> Result<Self, AcceleratorError> {
        Ok(Self {
            modifiers: modifiers & (SHIFT | CONTROL | ALT | SUPER),
            key: canonical_key(key)?,
        })
    }

    /// The default launcher accelerator, `<Alt>space`.
    pub fn default_launcher() -> Self {
        Self {
            modifiers: ALT,
            key: "space".to_string(),
        }
    }

    /// Modifier bit set.
    pub fn modifiers(&self) -> u8 {
        self.modifiers
    }

    /// Canonical key name.
    pub fn key(&self) -> &str {
        &self.key
    }

    /// Global shortcuts need Control, Alt or Super (the analogue of Command/Option/Control).
    pub fn has_command_modifier(&self) -> bool {
        self.modifiers & (CONTROL | ALT | SUPER) != 0
    }

    /// Case-insensitive identity used for conflict detection.
    pub fn identity(&self) -> String {
        format!("{}:{}", self.modifiers, self.key.to_ascii_lowercase())
    }

    /// Convert a macOS Carbon key code and modifier mask. Returns `None` for unmapped key codes.
    pub fn from_carbon(key_code: u32, modifiers: u32) -> Option<Self> {
        let (_, name) = KEYS.iter().find(|(code, _)| *code == key_code)?;
        let mut mask = 0u8;
        if modifiers & CARBON_SHIFT != 0 {
            mask |= SHIFT;
        }
        if modifiers & CARBON_CONTROL != 0 {
            mask |= CONTROL;
        }
        if modifiers & CARBON_OPTION != 0 {
            mask |= ALT;
        }
        if modifiers & CARBON_CMD != 0 {
            mask |= SUPER;
        }
        Some(Self {
            modifiers: mask,
            key: (*name).to_string(),
        })
    }

    /// Convert back to Carbon `(keyCode, modifiers)` where the key exists on macOS.
    pub fn to_carbon(&self) -> Option<(u32, u32)> {
        let (code, _) = KEYS
            .iter()
            .find(|(_, name)| name.eq_ignore_ascii_case(&self.key))?;
        let mut mask = 0u32;
        if self.modifiers & SHIFT != 0 {
            mask |= CARBON_SHIFT;
        }
        if self.modifiers & CONTROL != 0 {
            mask |= CARBON_CONTROL;
        }
        if self.modifiers & ALT != 0 {
            mask |= CARBON_OPTION;
        }
        if self.modifiers & SUPER != 0 {
            mask |= CARBON_CMD;
        }
        Some((*code, mask))
    }
}

impl fmt::Display for Accelerator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.modifiers & SHIFT != 0 {
            f.write_str("<Shift>")?;
        }
        if self.modifiers & CONTROL != 0 {
            f.write_str("<Control>")?;
        }
        if self.modifiers & ALT != 0 {
            f.write_str("<Alt>")?;
        }
        if self.modifiers & SUPER != 0 {
            f.write_str("<Super>")?;
        }
        f.write_str(&self.key)
    }
}

fn canonical_key(raw: &str) -> Result<String, AcceleratorError> {
    if raw.chars().count() == 1 {
        let c = raw.chars().next().unwrap_or(' ');
        if c.is_ascii_alphabetic() {
            return Ok(c.to_ascii_lowercase().to_string());
        }
    }
    if let Some((_, name)) = KEYS.iter().find(|(_, n)| n.eq_ignore_ascii_case(raw)) {
        return Ok((*name).to_string());
    }
    if let Some(name) = EXTRA_KEYS.iter().find(|n| n.eq_ignore_ascii_case(raw)) {
        return Ok((*name).to_string());
    }
    if raw.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        Ok(raw.to_string())
    } else {
        Err(AcceleratorError::InvalidKey(raw.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_formats_canonically() {
        let accel = Accelerator::parse("<alt><Ctrl>SPACE").unwrap();
        assert_eq!(accel.to_string(), "<Control><Alt>space");
        assert!(accel.has_command_modifier());
        assert!(
            !Accelerator::parse("<Shift>a")
                .unwrap()
                .has_command_modifier()
        );
        assert_eq!(
            Accelerator::parse("<Alt>"),
            Err(AcceleratorError::MissingKey)
        );
        assert!(Accelerator::parse("<Hyper>a").is_err());
    }

    #[test]
    fn carbon_round_trip() {
        // Option+Space on macOS.
        let accel = Accelerator::from_carbon(49, 0x800).unwrap();
        assert_eq!(accel.to_string(), "<Alt>space");
        assert_eq!(accel.to_carbon(), Some((49, 0x800)));
        // Command+Shift+F19.
        let accel = Accelerator::from_carbon(80, 0x300).unwrap();
        assert_eq!(accel.to_string(), "<Shift><Super>F19");
        assert_eq!(Accelerator::from_carbon(200, 0x100), None);
    }
}
