//! Message types for device communication.
//!
//! This module defines the core message types that flow through ferredge:
//! - Commands: Requests to devices (read/write/execute)
//! - Readings: Data read from devices
//! - Responses: Results of command execution

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::device::DeviceId;
use crate::types::Value;

/// Unique identifier for a message.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MessageId(pub String);

impl MessageId {
    /// Generates a new random message ID.
    pub fn generate() -> Self {
        Self(Uuid::new_v4().to_string())
    }
}

impl Default for MessageId {
    fn default() -> Self {
        Self::generate()
    }
}

/// A generic message envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// Unique message identifier
    pub id: MessageId,
    /// Correlation ID for request-response matching
    pub correlation_id: Option<MessageId>,
    /// Message timestamp
    pub timestamp: DateTime<Utc>,
    /// The message payload
    pub payload: MessagePayload,
}

impl Message {
    /// Creates a new message with the given payload.
    pub fn new(payload: MessagePayload) -> Self {
        Self {
            id: MessageId::generate(),
            correlation_id: None,
            timestamp: Utc::now(),
            payload,
        }
    }

    /// Sets the correlation ID.
    pub fn with_correlation(mut self, correlation_id: MessageId) -> Self {
        self.correlation_id = Some(correlation_id);
        self
    }

    /// Creates a command message.
    pub fn command(command: Command) -> Self {
        Self::new(MessagePayload::Command(command))
    }

    /// Creates a reading message.
    pub fn reading(reading: Reading) -> Self {
        Self::new(MessagePayload::Reading(reading))
    }

    /// Creates a response message.
    pub fn response(response: CommandResponse) -> Self {
        Self::new(MessagePayload::Response(response))
    }
}

/// The payload of a message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MessagePayload {
    /// A command to be executed
    Command(Command),
    /// A reading from a device
    Reading(Reading),
    /// A response to a command
    Response(CommandResponse),
}

/// A command to be executed on a device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Command {
    /// Target device ID
    pub device_id: DeviceId,
    /// The operation to perform
    pub operation: CommandOperation,
    /// Command-specific parameters
    pub params: serde_json::Value,
}

impl Command {
    /// Creates a read command for a resource.
    pub fn read(device_id: DeviceId, resource: impl Into<String>) -> Self {
        Self {
            device_id,
            operation: CommandOperation::Read {
                resource: resource.into(),
            },
            params: serde_json::Value::Null,
        }
    }

    /// Creates a write command for a resource.
    pub fn write(device_id: DeviceId, resource: impl Into<String>, value: Value) -> Self {
        Self {
            device_id,
            operation: CommandOperation::Write {
                resource: resource.into(),
                value,
            },
            params: serde_json::Value::Null,
        }
    }

    /// Creates an execute command.
    pub fn execute(device_id: DeviceId, command_name: impl Into<String>, args: serde_json::Value) -> Self {
        Self {
            device_id,
            operation: CommandOperation::Execute {
                command: command_name.into(),
                args,
            },
            params: serde_json::Value::Null,
        }
    }
}

/// The specific operation a command performs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum CommandOperation {
    /// Read a resource value
    Read {
        resource: String,
    },
    /// Write a value to a resource
    Write {
        resource: String,
        value: Value,
    },
    /// Execute a named command
    Execute {
        command: String,
        args: serde_json::Value,
    },
}

/// A reading from a device resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reading {
    /// Source device ID
    pub device_id: DeviceId,
    /// Resource name
    pub resource: String,
    /// The value read
    pub value: Value,
    /// Reading timestamp
    pub timestamp: DateTime<Utc>,
    /// Origin timestamp (from device if available)
    pub origin: Option<DateTime<Utc>>,
    /// Optional unit of measurement
    pub unit: Option<String>,
}

impl Reading {
    /// Creates a new reading.
    pub fn new(device_id: DeviceId, resource: impl Into<String>, value: Value) -> Self {
        Self {
            device_id,
            resource: resource.into(),
            value,
            timestamp: Utc::now(),
            origin: None,
            unit: None,
        }
    }

    /// Sets the origin timestamp.
    pub fn with_origin(mut self, origin: DateTime<Utc>) -> Self {
        self.origin = Some(origin);
        self
    }

    /// Sets the unit.
    pub fn with_unit(mut self, unit: impl Into<String>) -> Self {
        self.unit = Some(unit.into());
        self
    }
}

/// Response to a command execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandResponse {
    /// Whether the command succeeded
    pub success: bool,
    /// Result value (for read operations)
    pub result: Option<Value>,
    /// Error message (if failed)
    pub error: Option<String>,
    /// Response timestamp
    pub timestamp: DateTime<Utc>,
}

impl CommandResponse {
    /// Creates a successful response with a value.
    pub fn ok(result: Value) -> Self {
        Self {
            success: true,
            result: Some(result),
            error: None,
            timestamp: Utc::now(),
        }
    }

    /// Creates a successful response without a value.
    pub fn ok_empty() -> Self {
        Self {
            success: true,
            result: None,
            error: None,
            timestamp: Utc::now(),
        }
    }

    /// Creates a failed response.
    pub fn err(error: impl Into<String>) -> Self {
        Self {
            success: false,
            result: None,
            error: Some(error.into()),
            timestamp: Utc::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_command_creation() {
        let device_id = DeviceId::new("device-1");
        let cmd = Command::read(device_id.clone(), "temperature");

        assert_eq!(cmd.device_id, device_id);
        assert!(matches!(cmd.operation, CommandOperation::Read { .. }));
    }

    #[test]
    fn test_reading_creation() {
        let device_id = DeviceId::new("sensor-1");
        let reading = Reading::new(device_id, "temperature", Value::Float(25.5))
            .with_unit("°C");

        assert_eq!(reading.resource, "temperature");
        assert_eq!(reading.unit, Some("°C".to_string()));
    }

    #[test]
    fn test_message_serialization() {
        let reading = Reading::new(
            DeviceId::new("device-1"),
            "temperature",
            Value::Float(25.5),
        );
        let message = Message::reading(reading);

        let json = serde_json::to_string(&message).unwrap();
        let parsed: Message = serde_json::from_str(&json).unwrap();

        assert_eq!(message.id, parsed.id);
    }
}
