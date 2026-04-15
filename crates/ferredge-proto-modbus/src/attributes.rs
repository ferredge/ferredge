extern crate alloc;

use alloc::string::String;

use ferredge_core::prelude::DeviceResourceAttributes;
use serde::{Deserialize, Serialize};

/// Modbus address space used by one resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModbusRegisterKind {
    Coil,
    DiscreteInput,
    HoldingRegister,
    InputRegister,
}

/// Payload codec used to map between Modbus data and routed command bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModbusValueCodec {
    Bool,
    Bits,
    U16,
    I16,
    U32Be,
    U32Le,
    I32Be,
    I32Le,
    F32Be,
    F32Le,
    Bytes,
    Utf8String,
}

/// Modbus-specific resource metadata used to build requests and decode replies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModbusResourceAttributes {
    /// Register or coil address for this resource.
    pub address: u16,
    /// Address space used by this resource.
    pub register_kind: ModbusRegisterKind,
    /// Payload codec used for read/write conversion.
    pub codec: ModbusValueCodec,
    /// Optional protocol quantity override in coils/registers.
    pub quantity: Option<u16>,
    /// Optional override for response string decoding or error messages.
    pub description: Option<String>,
}

impl DeviceResourceAttributes for ModbusResourceAttributes {}
