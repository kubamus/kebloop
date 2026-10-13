//! Shortcut binding model. Keys are GTK accelerator strings (`<Alt>space`); macOS Carbon codes
//! from imported configurations are mapped where possible and otherwise kept as "unavailable".

use serde::{Deserialize, Serialize};

use crate::accelerator::Accelerator;
use crate::config::PortableSettings;
use crate::error::ConfigError;

/// Mask of the Carbon modifiers that count as "command" modifiers (cmd, option, control).
const CARBON_COMMAND_MASK: u32 = 0x1900;

/// Persisted launcher shortcut choice. `keyCode` / `modifiers` are the macOS Carbon values and are
/// optional so Linux-created shortcuts need not have a macOS equivalent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Shortcut {
    /// Stable identifier (`option`, `control`, `commandShift`, `command`, or custom).
    pub id: String,
    /// Human readable label.
    pub label: String,
    /// macOS Carbon modifier mask.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modifiers: Option<u32>,
    /// macOS Carbon virtual key code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_code: Option<u32>,
    /// GTK accelerator, preferred over the Carbon values when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accelerator: Option<String>,
}

impl Shortcut {
    /// The built-in launcher shortcut choices (same ids as the macOS app).
    pub fn choices() -> Vec<Shortcut> {
        let make = |id: &str, label: &str, modifiers: u32, accel: &str| Shortcut {
            id: id.to_string(),
            label: label.to_string(),
            modifiers: Some(modifiers),
            key_code: Some(49),
            accelerator: Some(accel.to_string()),
        };
        vec![
            make("option", "Alt+Space", 0x800, "<Alt>space"),
            make("control", "Ctrl+Space", 0x1000, "<Control>space"),
            make(
                "commandShift",
                "Super+Shift+Space",
                0x300,
                "<Shift><Super>space",
            ),
            make("command", "Super+Space", 0x100, "<Super>space"),
        ]
    }

    /// Resolve to an accelerator when one is known on Linux.
    pub fn resolved(&self) -> Option<Accelerator> {
        resolve(self.key_code, self.modifiers, self.accelerator.as_deref())
    }
}

/// Application launch shortcut. `appPath` holds a desktop-entry id or absolute path on Linux and a
/// bundle path for configurations imported from macOS.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppShortcutBinding {
    /// Identifier (equal to `appPath`).
    pub id: String,
    /// Application identity.
    pub app_path: String,
    /// Display name.
    pub app_name: String,
    /// macOS Carbon key code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_code: Option<u32>,
    /// macOS Carbon modifier mask.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modifiers: Option<u32>,
    /// Display label.
    pub label: String,
    /// GTK accelerator.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accelerator: Option<String>,
}

impl AppShortcutBinding {
    /// Resolve to a Linux accelerator, if representable.
    pub fn resolved(&self) -> Option<Accelerator> {
        resolve(self.key_code, self.modifiers, self.accelerator.as_deref())
    }
}

/// Launcher command shortcut (command ids are `LauncherCommand` ids, `snippet:…`, `script:…`, …).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandShortcutBinding {
    /// Command identifier.
    pub id: String,
    /// macOS Carbon key code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_code: Option<u32>,
    /// macOS Carbon modifier mask.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modifiers: Option<u32>,
    /// Display label.
    pub label: String,
    /// GTK accelerator.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accelerator: Option<String>,
}

impl CommandShortcutBinding {
    /// Resolve to a Linux accelerator, if representable.
    pub fn resolved(&self) -> Option<Accelerator> {
        resolve(self.key_code, self.modifiers, self.accelerator.as_deref())
    }
}

/// Who holds a shortcut; used for conflict detection and enabled-state callbacks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShortcutOwner {
    /// The launcher toggle shortcut.
    Launcher,
    /// An application shortcut, by `appPath`.
    App(String),
    /// A command shortcut, by command id.
    Command(String),
}

/// Reasons a shortcut cannot be assigned. Messages are user-presentable.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ShortcutError {
    /// The accelerator string could not be parsed.
    #[error("Invalid shortcut: {0}")]
    Invalid(String),
    /// No Control, Alt or Super modifier.
    #[error("Include Control, Alt, or Super.")]
    NeedsModifier,
    /// Same as the launcher shortcut.
    #[error("This shortcut opens Kebloop. Choose a different shortcut.")]
    LauncherConflict,
    /// Held by another application binding.
    #[error("Already assigned to {0}.")]
    AssignedToApp(String),
    /// Held by another application or command.
    #[error("This shortcut is already assigned to an app or command.")]
    AssignedElsewhere,
}

pub(crate) fn resolve(
    key_code: Option<u32>,
    modifiers: Option<u32>,
    accelerator: Option<&str>,
) -> Option<Accelerator> {
    if let Some(text) = accelerator {
        return Accelerator::parse(text).ok();
    }
    Accelerator::from_carbon(key_code?, modifiers.unwrap_or(0))
}

