extern crate alloc;

use alloc::{string::String, vec::Vec};
use core::time::Duration;

use ferredge_bridge::{BridgeAdapter, BridgeCodec, BridgeMessage, BridgePlannerError, planner};
use ferredge_core::prelude::*;
use rmodbus::ModbusProto;

use crate::{
    StackNet, StackRuntime, StackSerial, StackSerialPort, StackSocket,
    attributes::ModbusResourceAttributes, convert::endpoint_options,
};

type RuntimeMutex<T> = <StackRuntime as AsyncRuntime>::Mutex<T>;

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
#[derive(Debug, Clone, PartialEq)]
pub struct ModbusResponse {
    pub frame: Vec<u8>,
    pub payload: PayloadValue,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ModbusCommandConversionError {
    #[error("unsupported intent for Modbus driver")]
    UnsupportedIntent,
    #[error("resource {0} not found for Modbus driver")]
    UnknownResource(String),
    #[error("invalid Modbus resource: {0}")]
    InvalidResource(String),
    #[error("invalid Modbus payload: {0}")]
    InvalidPayload(String),
    #[error("resource {0} is not writable via Modbus")]
    UnsupportedWrite(String),
    #[error("invalid bridge request: {0}")]
    Bridge(#[from] BridgePlannerError),
    #[error("bridge message does not describe a Modbus register request")]
    InvalidBridgeMessage,
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

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ModbusValue {
    Ack,
    Bool(bool),
    Bits(Vec<bool>),
    U16(Vec<u16>),
    I16(Vec<i16>),
    U32(Vec<u32>),
    I32(Vec<i32>),
    F32(Vec<f32>),
    Bytes(Vec<u8>),
    Utf8String(String),
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ModbusCommandRef<'a> {
    pub device: &'a Device<ModbusResourceAttributes>,
}

/// Bridge adapter for Modbus register-access semantics.
pub struct ModbusBridgeAdapter<'a> {
    device: &'a Device<ModbusResourceAttributes>,
}

pub(crate) enum PersistentSession {
    Tcp(StackSocket),
    Rtu(StackSerialPort),
    Ascii(StackSerialPort),
}

#[derive(Clone)]
pub struct ModbusDriver {
    pub dvc: Device<ModbusResourceAttributes>,
    pub(crate) runtime: StackRuntime,
    pub(crate) net: StackNet,
    pub(crate) serial: StackSerial,
    pub(crate) persistent_session: Shared<RuntimeMutex<Option<PersistentSession>>>,
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
        let runtime = StackRuntime::default();
        Self {
            dvc,
            runtime: runtime.clone(),
            net: StackNet::default(),
            serial: StackSerial::default(),
            persistent_session: Shared::new(runtime.mutex(None)),
        }
    }

    pub async fn execute_command(&self, command: &Command) -> Result<ModbusResponse, String> {
        let request = self.bridge_request(command).map_err(|e| e.to_string())?;
        self.execute(request).await
    }

    pub fn bridge_request(
        &self,
        command: &Command,
    ) -> Result<ModbusRequest, ModbusCommandConversionError> {
        let (resource, attributes) = match &command.intent {
            Intent::Read { resource } | Intent::Write { resource, .. } => {
                let resource_def = self.dvc.resources.get(resource).ok_or_else(|| {
                    ModbusCommandConversionError::UnknownResource(resource.clone())
                })?;
                (resource.clone(), resource_def.resource_attributes.clone())
            }
            _ => return Err(ModbusCommandConversionError::UnsupportedIntent),
        };

        let message = planner::command_to_register_access(
            command,
            ferredge_bridge::RegisterMeta {
                address: attributes.address,
                kind: match attributes.register_kind {
                    crate::attributes::ModbusRegisterKind::Coil => {
                        ferredge_bridge::RegisterKind::Coil
                    }
                    crate::attributes::ModbusRegisterKind::DiscreteInput => {
                        ferredge_bridge::RegisterKind::DiscreteInput
                    }
                    crate::attributes::ModbusRegisterKind::HoldingRegister => {
                        ferredge_bridge::RegisterKind::HoldingRegister
                    }
                    crate::attributes::ModbusRegisterKind::InputRegister => {
                        ferredge_bridge::RegisterKind::InputRegister
                    }
                },
                quantity: attributes.quantity,
            },
            endpoint_options(&self.dvc.endpoint)
                .ok_or_else(|| {
                    ModbusCommandConversionError::InvalidResource(
                        "missing Modbus endpoint options".to_string(),
                    )
                })?
                .unit_id,
        )?;

        crate::ModbusBridgeCodec::new(&self.dvc, &resource, &attributes).encode(&message)
    }
}

impl<'a> ModbusBridgeAdapter<'a> {
    /// Creates a bridge adapter bound to one Modbus device definition.
    pub fn new(device: &'a Device<ModbusResourceAttributes>) -> Self {
        Self { device }
    }
}

impl BridgeAdapter for ModbusBridgeAdapter<'_> {
    type Error = ModbusCommandConversionError;

    fn command_to_bridge(&self, command: &Command) -> Result<BridgeMessage, Self::Error> {
        let (_resource, attributes) = match &command.intent {
            Intent::Read { resource } | Intent::Write { resource, .. } => {
                let resource_def = self.device.resources.get(resource).ok_or_else(|| {
                    ModbusCommandConversionError::UnknownResource(resource.clone())
                })?;
                (resource.clone(), resource_def.resource_attributes.clone())
            }
            _ => return Err(ModbusCommandConversionError::UnsupportedIntent),
        };

        let unit_id = endpoint_options(&self.device.endpoint)
            .ok_or_else(|| {
                ModbusCommandConversionError::InvalidResource(
                    "missing Modbus endpoint options".to_string(),
                )
            })?
            .unit_id;

        planner::command_to_register_access(
            command,
            ferredge_bridge::RegisterMeta {
                address: attributes.address,
                kind: match attributes.register_kind {
                    crate::attributes::ModbusRegisterKind::Coil => {
                        ferredge_bridge::RegisterKind::Coil
                    }
                    crate::attributes::ModbusRegisterKind::DiscreteInput => {
                        ferredge_bridge::RegisterKind::DiscreteInput
                    }
                    crate::attributes::ModbusRegisterKind::HoldingRegister => {
                        ferredge_bridge::RegisterKind::HoldingRegister
                    }
                    crate::attributes::ModbusRegisterKind::InputRegister => {
                        ferredge_bridge::RegisterKind::InputRegister
                    }
                },
                quantity: attributes.quantity,
            },
            unit_id,
        )
        .map_err(Into::into)
    }

    fn event_to_bridge(&self, event: &RoutedEvent) -> Result<BridgeMessage, Self::Error> {
        Ok(planner::routed_event_to_bridge(event))
    }

    fn result_to_bridge(&self, result: &RoutedResult) -> Result<BridgeMessage, Self::Error> {
        Ok(planner::routed_result_to_bridge(result))
    }
}
