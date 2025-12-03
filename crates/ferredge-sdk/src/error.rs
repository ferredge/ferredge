//! SDK-specific error types.

use thiserror::Error;

/// SDK-specific result type.
pub type SdkResult<T> = Result<T, SdkError>;

/// SDK error type.
#[derive(Debug, Error)]
pub enum SdkError {
    /// Connection error
    #[error("Connection error: {0}")]
    Connection(String),

    /// Device not found
    #[error("Device not found: {0}")]
    DeviceNotFound(String),

    /// Command failed
    #[error("Command failed: {0}")]
    CommandFailed(String),

    /// Timeout
    #[error("Operation timed out")]
    Timeout,

    /// Serialization error
    #[error("Serialization error: {0}")]
    Serialization(String),

    /// Core error
    #[error("Core error: {0}")]
    Core(#[from] ferredge_core::Error),

    /// Channel closed
    #[error("Channel closed")]
    ChannelClosed,
}

impl From<serde_json::Error> for SdkError {
    fn from(err: serde_json::Error) -> Self {
        SdkError::Serialization(err.to_string())
    }
}
