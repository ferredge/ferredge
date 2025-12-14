use crate::{Command, device::DeviceProtocol};

pub trait ProtocolBridge: Send + Sync {
    /// Translate a command from source protocol to target protocol
    fn translate_command(
        &self,
        command: Command,
        source_protocol: DeviceProtocol,
        target_protocol: DeviceProtocol,
    ) -> impl Future<Output = Result<Command, String>> + Send;

    /// Check if translation between protocols is supported
    fn supports_translation(&self, from: &DeviceProtocol, to: &DeviceProtocol) -> bool;
}
