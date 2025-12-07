use crate::device::DeviceId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CommandType {
    Get,
    Set,
    Action,
}

/// Represents a command sent to a device or the core.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Command {
    pub id: String,
    pub target_device_id: DeviceId,
    pub command_type: CommandType,
    pub resource: String,
    pub payload: Option<Vec<u8>>,
}

/// Represents the result of a command execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandResult {
    pub command_id: String,
    pub device_id: DeviceId,
    pub success: bool,
    pub data: Option<Vec<u8>>,
    pub error: Option<String>,
}
