//! Core error types for ferredge.

use thiserror::Error;

/// Result type alias using ferredge's Error type.
pub type Result<T> = std::result::Result<T, Error>;

/// Core error type for ferredge operations.
#[derive(Debug, Error)]
pub enum Error {
    /// Device-related errors
    #[error("Device error: {0}")]
    Device(#[from] DeviceError),

    /// Driver-related errors
    #[error("Driver error: {0}")]
    Driver(#[from] DriverError),

    /// Storage-related errors
    #[error("Storage error: {0}")]
    Storage(#[from] StorageError),

    /// Routing errors
    #[error("Routing error: {0}")]
    Routing(#[from] RoutingError),

    /// Command execution errors
    #[error("Command error: {0}")]
    Command(String),

    /// Serialization/deserialization errors
    #[error("Serialization error: {0}")]
    Serialization(String),

    /// Channel communication errors
    #[error("Channel error: {0}")]
    Channel(String),

    /// Timeout errors
    #[error("Operation timed out: {0}")]
    Timeout(String),

    /// Generic internal error
    #[error("Internal error: {0}")]
    Internal(String),
}

/// Device-specific errors.
#[derive(Debug, Error)]
pub enum DeviceError {
    #[error("Device not found: {0}")]
    NotFound(String),

    #[error("Device already exists: {0}")]
    AlreadyExists(String),

    #[error("Device is offline: {0}")]
    Offline(String),

    #[error("Device resource not found: {resource} on device {device_id}")]
    ResourceNotFound { device_id: String, resource: String },

    #[error("Invalid device state: expected {expected}, got {actual}")]
    InvalidState { expected: String, actual: String },

    #[error("Device operation failed: {0}")]
    OperationFailed(String),
}

/// Driver-specific errors.
#[derive(Debug, Error)]
pub enum DriverError {
    #[error("Driver not found: {0}")]
    NotFound(String),

    #[error("Driver initialization failed: {0}")]
    InitializationFailed(String),

    #[error("Protocol error: {0}")]
    Protocol(String),

    #[error("Connection failed: {0}")]
    Connection(String),

    #[error("Driver is busy")]
    Busy,
}

/// Storage-specific errors.
#[derive(Debug, Error)]
pub enum StorageError {
    #[error("Storage backend not available: {0}")]
    NotAvailable(String),

    #[error("Write failed: {0}")]
    WriteFailed(String),

    #[error("Read failed: {0}")]
    ReadFailed(String),

    #[error("Query failed: {0}")]
    QueryFailed(String),

    #[error("Connection error: {0}")]
    Connection(String),
}

/// Routing-specific errors.
#[derive(Debug, Error)]
pub enum RoutingError {
    #[error("No route found for target: {0}")]
    NoRoute(String),

    #[error("Invalid route configuration: {0}")]
    InvalidConfiguration(String),

    #[error("Route handler not registered: {0}")]
    HandlerNotRegistered(String),
}

impl From<serde_json::Error> for Error {
    fn from(err: serde_json::Error) -> Self {
        Error::Serialization(err.to_string())
    }
}
