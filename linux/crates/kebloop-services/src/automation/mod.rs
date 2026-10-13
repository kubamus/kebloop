//! Script commands, the v1 extension protocol and the subprocess runner.

mod extension;
mod process;
mod script;

pub use extension::*;
pub use process::*;
pub use script::*;

use crate::error::ConfigError;

/// Failures of automation execution and validation. Messages are user-presentable and match the
/// macOS app.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AutomationError {
    /// The run was cancelled through its [`CancelHandle`].
    #[error("Execution cancelled.")]
    Cancelled,
    /// The time limit elapsed.
    #[error("Execution exceeded its time limit. Increase the limit after reviewing the command.")]
    Timeout,
    /// Combined stdout and stderr exceeded the limit.
    #[error("Execution exceeded the output limit. Reduce the command's output.")]
    OutputLimit,
    /// The executable does not exist or is not executable.
    #[error("Executable or runtime is missing or not executable. Select a valid local executable.")]
    MissingExecutable,
    /// The process exited with a non-zero status (128 + n for signals).
    #[error("Execution exited with status {0}. Review the script or extension locally.")]
    Failed(i32),
    /// Invalid configuration, limits, input or protocol data.
    #[error("{0}")]
    Invalid(String),
    /// The process could not be started for another reason.
    #[error("Could not start the process: {0}")]
    Spawn(String),
    /// The script or extension is disabled and awaiting review.
    #[error("Review and enable this {0} in Settings first.")]
    NotEnabled(&'static str),
    /// The requested extension result or action is not part of the last response.
    #[error("This extension action is no longer available.")]
    ActionUnavailable,
}

impl From<AutomationError> for ConfigError {
    fn from(error: AutomationError) -> Self {
        ConfigError::Invalid(error.to_string())
    }
}

#[cfg(test)]
mod tests;
