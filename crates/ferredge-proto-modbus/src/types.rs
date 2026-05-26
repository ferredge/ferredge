extern crate alloc;

use alloc::{borrow::Cow, string::String, vec::Vec};
use core::time::Duration;

use ferredge_bridge::{
    BridgeMessage, BridgeOp, BridgeOutbound, BridgePlannerError, NativeOutbound, ProtocolDecoder,
    ProtocolPlanner, RegisterAccessAction, planner,
};
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
    pub proto: ModbusProto,
    pub unit_id: u8,
    pub parser_seed: ModbusParserSeed,
    pub decoder: ModbusResponseDecoder,
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
pub(crate) enum ModbusValue<'a> {
    Ack,
    Bool(bool),
    Bits(Vec<bool>),
    U16(Vec<u16>),
    I16(Vec<i16>),
    U32(Vec<u32>),
    I32(Vec<i32>),
    F32(Vec<f32>),
    Bytes(Cow<'a, [u8]>),
    Utf8String(Cow<'a, str>),
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ModbusCommandRef<'a> {
    pub device: &'a Device<ModbusResourceAttributes>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModbusNativePlan<'a> {
    pub resource: Cow<'a, str>,
    pub action: RegisterAccessAction,
    pub payload: Option<PayloadValue<'a>>,
    pub register: ferredge_bridge::AddressedAccessMeta,
    pub unit_id: u8,
}

impl ModbusNativePlan<'_> {
    pub fn into_owned(self) -> ModbusNativePlan<'static> {
        ModbusNativePlan {
            resource: Cow::Owned(self.resource.into_owned()),
            action: self.action,
            payload: self.payload.map(PayloadValue::into_owned),
            register: self.register,
            unit_id: self.unit_id,
        }
    }
}

pub struct ModbusCommandPlanner<'a> {
    device: &'a Device<ModbusResourceAttributes>,
}

/// Inbound Modbus response decoder bound to one originating command.
pub struct ModbusResponseDecoderContext<'a> {
    device: &'a Device<ModbusResourceAttributes>,
    command: &'a Command,
}

/// Decoded inbound Modbus response plus bound command context.
pub struct ModbusDecodedResponse<'a, 'ctx> {
    device: &'ctx Device<ModbusResourceAttributes>,
    command: &'ctx Command,
    response: &'a ModbusResponse,
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

    pub async fn execute_command(&self, command: Command) -> Result<ModbusResponse, String> {
        let request = self.native_request(command).map_err(|e| e.to_string())?;
        self.execute(request).await
    }

    pub fn native_request(
        &self,
        command: Command,
    ) -> Result<ModbusRequest, ModbusCommandConversionError> {
        let planner = ModbusCommandPlanner::new(&self.dvc);
        let codec = crate::ModbusBridgeCodec::new(&self.dvc);
        let plan = planner.plan_ref(&command)?;
        codec.encode_ref(plan)
    }

    pub fn bridge_request(
        &self,
        command: Command,
    ) -> Result<ModbusRequest, ModbusCommandConversionError> {
        let register = match &command.intent {
            Intent::Read { resource, .. } | Intent::Write { resource, .. } => {
                let resource_def = self.dvc.resources.get(resource).ok_or_else(|| {
                    ModbusCommandConversionError::UnknownResource(resource.clone())
                })?;
                register_meta(&resource_def.resource_attributes)
            }
            _ => return Err(ModbusCommandConversionError::UnsupportedIntent),
        };

        let message = planner::command_to_register_access(
            command,
            register,
            endpoint_options(&self.dvc.endpoint)
                .ok_or_else(|| {
                    ModbusCommandConversionError::InvalidResource(
                        "missing Modbus endpoint options".to_string(),
                    )
                })?
                .unit_id,
        )?;
        crate::ModbusBridgeCodec::new(&self.dvc).encode_ref(ModbusNativePlan::try_from(&message)?)
    }
}

impl<'a> ModbusCommandPlanner<'a> {
    /// Creates an outbound planner bound to one Modbus device definition.
    pub fn new(device: &'a Device<ModbusResourceAttributes>) -> Self {
        Self { device }
    }

