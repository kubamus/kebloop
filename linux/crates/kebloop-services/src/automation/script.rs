//! Script commands: model, validation, import (always disabled) and execution.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::{
    AutomationError, AutomationOutput, AutomationRunner, DEFAULT_OUTPUT_LIMIT, ExecuteRequest,
};

fn default_timeout() -> f64 {
    10.0
}

/// A user-defined local command. Disabled until reviewed and enabled by the user.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScriptCommand {
    /// Stable identifier.
    pub id: String,
    /// Display title.
    pub title: String,
    /// Description.
    #[serde(default)]
    pub description: String,
    /// Absolute executable path.
    pub executable: String,
    /// Literal arguments.
    #[serde(default)]
    pub arguments: Vec<String>,
    /// Timeout in seconds (0 exclusive to 300 inclusive, default 10).
    #[serde(default = "default_timeout")]
    pub timeout: f64,
    /// Whether the user reviewed and enabled it (default false).
    #[serde(default)]
    pub enabled: bool,
}

impl ScriptCommand {
    /// New disabled command with the default timeout.
    pub fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        executable: impl Into<String>,
        arguments: Vec<String>,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            description: String::new(),
            executable: executable.into(),
            arguments,
            timeout: default_timeout(),
            enabled: false,
        }
    }

    /// Launcher command identifier (`script:<id>`).
    pub fn command_id(&self) -> String {
        format!("script:{}", self.id)
    }

    /// Check the model rules.
    pub fn validate(&self) -> Result<(), AutomationError> {
        let ok = !self.id.is_empty()
            && !self.title.trim().is_empty()
            && self.executable.starts_with('/')
            && !self.executable.contains('\0')
            && self.arguments.iter().all(|a| !a.contains('\0'))
            && self.timeout.is_finite()
            && self.timeout > 0.0
            && self.timeout <= 300.0;
        if ok {
            Ok(())
        } else {
            Err(AutomationError::Invalid(
                "Provide a title, absolute executable path, valid arguments, and a timeout from 0 to 300 seconds."
                    .into(),
            ))
        }
    }

    /// The timeout as a [`Duration`] (validated range).
    pub fn timeout_duration(&self) -> Duration {
        Duration::try_from_secs_f64(self.timeout).unwrap_or(Duration::from_secs(10))
    }
}

/// Insert or replace (by id) a validated command.
pub fn save_script(
    commands: &mut Vec<ScriptCommand>,
    command: ScriptCommand,
) -> Result<(), AutomationError> {
    command.validate()?;
    match commands.iter_mut().find(|c| c.id == command.id) {
        Some(existing) => *existing = command,
        None => commands.push(command),
    }
    Ok(())
}

/// Import commands atomically; every imported command ends up disabled.
pub fn import_scripts(
    commands: &mut Vec<ScriptCommand>,
    imported: Vec<ScriptCommand>,
) -> Result<(), AutomationError> {
    for command in &imported {
        command.validate()?;
    }
    for mut command in imported {
        command.enabled = false;
        save_script(commands, command)?;
    }
    Ok(())
}

/// Enable or disable a command; returns whether it exists.
pub fn set_script_enabled(commands: &mut [ScriptCommand], id: &str, enabled: bool) -> bool {
    match commands.iter_mut().find(|c| c.id == id) {
        Some(command) => {
            command.enabled = enabled;
            true
        }
        None => false,
    }
}

/// Remove a command by id.
pub fn remove_script(commands: &mut Vec<ScriptCommand>, id: &str) {
    commands.retain(|c| c.id != id);
}

/// Drop commands that no longer validate (used when loading stored data).
pub fn retain_valid_scripts(commands: &mut Vec<ScriptCommand>) {
    commands.retain(|c| c.validate().is_ok());
}

/// Run an enabled command with extra literal arguments appended. Blocking; call it from a worker
/// thread and keep the runner's cancel handle.
pub fn run_script(
    commands: &[ScriptCommand],
    id: &str,
    extra_arguments: &[String],
    runner: &AutomationRunner,
) -> Result<AutomationOutput, AutomationError> {
    let command = commands
        .iter()
        .find(|c| c.id == id && c.enabled)
        .ok_or(AutomationError::NotEnabled("script"))?;
    let mut arguments = command.arguments.clone();
    arguments.extend_from_slice(extra_arguments);
    runner.execute(
        &ExecuteRequest::new(command.executable.clone(), arguments)
            .with_timeout(command.timeout_duration())
            .with_output_limit(DEFAULT_OUTPUT_LIMIT),
    )
}
