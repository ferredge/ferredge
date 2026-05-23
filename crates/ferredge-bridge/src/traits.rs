use ferredge_core::prelude::{Command, RoutedEvent, RoutedResult};

use crate::message::BridgeMessage;

/// Adapter contract between routed core messages and bridge messages.
pub trait BridgeAdapter {
    /// Adapter-specific error type.
    type Error;

    /// Converts an outbound core command into a bridge message.
    fn command_to_bridge(&self, command: &Command) -> Result<BridgeMessage, Self::Error>;
    /// Converts an inbound routed event into a bridge message.
    fn event_to_bridge(&self, event: &RoutedEvent) -> Result<BridgeMessage, Self::Error>;
    /// Converts an inbound routed result into a bridge message.
    fn result_to_bridge(&self, result: &RoutedResult) -> Result<BridgeMessage, Self::Error>;
}

/// Sink for bridge messages produced by planners or adapters.
pub trait BridgeEmitter {
    /// Sink-specific error type.
    type Error;

    /// Emits one bridge message.
    fn emit(&mut self, message: BridgeMessage) -> Result<(), Self::Error>;
}

/// Encode/decode boundary between bridge messages and one native protocol type.
pub trait BridgeCodec<TNative> {
    /// Codec-specific error type.
    type Error;

    /// Encodes one bridge message into the native protocol form.
    fn encode(&self, message: &BridgeMessage) -> Result<TNative, Self::Error>;
    /// Decodes one native protocol value into a bridge message.
    fn decode(&self, native: TNative) -> Result<BridgeMessage, Self::Error>;
}