    pub fn plan_ref<'b>(
        &self,
        command: &'b Command,
    ) -> Result<ModbusNativePlan<'b>, ModbusCommandConversionError> {
        let (resource, action, payload) = match &command.intent {
            Intent::Read { resource, .. } => (resource.as_str(), RegisterAccessAction::Read, None),
            Intent::Write {
                resource, payload, ..
            } => (
                resource.as_str(),
                RegisterAccessAction::Write,
                Some(payload.as_borrowed()),
            ),
            _ => return Err(ModbusCommandConversionError::UnsupportedIntent),
        };
        let resource_def =
            self.device.resources.get(resource).ok_or_else(|| {
                ModbusCommandConversionError::UnknownResource(resource.to_string())
            })?;
        let unit_id = endpoint_options(&self.device.endpoint)
            .ok_or_else(|| {
                ModbusCommandConversionError::InvalidResource(
                    "missing Modbus endpoint options".to_string(),
                )
            })?
            .unit_id;

        Ok(ModbusNativePlan {
            resource: Cow::Borrowed(resource),
            action,
            payload,
            register: register_meta(&resource_def.resource_attributes),
            unit_id,
        })
    }
}

impl ProtocolPlanner<BridgeOutbound, BridgeMessage<'static>> for ModbusCommandPlanner<'_> {
    type Error = ModbusCommandConversionError;

    fn plan(&self, command: Command) -> Result<BridgeMessage<'static>, Self::Error> {
        let register = match &command.intent {
            Intent::Read { resource, .. } | Intent::Write { resource, .. } => {
                let resource_def = self.device.resources.get(resource).ok_or_else(|| {
                    ModbusCommandConversionError::UnknownResource(resource.clone())
                })?;
                register_meta(&resource_def.resource_attributes)
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

        planner::command_to_register_access(command, register, unit_id).map_err(Into::into)
    }
}

impl ProtocolPlanner<NativeOutbound, ModbusNativePlan<'static>> for ModbusCommandPlanner<'_> {
    type Error = ModbusCommandConversionError;

    fn plan(&self, command: Command) -> Result<ModbusNativePlan<'static>, Self::Error> {
        self.plan_ref(&command).map(ModbusNativePlan::into_owned)
    }
}

impl<'a> ModbusResponseDecoderContext<'a> {
    pub fn new(device: &'a Device<ModbusResourceAttributes>, command: &'a Command) -> Self {
        Self { device, command }
    }

    fn decode_response<'b>(&self, response: &'b ModbusResponse) -> ModbusDecodedResponse<'b, 'a> {
        ModbusDecodedResponse {
            device: self.device,
            command: self.command,
            response,
        }
    }
}

impl<'ctx> ProtocolDecoder<ModbusResponse> for ModbusResponseDecoderContext<'ctx> {
    type Error = ModbusCommandConversionError;
    type Decoded<'a>
        = ModbusDecodedResponse<'a, 'ctx>
    where
        ModbusResponse: 'a;

    fn decode<'a>(&self, native: &'a ModbusResponse) -> Result<Self::Decoded<'a>, Self::Error> {
        Ok(self.decode_response(native))
    }
}

impl<'a, 'ctx> TryFrom<ModbusDecodedResponse<'a, 'ctx>> for RoutedMessage<'a>
where
    'ctx: 'a,
{
    type Error = ModbusCommandConversionError;

    fn try_from(value: ModbusDecodedResponse<'a, 'ctx>) -> Result<Self, Self::Error> {
        let self_ = value;
        let resource = match &self_.command.intent {
            Intent::Read { resource, .. } | Intent::Write { resource, .. } => resource.as_str(),
            _ => return Err(ModbusCommandConversionError::UnsupportedIntent),
        };

        Ok(RoutedMessage::Result(RoutedResult {
            source: EndpointRef {
                device_id: self_.device.id.clone(),
                protocol: DeviceProtocol::Modbus,
            },
            result: CommandResult {
                command_id: self_.command.id.clone(),
                device_id: self_.device.id.clone(),
                state: DeliveryState::Completed,
                payload: Some(self_.response.payload()?),
                error: None,
                correlation: self_
                    .command
                    .correlation
                    .as_ref()
                    .map(Correlation::as_borrowed),
            },
            transport: None,
        }))
        .map(|message| match message {
            RoutedMessage::Result(mut result) => {
                result.result.correlation = result.result.correlation.or_else(|| {
                    Some(Correlation {
                        request_id: Cow::Borrowed(self_.command.id.as_str()),
                        reply_to: Some(Address::Resource(Cow::Borrowed(resource))),
                    })
                });
                RoutedMessage::Result(result)
            }
            other => other,
        })
    }
}

