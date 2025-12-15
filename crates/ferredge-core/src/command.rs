use crate::device::DeviceId;
use serde::{Deserialize, Serialize};

pub type CommandId = String;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CommandType {
    Get,
    Set,
    Execute,
}

/// Represents a command sent to a device or the core.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Command {
    pub id: CommandId,
    pub target_device_id: DeviceId,
    pub command_type: CommandType,
    pub resource: String,
    pub payload: Option<Vec<u8>>,
}

/// Represents the result of a command execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandResult {
    pub command_id: CommandId,
    pub device_id: DeviceId,
    pub res: Result<Option<Vec<u8>>, String>,
}
