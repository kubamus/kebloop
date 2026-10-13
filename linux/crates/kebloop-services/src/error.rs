//! Shared error types.

use std::io;
use std::path::PathBuf;

/// Errors raised by configuration, validation and persistence code.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// A validation rule failed. The message is user-presentable.
    #[error("{0}")]
    Invalid(String),
    /// Reading or writing a file failed.
    #[error("{} could not be accessed: {source}", path.display())]
    Io {
        /// File or directory involved.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: io::Error,
    },
    /// JSON could not be encoded.
    #[error("serialization failed: {0}")]
    Serialize(#[from] serde_json::Error),
}

impl ConfigError {
    /// Shorthand for [`ConfigError::Invalid`].
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid(message.into())
    }

    /// Wrap an I/O error together with the path it concerns.
    pub fn io(path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}