impl<'a, 'ctx> TryFrom<ModbusDecodedResponse<'a, 'ctx>> for BridgeMessage<'a>
where
    'ctx: 'a,
{
    type Error = ModbusCommandConversionError;

    fn try_from(value: ModbusDecodedResponse<'a, 'ctx>) -> Result<Self, Self::Error> {
        let routed = RoutedMessage::try_from(value)?;
        let RoutedMessage::Result(result) = routed else {
            unreachable!("modbus decoded response always projects to result")
        };
        Ok(planner::routed_result_to_bridge(result))
    }
}

impl<'a> TryFrom<&'a BridgeMessage<'a>> for ModbusNativePlan<'a> {
    type Error = ModbusCommandConversionError;

    fn try_from(message: &'a BridgeMessage<'a>) -> Result<Self, Self::Error> {
        let BridgeMessage::Command(command) = message else {
            return Err(ModbusCommandConversionError::InvalidBridgeMessage);
        };
        let BridgeOp::RegisterAccess(operation) = &command.operation else {
            return Err(ModbusCommandConversionError::InvalidBridgeMessage);
        };
        let ferredge_bridge::BridgeRoute::AddressedAccess {
            resource,
            access,
            node_id,
        } = &command.route
        else {
            return Err(ModbusCommandConversionError::InvalidBridgeMessage);
        };

        Ok(ModbusNativePlan {
            resource: resource.clone(),
            action: operation.action.clone(),
            payload: command
                .payload
                .as_ref()
                .map(payload_value_from_bridge_payload),
            register: access.clone(),
            unit_id: node_id.unwrap_or(1) as u8,
        })
    }
}

impl ModbusResponse {
    pub fn payload(&self) -> Result<PayloadValue<'_>, ModbusCommandConversionError> {
        crate::codec::decode_modbus_value(
            self.proto,
            self.unit_id,
            &self.parser_seed,
            &self.decoder,
            &self.frame,
        )
        .map(crate::codec::payload_value_from_modbus_value)
    }

    pub fn into_payload(self) -> Result<PayloadValue<'static>, ModbusCommandConversionError> {
        self.payload().map(PayloadValue::into_owned)
    }
}

fn payload_value_from_bridge_payload<'a>(
    payload: &'a ferredge_bridge::BridgePayload<'a>,
) -> PayloadValue<'a> {
    match payload {
        ferredge_bridge::BridgePayload::Empty => PayloadValue::Null,
        ferredge_bridge::BridgePayload::Scalar(ferredge_bridge::BridgeScalar::Bool(value)) => {
            PayloadValue::Bool(*value)
        }
        ferredge_bridge::BridgePayload::Scalar(ferredge_bridge::BridgeScalar::I64(value)) => {
            PayloadValue::I64(*value)
        }
        ferredge_bridge::BridgePayload::Scalar(ferredge_bridge::BridgeScalar::U64(value)) => {
            PayloadValue::U64(*value)
        }
        ferredge_bridge::BridgePayload::Scalar(ferredge_bridge::BridgeScalar::F64(value)) => {
            PayloadValue::F64(*value)
        }
        ferredge_bridge::BridgePayload::Text(value) => {
            PayloadValue::String(Cow::Borrowed(value.as_ref()))
        }
        ferredge_bridge::BridgePayload::Binary(value) => {
            PayloadValue::Bytes(Cow::Borrowed(value.as_ref()))
        }
        ferredge_bridge::BridgePayload::Sequence(values) => PayloadValue::List(Cow::Owned(
            values
                .iter()
                .map(payload_value_from_bridge_payload)
                .collect(),
        )),
        ferredge_bridge::BridgePayload::Object(values) => PayloadValue::Map(Cow::Owned(
            values
                .iter()
                .map(|(key, value)| {
                    (
                        Cow::Borrowed(key.as_ref()),
                        payload_value_from_bridge_payload(value),
                    )
                })
                .collect(),
        )),
    }
}

fn register_meta(attributes: &ModbusResourceAttributes) -> ferredge_bridge::AddressedAccessMeta {
    ferredge_bridge::AddressedAccessMeta {
        address: u32::from(attributes.address),
        domain: match attributes.register_kind {
            crate::attributes::ModbusRegisterKind::Coil => "coil".into(),
            crate::attributes::ModbusRegisterKind::DiscreteInput => "discrete-input".into(),
            crate::attributes::ModbusRegisterKind::HoldingRegister => "holding-register".into(),
            crate::attributes::ModbusRegisterKind::InputRegister => "input-register".into(),
        },
        quantity: attributes.quantity,
    }
}