/// Identity string used to detect duplicates, including bindings that cannot be mapped to Linux.
pub(crate) fn identity(
    key_code: Option<u32>,
    modifiers: Option<u32>,
    accelerator: Option<&str>,
) -> Option<String> {
    if let Some(accel) = resolve(key_code, modifiers, accelerator) {
        return Some(accel.identity());
    }
    key_code.map(|code| format!("carbon:{code}:{}", modifiers.unwrap_or(0)))
}

/// Validate one stored chord: accelerator syntax, Carbon ranges and the modifier requirement.
pub(crate) fn validate_chord(
    key_code: Option<u32>,
    modifiers: Option<u32>,
    accelerator: Option<&str>,
    what: &str,
) -> Result<(), ConfigError> {
    if accelerator.is_none() && key_code.is_none() {
        return Err(ConfigError::invalid(format!("{what} has no key.")));
    }
    if let Some(text) = accelerator {
        let parsed =
            Accelerator::parse(text).map_err(|e| ConfigError::invalid(format!("{what}: {e}")))?;
        if !parsed.has_command_modifier() {
            return Err(ConfigError::invalid(
                "Shortcuts must include Control, Alt, or Super.",
            ));
        }
    }
    if let Some(code) = key_code
        && (code > 127 || modifiers.unwrap_or(0) & CARBON_COMMAND_MASK == 0)
    {
        return Err(ConfigError::invalid(
            "Shortcuts must include Command, Option, or Control.",
        ));
    }
    Ok(())
}

fn launcher_identity(settings: &PortableSettings) -> Option<String> {
    let shortcut = settings.launcher_shortcut()?;
    identity(
        shortcut.key_code,
        shortcut.modifiers,
        shortcut.accelerator.as_deref(),
    )
}

/// Find an enabled owner (other than `excluding`) that already holds `chord`.
pub fn find_conflict(
    settings: &PortableSettings,
    chord: &Accelerator,
    excluding: &ShortcutOwner,
    enabled: &dyn Fn(&ShortcutOwner) -> bool,
) -> Option<ShortcutOwner> {
    let wanted = chord.identity();
    if *excluding != ShortcutOwner::Launcher
        && launcher_identity(settings).as_deref() == Some(&wanted)
    {
        return Some(ShortcutOwner::Launcher);
    }
    for binding in &settings.app_shortcuts {
        let owner = ShortcutOwner::App(binding.app_path.clone());
        if owner != *excluding
            && enabled(&owner)
            && identity(
                binding.key_code,
                binding.modifiers,
                binding.accelerator.as_deref(),
            )
            .as_deref()
                == Some(&wanted)
        {
            return Some(owner);
        }
    }
    for binding in &settings.command_shortcuts {
        let owner = ShortcutOwner::Command(binding.id.clone());
        if owner != *excluding
            && enabled(&owner)
            && identity(
                binding.key_code,
                binding.modifiers,
                binding.accelerator.as_deref(),
            )
            .as_deref()
                == Some(&wanted)
        {
            return Some(owner);
        }
    }
    None
}

fn parse_assignable(accelerator: &str) -> Result<Accelerator, ShortcutError> {
    let parsed =
        Accelerator::parse(accelerator).map_err(|e| ShortcutError::Invalid(e.to_string()))?;
    if parsed.has_command_modifier() {
        Ok(parsed)
    } else {
        Err(ShortcutError::NeedsModifier)
    }
}

/// Assign (or replace) the shortcut of an application.
pub fn assign_app_shortcut(
    settings: &mut PortableSettings,
    app_path: &str,
    app_name: &str,
    accelerator: &str,
    label: &str,
    enabled: &dyn Fn(&ShortcutOwner) -> bool,
) -> Result<(), ShortcutError> {
    let chord = parse_assignable(accelerator)?;
    let owner = ShortcutOwner::App(app_path.to_string());
    match find_conflict(settings, &chord, &owner, enabled) {
        Some(ShortcutOwner::Launcher) => return Err(ShortcutError::LauncherConflict),
        Some(ShortcutOwner::App(path)) => {
            let name = settings
                .app_shortcuts
                .iter()
                .find(|b| b.app_path == path)
                .map(|b| b.app_name.clone())
                .unwrap_or(path);
            return Err(ShortcutError::AssignedToApp(name));
        }
        Some(ShortcutOwner::Command(_)) => return Err(ShortcutError::AssignedElsewhere),
        None => {}
    }
    let (key_code, modifiers) = chord.to_carbon().unzip();
    let binding = AppShortcutBinding {
        id: app_path.to_string(),
        app_path: app_path.to_string(),
        app_name: app_name.to_string(),
        key_code,
        modifiers,
        label: label.to_string(),
        accelerator: Some(chord.to_string()),
    };
    match settings
        .app_shortcuts
        .iter_mut()
        .find(|b| b.app_path == app_path)
    {
        Some(existing) => *existing = binding,
        None => settings.app_shortcuts.push(binding),
    }
    Ok(())
}

