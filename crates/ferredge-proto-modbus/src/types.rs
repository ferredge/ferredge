extern crate alloc;

use alloc::{string::String, vec::Vec};
use core::time::Duration;

use ferredge_core::prelude::*;
use rmodbus::ModbusProto;

use crate::{
    StackNet, StackRuntime, StackSerial,
    attributes::ModbusResourceAttributes,
};

/// Native Modbus request used by the driver execute path.
#[derive(Debug, Clone, PartialEq)]
pub struct ModbusRequest {
    pub resource: String,
    pub is_write: bool,
    pub frame: Vec<u8>,
    pub proto: ModbusProto,
    pub unit_id: u8,
    pub parser_seed: ModbusParserSeed,
    pub decoder: ModbusResponseDecoder,
    pub timeout: Option<Duration>,
}

/// Native Modbus response returned by the driver execute path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModbusResponse {
    pub frame: Vec<u8>,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModbusCommandConversionError {
    UnsupportedIntent,
    UnknownResource(String),
    InvalidResource(String),
    InvalidPayload(&'static str),
    UnsupportedWrite(String),
}

impl core::fmt::Display for ModbusCommandConversionError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnsupportedIntent => write!(f, "unsupported intent for Modbus driver"),
            Self::UnknownResource(resource) => {
                write!(f, "resource {resource} not found for Modbus driver")
            }
            Self::InvalidResource(resource) => write!(f, "invalid Modbus resource: {resource}"),
            Self::InvalidPayload(reason) => write!(f, "invalid Modbus payload: {reason}"),
            Self::UnsupportedWrite(resource) => {
                write!(f, "resource {resource} is not writable via Modbus")
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModbusParserSeed {
    ReadCoils { address: u16, quantity: u16 },
    ReadDiscretes { address: u16, quantity: u16 },
    ReadHoldings { address: u16, quantity: u16 },
    ReadInputs { address: u16, quantity: u16 },
    WriteSingleCoil { address: u16, value: bool },
    WriteSingleHolding { address: u16, value: u16 },
    WriteMultipleCoils { address: u16, values: Vec<u8> },
    WriteMultipleHoldings { address: u16, values: Vec<u16> },
    WriteMultipleHoldingsBytes { address: u16, values: Vec<u8> },
    WriteString { address: u16, value: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModbusResponseDecoder {
    Ack,
    Bool,
    Bits { quantity: u16 },
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

#[derive(Debug, Clone, Copy)]
pub struct ModbusCommandRef<'a> {
    pub device: &'a Device<ModbusResourceAttributes>,
    pub command: &'a Command,
}

#[derive(Clone)]
pub struct ModbusDriver {
    pub dvc: Device<ModbusResourceAttributes>,
    pub(crate) runtime: StackRuntime,
    pub(crate) net: StackNet,
    pub(crate) serial: StackSerial,
}

impl core::fmt::Debug for ModbusDriver {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ModbusDriver")
            .field("dvc", &self.dvc)
            .finish()
    }
}

impl ModbusDriver {
    pub fn new(dvc: Device<ModbusResourceAttributes>) -> Self {
        Self {
            dvc,
            runtime: StackRuntime::default(),
            net: StackNet::default(),
            serial: StackSerial::default(),
        }
    }

    pub async fn execute_command(&self, command: &Command) -> Result<ModbusResponse, String> {
        let request = ModbusRequest::try_from(ModbusCommandRef {
            device: &self.dvc,
            command,
        })
        .map_err(|e| e.to_string())?;
        self.execute(request).await
    }
}
