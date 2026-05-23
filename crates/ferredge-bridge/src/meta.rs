use alloc::string::String;

use ferredge_core::prelude::Address;
use serde::{Deserialize, Serialize};

use crate::capability::BridgeProtocolHint;

/// Compact typed metadata carried alongside bridge operations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct BridgeMeta {
    /// Logical resource name.
    pub resource: Option<String>,
    /// Path hint for resource-oriented protocols.
    pub path: Option<String>,
    /// Topic hint for message-oriented protocols.
    pub topic: Option<String>,
    /// Register metadata for register-oriented protocols.
    pub register: Option<RegisterMeta>,
    /// Modbus or fieldbus unit identifier.
    pub unit_id: Option<u8>,
    /// Application-level correlation identifier.
    pub correlation_id: Option<String>,
    /// Logical reply destination.
    pub reply_to: Option<Address>,
    /// Declared content type.
    pub content_type: Option<String>,
    /// Protocol hint carried through planning/encoding.
    pub protocol: Option<BridgeProtocolHint>,
}

/// Typed register metadata used by register-access planners and codecs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterMeta {
    /// Register or coil base address.
    pub address: u16,
    /// Register family.
    pub kind: RegisterKind,
    /// Optional quantity in registers or coils.
    pub quantity: Option<u16>,
}

/// Register family used by register-access bridge messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RegisterKind {
    /// Coil address space.
    Coil,
    /// Discrete input address space.
    DiscreteInput,
    /// Holding register address space.
    HoldingRegister,
    /// Input register address space.
    InputRegister,
}
