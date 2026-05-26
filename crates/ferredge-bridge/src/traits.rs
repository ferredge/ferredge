use ferredge_core::prelude::Command;

/// Marker for planning or encoding via the optional bridge IR.
#[derive(Debug, Clone, Copy, Default)]
pub struct BridgeOutbound;

/// Marker for planning or encoding directly through protocol-native semantics.
#[derive(Debug, Clone, Copy, Default)]
pub struct NativeOutbound;

/// Outbound planning contract from routed commands into protocol-specific planned messages.
pub trait ProtocolPlanner<TTarget, TPlanned> {
    /// Planner-specific error type.
    type Error;

    /// Converts one outbound core command into a protocol-planned message.
    fn plan(&self, command: Command) -> Result<TPlanned, Self::Error>;
}

/// Encode boundary between planned outbound messages and one native protocol type.
pub trait ProtocolEncoder<TTarget, TPlanned, TNative> {
    /// Codec-specific error type.
    type Error;

    /// Encodes one planned outbound message into the native protocol form.
    fn encode(&self, planned: TPlanned) -> Result<TNative, Self::Error>;
}

/// Inbound decode boundary between one native protocol type and one decoded semantic view.
pub trait ProtocolDecoder<TNative> {
    /// Decoder-specific error type.
    type Error;
    /// Decoded inbound semantic view produced for one native value.
    type Decoded<'a>
    where
        TNative: 'a;

    /// Decodes one native protocol value into a protocol-specific semantic view.
    fn decode<'a>(&self, native: &'a TNative) -> Result<Self::Decoded<'a>, Self::Error>;
}
