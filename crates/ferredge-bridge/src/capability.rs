use serde::{Deserialize, Serialize};

/// Capability family expressed by a bridge message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum BridgeCapability {
    /// Topic or channel oriented message exchange.
    Messaging(MessagingCapability),
    /// Request/response style resource interaction.
    RequestResponse(RequestResponseCapability),
    /// Register and field oriented access.
    RegisterAccess(RegisterAccessCapability),
    /// Reserved space for future binary stream semantics.
    BinaryStreamReserved,
    /// Reserved space for future session lifecycle semantics.
    SessionStreamReserved,
}

/// Current messaging capability flags.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MessagingCapability {
    /// Whether the capability accepts opaque binary payloads without serialization.
    pub binary_payloads: bool,
}

/// Current request/response capability flags.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RequestResponseCapability {
    /// Whether the capability accepts opaque binary payloads without serialization.
    pub binary_payloads: bool,
}

/// Current register-access capability flags.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RegisterAccessCapability {
    /// Whether the capability accepts opaque binary payloads without serialization.
    pub binary_payloads: bool,
}