/// Assign (or replace) the shortcut of a launcher command.
pub fn assign_command_shortcut(
    settings: &mut PortableSettings,
    command_id: &str,
    accelerator: &str,
    label: &str,
    enabled: &dyn Fn(&ShortcutOwner) -> bool,
) -> Result<(), ShortcutError> {
    let chord = parse_assignable(accelerator)?;
    let owner = ShortcutOwner::Command(command_id.to_string());
    match find_conflict(settings, &chord, &owner, enabled) {
        Some(ShortcutOwner::Launcher) => return Err(ShortcutError::LauncherConflict),
        Some(_) => return Err(ShortcutError::AssignedElsewhere),
        None => {}
    }
    let (key_code, modifiers) = chord.to_carbon().unzip();
    let binding = CommandShortcutBinding {
        id: command_id.to_string(),
        key_code,
        modifiers,
        label: label.to_string(),
        accelerator: Some(chord.to_string()),
    };
    match settings
        .command_shortcuts
        .iter_mut()
        .find(|b| b.id == command_id)
    {
        Some(existing) => *existing = binding,
        None => settings.command_shortcuts.push(binding),
    }
    Ok(())
}

/// Choose one of the built-in launcher shortcuts (`option`, `control`, ...) or `custom` (which
/// needs a recorded custom shortcut). Fails when another binding already uses the chord.
pub fn choose_launcher_shortcut(
    settings: &mut PortableSettings,
    id: &str,
) -> Result<(), ShortcutError> {
    let candidate = if id == "custom" {
        settings.launcher_custom_shortcut.clone()
    } else {
        Shortcut::choices().into_iter().find(|s| s.id == id)
    };
    let Some(shortcut) = candidate else {
        return Err(ShortcutError::Invalid(id.to_string()));
    };
    if let Some(chord) = shortcut.resolved() {
        check_launcher_conflict(settings, &chord)?;
    }
    settings.launcher_shortcut_id = id.to_string();
    Ok(())
}

/// Record a custom launcher shortcut from a GTK accelerator and select it.
pub fn assign_launcher_shortcut(
    settings: &mut PortableSettings,
    accelerator: &str,
    label: &str,
) -> Result<(), ShortcutError> {
    let chord = parse_assignable(accelerator)?;
    check_launcher_conflict(settings, &chord)?;
    settings.launcher_custom_shortcut = Some(Shortcut {
        id: "custom".to_string(),
        label: label.to_string(),
        modifiers: None,
        key_code: None,
        accelerator: Some(chord.to_string()),
    });
    settings.launcher_shortcut_id = "custom".to_string();
    Ok(())
}

fn check_launcher_conflict(
    settings: &PortableSettings,
    chord: &Accelerator,
) -> Result<(), ShortcutError> {
    match find_conflict(settings, chord, &ShortcutOwner::Launcher, &|_| true) {
        None => Ok(()),
        Some(ShortcutOwner::App(path)) => {
            let name = settings
                .app_shortcuts
                .iter()
                .find(|b| b.app_path == path)
                .map(|b| b.app_name.clone())
                .unwrap_or(path);
            Err(ShortcutError::AssignedToApp(name))
        }
        Some(_) => Err(ShortcutError::AssignedElsewhere),
    }
}

/// Remove an application shortcut; returns whether one existed.
pub fn remove_app_shortcut(settings: &mut PortableSettings, id: &str) -> bool {
    let before = settings.app_shortcuts.len();
    settings.app_shortcuts.retain(|b| b.id != id);
    settings.app_shortcuts.len() != before
}

/// Remove a command shortcut; returns whether one existed.
pub fn remove_command_shortcut(settings: &mut PortableSettings, id: &str) -> bool {
    let before = settings.command_shortcuts.len();
    settings.command_shortcuts.retain(|b| b.id != id);
    settings.command_shortcuts.len() != before
}

/// Bindings (by owner) whose key cannot be represented on Linux (unmapped macOS key codes).
pub fn unavailable_bindings(settings: &PortableSettings) -> Vec<(ShortcutOwner, String)> {
    let mut out = Vec::new();
    if let Some(shortcut) = settings.launcher_shortcut()
        && shortcut.resolved().is_none()
    {
        out.push((ShortcutOwner::Launcher, shortcut.label));
    }
    for b in &settings.app_shortcuts {
        if b.resolved().is_none() {
            out.push((ShortcutOwner::App(b.app_path.clone()), b.label.clone()));
        }
    }
    for b in &settings.command_shortcuts {
        if b.resolved().is_none() {
            out.push((ShortcutOwner::Command(b.id.clone()), b.label.clone()));
        }
    }
    out
}
