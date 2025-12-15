use crate::command::{Command, CommandResult};
use crate::device::{Device, DeviceId, DeviceResourceAttributes};

/// A generic message that flows through the system.
#[derive(Debug, Clone)]
pub enum Message {
    Command(Command),
    Event(DeviceEvent),
    Response(CommandResult),
}

#[derive(Debug, Clone)]
pub struct DeviceEvent {
    pub device_id: String,
    pub timestamp: u64,
    pub data: Vec<u8>,
}

/// The main trait for the Core logic.
/// It routes messages between the external world (API), Drivers, and Storage.

pub trait Router<T: DeviceResourceAttributes, Dr: Driver>: Send + Sync {
    /// Register a new device with the core.
    fn register_device(&self, device: Device<T, Dr>) -> impl Future<Output = Result<(), String>> + Send;

    /// Route a command to a specific device.
    fn route_command(
        &self,
        command: Command,
    ) -> impl Future<Output = Result<CommandResult, String>> + Send;

    /// Handle an incoming event from a device (e.g., sensor reading).
    /// This typically involves routing to storage or triggering other actions.
    fn handle_event(&self, event: DeviceEvent) -> impl Future<Output = Result<(), String>> + Send;

    /// Route a command from one device to another
    fn route_device_to_device(
        &self,
        source_device_id: DeviceId,
        command: Command,
    ) -> impl Future<Output = Result<CommandResult, String>> + Send;
}

/// Trait for Device Drivers to implement.
/// The Core uses this to communicate with the actual hardware/protocol.
pub trait Driver: Send + Sync {
    /// Execute a command on the hardware.
    fn execute(&self, command: Command) -> impl Future<Output = CommandResult> + Send;

    /// Start listening for events from the hardware.
    /// This usually spawns a task that feeds events back to the Core.
    fn start(&self) -> impl Future<Output = Result<(), String>> + Send;

    /// Stop the driver.
    fn stop(&self) -> impl Future<Output = Result<(), String>> + Send;
}
