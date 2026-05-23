extern crate alloc;

use alloc::{string::ToString, vec::Vec};

use ferredge_bridge::{BridgeCodec, BridgeMessage, BridgeOp, BridgePayload, RegisterAccessAction};
use ferredge_core::prelude::*;
use rmodbus::{ErrorKind as RmodbusError, ModbusProto, client::ModbusRequest as RmodbusRequest};

use crate::{
    ModbusCommandConversionError, ModbusParserSeed, ModbusRequest, ModbusResponseDecoder,
    attributes::{ModbusRegisterKind, ModbusResourceAttributes, ModbusValueCodec},
    codec::encode_wire_frame,
    types::ModbusCommandRef,
    types::ModbusValue,
};

/// Bridge codec that turns a planned bridge message into a native Modbus request.
pub struct ModbusBridgeCodec<'a> {
    value: ModbusCommandRef<'a>,
    resource: &'a str,
    attributes: &'a ModbusResourceAttributes,
}

impl<'a> ModbusBridgeCodec<'a> {
    /// Creates a codec bound to one Modbus device/resource context.
    pub fn new(
        device: &'a Device<ModbusResourceAttributes>,
        resource: &'a str,
        attributes: &'a ModbusResourceAttributes,
    ) -> Self {
        Self {
            value: ModbusCommandRef { device },
            resource,
            attributes,
        }
    }
}

impl BridgeCodec<ModbusRequest> for ModbusBridgeCodec<'_> {
    type Error = ModbusCommandConversionError;

    fn encode(&self, message: &BridgeMessage) -> Result<ModbusRequest, Self::Error> {
        let proto = proto_from_endpoint(&self.value.device.endpoint).ok_or_else(|| {
            ModbusCommandConversionError::InvalidResource(
                "device endpoint is not Modbus".to_string(),
            )
        })?;
        let options = endpoint_options(&self.value.device.endpoint).ok_or_else(|| {
            ModbusCommandConversionError::InvalidResource(
                "missing Modbus endpoint options".to_string(),
            )
        })?;
        let BridgeMessage::Command(command) = message else {
            return Err(ModbusCommandConversionError::InvalidBridgeMessage);
        };
        let BridgeOp::RegisterAccess(operation) = &command.operation else {
            return Err(ModbusCommandConversionError::InvalidBridgeMessage);
        };

        match operation.action {
            RegisterAccessAction::Read => {
                build_read_request(self.resource, self.attributes, proto, options)
            }
            RegisterAccessAction::Write => {
                let payload = bridge_payload_to_payload_value(
                    command
                        .payload
                        .as_ref()
                        .ok_or(ModbusCommandConversionError::InvalidBridgeMessage)?,
                );
                build_write_request(self.resource, self.attributes, &payload, proto, options)
            }
        }
    }

    fn decode(&self, _native: ModbusRequest) -> Result<BridgeMessage, Self::Error> {
        Err(ModbusCommandConversionError::InvalidBridgeMessage)
    }
}

fn bridge_payload_to_payload_value(payload: &BridgePayload) -> PayloadValue {
    PayloadValue::from(payload.clone())
}

fn build_read_request(
    resource: &str,
    attributes: &ModbusResourceAttributes,
    proto: ModbusProto,
    options: &ModbusClientOptions,
) -> Result<ModbusRequest, ModbusCommandConversionError> {
    let quantity = quantity_for_read(attributes)?;
    let mut builder = RmodbusRequest::new(options.unit_id, proto);
    let mut binary_frame = Vec::new();
    let parser_seed = match attributes.register_kind {
        ModbusRegisterKind::Coil => {
            validate_bit_codec(attributes)?;
            builder
                .generate_get_coils(attributes.address, quantity, &mut binary_frame)
                .map_err(map_rmodbus_build_error)?;
            ModbusParserSeed::ReadCoils {
                address: attributes.address,
                quantity,
            }
        }
        ModbusRegisterKind::DiscreteInput => {
            validate_bit_codec(attributes)?;
            builder
                .generate_get_discretes(attributes.address, quantity, &mut binary_frame)
                .map_err(map_rmodbus_build_error)?;
            ModbusParserSeed::ReadDiscretes {
                address: attributes.address,
                quantity,
            }
        }
        ModbusRegisterKind::HoldingRegister => {
            validate_register_codec(attributes)?;
            builder
                .generate_get_holdings(attributes.address, quantity, &mut binary_frame)
                .map_err(map_rmodbus_build_error)?;
            ModbusParserSeed::ReadHoldings {
                address: attributes.address,
                quantity,
            }
        }
        ModbusRegisterKind::InputRegister => {
            validate_register_codec(attributes)?;
            builder
                .generate_get_inputs(attributes.address, quantity, &mut binary_frame)
                .map_err(map_rmodbus_build_error)?;
            ModbusParserSeed::ReadInputs {
                address: attributes.address,
                quantity,
            }
        }
    };
    Ok(ModbusRequest {
        resource: resource.to_string(),
        is_write: false,
        frame: encode_wire_frame(proto, &binary_frame).map_err(map_rmodbus_build_error)?,
        proto,
        unit_id: options.unit_id,
        parser_seed,
        decoder: decoder_for_codec(attributes, quantity),
        timeout: options.request_timeout,
    })
}

fn build_write_request(
    resource: &str,
    attributes: &ModbusResourceAttributes,
    payload: &PayloadValue,
    proto: ModbusProto,
    options: &ModbusClientOptions,
) -> Result<ModbusRequest, ModbusCommandConversionError> {
    let payload = modbus_value_from_payload(attributes, payload)?;
    let mut builder = RmodbusRequest::new(options.unit_id, proto);
    let mut binary_frame = Vec::new();
    let parser_seed = match attributes.register_kind {
        ModbusRegisterKind::Coil => build_coil_write_request(
            resource,
            attributes,
            &payload,
            &mut builder,
            &mut binary_frame,
        )?,
        ModbusRegisterKind::HoldingRegister => build_holding_write_request(
            resource,
            attributes,
            &payload,
            &mut builder,
            &mut binary_frame,
        )?,
        ModbusRegisterKind::DiscreteInput | ModbusRegisterKind::InputRegister => {
            return Err(ModbusCommandConversionError::UnsupportedWrite(
                resource.to_string(),
            ));
        }
    };

    Ok(ModbusRequest {
        resource: resource.to_string(),
        is_write: true,
        frame: encode_wire_frame(proto, &binary_frame).map_err(map_rmodbus_build_error)?,
        proto,
        unit_id: options.unit_id,
        parser_seed,
        decoder: ModbusResponseDecoder::Ack,
        timeout: options.request_timeout,
    })
}

fn build_coil_write_request(
    resource: &str,
    attributes: &ModbusResourceAttributes,
    payload: &ModbusValue,
    builder: &mut RmodbusRequest,
    binary_frame: &mut Vec<u8>,
) -> Result<ModbusParserSeed, ModbusCommandConversionError> {
    match attributes.codec {
        ModbusValueCodec::Bool => {
            let value = decode_bool(payload)?;
            builder
                .generate_set_coil(attributes.address, value, binary_frame)
                .map_err(map_rmodbus_build_error)?;
            Ok(ModbusParserSeed::WriteSingleCoil {
                address: attributes.address,
                value,
            })
        }
        ModbusValueCodec::Bits => {
            let values = decode_bits(payload)?;
            builder
                .generate_set_coils_bulk(attributes.address, &values, binary_frame)
                .map_err(map_rmodbus_build_error)?;
            Ok(ModbusParserSeed::WriteMultipleCoils {
                address: attributes.address,
                values,
            })
        }
        _ => Err(ModbusCommandConversionError::UnsupportedWrite(
            resource.to_string(),
        )),
    }
}

fn build_holding_write_request(
    resource: &str,
    attributes: &ModbusResourceAttributes,
    payload: &ModbusValue,
    builder: &mut RmodbusRequest,
    binary_frame: &mut Vec<u8>,
) -> Result<ModbusParserSeed, ModbusCommandConversionError> {
    match attributes.codec {
        ModbusValueCodec::U16 | ModbusValueCodec::I16 => {
            let value = decode_u16(payload)?;
            builder
                .generate_set_holding(attributes.address, value, binary_frame)
                .map_err(map_rmodbus_build_error)?;
            Ok(ModbusParserSeed::WriteSingleHolding {
                address: attributes.address,
                value,
            })
        }
        ModbusValueCodec::U32Be
        | ModbusValueCodec::U32Le
        | ModbusValueCodec::I32Be
        | ModbusValueCodec::I32Le
        | ModbusValueCodec::F32Be
        | ModbusValueCodec::F32Le => {
            let values = words_from_payload(attributes, payload)?;
            builder
                .generate_set_holdings_bulk(attributes.address, &values, binary_frame)
                .map_err(map_rmodbus_build_error)?;
            Ok(ModbusParserSeed::WriteMultipleHoldings {
                address: attributes.address,
                values,
            })
        }
        ModbusValueCodec::Bytes => {
            let values = decode_bytes(payload)?;
            builder
                .generate_set_holdings_bulk_from_slice(attributes.address, &values, binary_frame)
                .map_err(map_rmodbus_build_error)?;
            Ok(ModbusParserSeed::WriteMultipleHoldingsBytes {
                address: attributes.address,
                values,
            })
        }
        ModbusValueCodec::Utf8String => {
            let value = decode_string(payload)?;
            builder
                .generate_set_holdings_string(attributes.address, &value, binary_frame)
                .map_err(map_rmodbus_build_error)?;
            Ok(ModbusParserSeed::WriteString {
                address: attributes.address,
                value,
            })
        }
        ModbusValueCodec::Bool | ModbusValueCodec::Bits => Err(
            ModbusCommandConversionError::UnsupportedWrite(resource.to_string()),
        ),
    }
}

pub(crate) fn proto_from_endpoint(endpoint: &DeviceEndpoint) -> Option<ModbusProto> {
    match endpoint {
        DeviceEndpoint::ModbusTCP(_) | DeviceEndpoint::ModbusUDP(_) => Some(ModbusProto::TcpUdp),
        DeviceEndpoint::ModbusRTUOverTCP(_) | DeviceEndpoint::ModbusRTU(_) => {
            Some(ModbusProto::Rtu)
        }
        DeviceEndpoint::ModbusASCII(_) => Some(ModbusProto::Ascii),
        _ => None,
    }
}

pub(crate) fn endpoint_options(endpoint: &DeviceEndpoint) -> Option<&ModbusClientOptions> {
    match endpoint {
        DeviceEndpoint::ModbusTCP(config) => Some(&config.options),
        DeviceEndpoint::ModbusRTUOverTCP(config) => Some(&config.options),
        DeviceEndpoint::ModbusUDP(config) => Some(&config.options),
        DeviceEndpoint::ModbusRTU(config) => Some(&config.options),
        DeviceEndpoint::ModbusASCII(config) => Some(&config.options),
        _ => None,
    }
}

fn quantity_for_read(
    attributes: &ModbusResourceAttributes,
) -> Result<u16, ModbusCommandConversionError> {
    if let Some(quantity) = attributes.quantity {
        if quantity == 0 {
            return Err(ModbusCommandConversionError::InvalidResource(
                "quantity must be greater than zero".to_string(),
            ));
        }
        return Ok(quantity);
    }

    match attributes.codec {
        ModbusValueCodec::Bool
        | ModbusValueCodec::Bits
        | ModbusValueCodec::U16
        | ModbusValueCodec::I16 => Ok(1),
        ModbusValueCodec::U32Be
        | ModbusValueCodec::U32Le
        | ModbusValueCodec::I32Be
        | ModbusValueCodec::I32Le
        | ModbusValueCodec::F32Be
        | ModbusValueCodec::F32Le => Ok(2),
        ModbusValueCodec::Bytes | ModbusValueCodec::Utf8String => {
            Err(ModbusCommandConversionError::InvalidResource(
                "quantity is required for raw bytes and string codecs".to_string(),
            ))
        }
    }
}

fn decoder_for_codec(
    attributes: &ModbusResourceAttributes,
    quantity: u16,
) -> ModbusResponseDecoder {
    match attributes.codec {
        ModbusValueCodec::Bool => ModbusResponseDecoder::Bool,
        ModbusValueCodec::Bits => ModbusResponseDecoder::Bits { quantity },
        ModbusValueCodec::U16 => ModbusResponseDecoder::U16,
        ModbusValueCodec::I16 => ModbusResponseDecoder::I16,
        ModbusValueCodec::U32Be => ModbusResponseDecoder::U32Be,
        ModbusValueCodec::U32Le => ModbusResponseDecoder::U32Le,
        ModbusValueCodec::I32Be => ModbusResponseDecoder::I32Be,
        ModbusValueCodec::I32Le => ModbusResponseDecoder::I32Le,
        ModbusValueCodec::F32Be => ModbusResponseDecoder::F32Be,
        ModbusValueCodec::F32Le => ModbusResponseDecoder::F32Le,
        ModbusValueCodec::Bytes => ModbusResponseDecoder::Bytes,
        ModbusValueCodec::Utf8String => ModbusResponseDecoder::Utf8String,
    }
}

fn validate_bit_codec(
    attributes: &ModbusResourceAttributes,
) -> Result<(), ModbusCommandConversionError> {
    match attributes.codec {
        ModbusValueCodec::Bool | ModbusValueCodec::Bits => Ok(()),
        _ => Err(ModbusCommandConversionError::InvalidResource(
            "bit resources require Bool or Bits codec".to_string(),
        )),
    }
}

fn validate_register_codec(
    attributes: &ModbusResourceAttributes,
) -> Result<(), ModbusCommandConversionError> {
    match attributes.codec {
        ModbusValueCodec::Bool | ModbusValueCodec::Bits => {
            Err(ModbusCommandConversionError::InvalidResource(
                "register resources cannot use Bool or Bits codec".to_string(),
            ))
        }
        _ => Ok(()),
    }
}

fn map_rmodbus_build_error(error: RmodbusError) -> ModbusCommandConversionError {
    ModbusCommandConversionError::InvalidPayload(
        match error {
            RmodbusError::OOB => "out of bounds",
            RmodbusError::FrameBroken => "frame broken",
            RmodbusError::FrameCRCError => "frame crc error",
            RmodbusError::Utf8Error => "utf8 error",
            _ => "rmodbus error",
        }
        .to_string(),
    )
}

fn modbus_value_from_payload(
    attributes: &ModbusResourceAttributes,
    payload: &PayloadValue,
) -> Result<ModbusValue, ModbusCommandConversionError> {
    match attributes.codec {
        ModbusValueCodec::Bool => Ok(ModbusValue::Bool(decode_payload_bool(payload)?)),
        ModbusValueCodec::Bits => Ok(ModbusValue::Bits(decode_payload_bits(payload)?)),
        ModbusValueCodec::U16 => Ok(ModbusValue::U16(decode_payload_u16_list(payload)?)),
        ModbusValueCodec::I16 => Ok(ModbusValue::I16(decode_payload_i16_list(payload)?)),
        ModbusValueCodec::U32Be | ModbusValueCodec::U32Le => {
            Ok(ModbusValue::U32(decode_payload_u32_list(payload)?))
        }
        ModbusValueCodec::I32Be | ModbusValueCodec::I32Le => {
            Ok(ModbusValue::I32(decode_payload_i32_list(payload)?))
        }
        ModbusValueCodec::F32Be | ModbusValueCodec::F32Le => {
            Ok(ModbusValue::F32(decode_payload_f32_list(payload)?))
        }
        ModbusValueCodec::Bytes => Ok(ModbusValue::Bytes(decode_bytes_from_payload_value(
            payload,
        )?)),
        ModbusValueCodec::Utf8String => {
            Ok(ModbusValue::Utf8String(decode_payload_string(payload)?))
        }
    }
}

fn decode_bool(payload: &ModbusValue) -> Result<bool, ModbusCommandConversionError> {
    match payload {
        ModbusValue::Bool(value) => Ok(*value),
        _ => Err(ModbusCommandConversionError::InvalidPayload(
            "bool payload must be a boolean".to_string(),
        )),
    }
}

fn decode_bits(payload: &ModbusValue) -> Result<Vec<u8>, ModbusCommandConversionError> {
    match payload {
        ModbusValue::Bits(values) => Ok(values.iter().map(|value| u8::from(*value)).collect()),
        _ => Err(ModbusCommandConversionError::InvalidPayload(
            "bits payload must be a boolean list".to_string(),
        )),
    }
}

fn decode_u16(payload: &ModbusValue) -> Result<u16, ModbusCommandConversionError> {
    match payload {
        ModbusValue::U16(values) if values.len() == 1 => Ok(values[0]),
        ModbusValue::I16(values) if values.len() == 1 => u16::try_from(values[0]).map_err(|_| {
            ModbusCommandConversionError::InvalidPayload(
                "i16 payload must fit into unsigned u16".to_string(),
            )
        }),
        _ => Err(ModbusCommandConversionError::InvalidPayload(
            "u16 payload must be a single 16-bit value".to_string(),
        )),
    }
}

fn words_from_payload(
    attributes: &ModbusResourceAttributes,
    payload: &ModbusValue,
) -> Result<Vec<u16>, ModbusCommandConversionError> {
    match attributes.codec {
        ModbusValueCodec::U32Be => words_from_u32_be(payload),
        ModbusValueCodec::I32Be => words_from_i32_be(payload),
        ModbusValueCodec::F32Be => words_from_f32_be(payload),
        ModbusValueCodec::U32Le => words_from_u32_le(payload),
        ModbusValueCodec::I32Le => words_from_i32_le(payload),
        ModbusValueCodec::F32Le => words_from_f32_le(payload),
        _ => Err(ModbusCommandConversionError::InvalidPayload(
            "word conversion requires 32-bit codec".to_string(),
        )),
    }
}

fn words_from_u32_be(payload: &ModbusValue) -> Result<Vec<u16>, ModbusCommandConversionError> {
    match payload {
        ModbusValue::U32(values) => Ok(values
            .iter()
            .flat_map(|value: &u32| {
                let bytes = value.to_be_bytes();
                [
                    u16::from_be_bytes([bytes[0], bytes[1]]),
                    u16::from_be_bytes([bytes[2], bytes[3]]),
                ]
            })
            .collect()),
        _ => Err(ModbusCommandConversionError::InvalidPayload(
            "u32 payload must be numeric".to_string(),
        )),
    }
}

fn words_from_i32_be(payload: &ModbusValue) -> Result<Vec<u16>, ModbusCommandConversionError> {
    match payload {
        ModbusValue::I32(values) => Ok(values
            .iter()
            .flat_map(|value: &i32| {
                let bytes = value.to_be_bytes();
                [
                    u16::from_be_bytes([bytes[0], bytes[1]]),
                    u16::from_be_bytes([bytes[2], bytes[3]]),
                ]
            })
            .collect()),
        _ => Err(ModbusCommandConversionError::InvalidPayload(
            "i32 payload must be numeric".to_string(),
        )),
    }
}

fn words_from_f32_be(payload: &ModbusValue) -> Result<Vec<u16>, ModbusCommandConversionError> {
    match payload {
        ModbusValue::F32(values) => Ok(values
            .iter()
            .flat_map(|value: &f32| {
                let bytes = value.to_bits().to_be_bytes();
                [
                    u16::from_be_bytes([bytes[0], bytes[1]]),
                    u16::from_be_bytes([bytes[2], bytes[3]]),
                ]
            })
            .collect()),
        _ => Err(ModbusCommandConversionError::InvalidPayload(
            "f32 payload must be numeric".to_string(),
        )),
    }
}

fn words_from_u32_le(payload: &ModbusValue) -> Result<Vec<u16>, ModbusCommandConversionError> {
    match payload {
        ModbusValue::U32(values) => Ok(values
            .iter()
            .flat_map(|value: &u32| {
                let bytes = value.to_le_bytes();
                [
                    u16::from_be_bytes([bytes[2], bytes[3]]),
                    u16::from_be_bytes([bytes[0], bytes[1]]),
                ]
            })
            .collect()),
        _ => Err(ModbusCommandConversionError::InvalidPayload(
            "u32 payload must be numeric".to_string(),
        )),
    }
}

fn words_from_i32_le(payload: &ModbusValue) -> Result<Vec<u16>, ModbusCommandConversionError> {
    match payload {
        ModbusValue::I32(values) => Ok(values
            .iter()
            .flat_map(|value: &i32| {
                let bytes = value.to_le_bytes();
                [
                    u16::from_be_bytes([bytes[2], bytes[3]]),
                    u16::from_be_bytes([bytes[0], bytes[1]]),
                ]
            })
            .collect()),
        _ => Err(ModbusCommandConversionError::InvalidPayload(
            "i32 payload must be numeric".to_string(),
        )),
    }
}

fn words_from_f32_le(payload: &ModbusValue) -> Result<Vec<u16>, ModbusCommandConversionError> {
    match payload {
        ModbusValue::F32(values) => Ok(values
            .iter()
            .flat_map(|value: &f32| {
                let bytes = value.to_bits().to_le_bytes();
                [
                    u16::from_be_bytes([bytes[2], bytes[3]]),
                    u16::from_be_bytes([bytes[0], bytes[1]]),
                ]
            })
            .collect()),
        _ => Err(ModbusCommandConversionError::InvalidPayload(
            "f32 payload must be numeric".to_string(),
        )),
    }
}

fn decode_bytes(payload: &ModbusValue) -> Result<Vec<u8>, ModbusCommandConversionError> {
    match payload {
        ModbusValue::Bytes(bytes) => Ok(bytes.clone()),
        _ => Err(ModbusCommandConversionError::InvalidPayload(
            "bytes payload must be binary data".to_string(),
        )),
    }
}

fn decode_string(payload: &ModbusValue) -> Result<String, ModbusCommandConversionError> {
    match payload {
        ModbusValue::Utf8String(value) => Ok(value.clone()),
        _ => Err(ModbusCommandConversionError::InvalidPayload(
            "string payload must be utf8 text".to_string(),
        )),
    }
}

fn decode_payload_bool(payload: &PayloadValue) -> Result<bool, ModbusCommandConversionError> {
    match payload {
        PayloadValue::Bool(value) => Ok(*value),
        _ => Err(ModbusCommandConversionError::InvalidPayload(
            "bool payload must be a boolean".to_string(),
        )),
    }
}

fn decode_payload_bits(payload: &PayloadValue) -> Result<Vec<bool>, ModbusCommandConversionError> {
    let values = payload_list(payload)?;
    values
        .iter()
        .map(|value| match value {
            PayloadValue::Bool(flag) => Ok(*flag),
            _ => Err(ModbusCommandConversionError::InvalidPayload(
                "bits payload entries must be booleans".to_string(),
            )),
        })
        .collect()
}

fn decode_payload_u16_list(
    payload: &PayloadValue,
) -> Result<Vec<u16>, ModbusCommandConversionError> {
    decode_numeric_list(payload, "u16 payload must be unsigned integer", |value| {
        u16::try_from(value).map_err(|_| {
            ModbusCommandConversionError::InvalidPayload("u16 payload out of range".to_string())
        })
    })
}

fn decode_payload_i16_list(
    payload: &PayloadValue,
) -> Result<Vec<i16>, ModbusCommandConversionError> {
    decode_numeric_list(payload, "i16 payload must be integer", |value| {
        i16::try_from(value).map_err(|_| {
            ModbusCommandConversionError::InvalidPayload("i16 payload out of range".to_string())
        })
    })
}

fn decode_payload_u32_list(
    payload: &PayloadValue,
) -> Result<Vec<u32>, ModbusCommandConversionError> {
    decode_numeric_list(payload, "u32 payload must be unsigned integer", |value| {
        u32::try_from(value).map_err(|_| {
            ModbusCommandConversionError::InvalidPayload("u32 payload out of range".to_string())
        })
    })
}

fn decode_payload_i32_list(
    payload: &PayloadValue,
) -> Result<Vec<i32>, ModbusCommandConversionError> {
    decode_numeric_list(payload, "i32 payload must be integer", |value| {
        i32::try_from(value).map_err(|_| {
            ModbusCommandConversionError::InvalidPayload("i32 payload out of range".to_string())
        })
    })
}

fn decode_payload_f32_list(
    payload: &PayloadValue,
) -> Result<Vec<f32>, ModbusCommandConversionError> {
    match payload {
        PayloadValue::F64(value) => Ok(vec![*value as f32]),
        PayloadValue::I64(value) => Ok(vec![*value as f32]),
        PayloadValue::U64(value) => Ok(vec![*value as f32]),
        PayloadValue::List(values) => values
            .iter()
            .map(|value| match value {
                PayloadValue::F64(value) => Ok(*value as f32),
                PayloadValue::I64(value) => Ok(*value as f32),
                PayloadValue::U64(value) => Ok(*value as f32),
                _ => Err(ModbusCommandConversionError::InvalidPayload(
                    "f32 payload entries must be numeric".to_string(),
                )),
            })
            .collect(),
        _ => Err(ModbusCommandConversionError::InvalidPayload(
            "f32 payload must be numeric".to_string(),
        )),
    }
}

fn decode_bytes_from_payload_value(
    payload: &PayloadValue,
) -> Result<Vec<u8>, ModbusCommandConversionError> {
    match payload {
        PayloadValue::Bytes(bytes) => Ok(bytes.clone()),
        _ => Err(ModbusCommandConversionError::InvalidPayload(
            "bytes payload must be binary data".to_string(),
        )),
    }
}

fn decode_payload_string(payload: &PayloadValue) -> Result<String, ModbusCommandConversionError> {
    match payload {
        PayloadValue::String(value) => Ok(value.clone()),
        _ => Err(ModbusCommandConversionError::InvalidPayload(
            "string payload must be utf8 text".to_string(),
        )),
    }
}

fn payload_list(payload: &PayloadValue) -> Result<&[PayloadValue], ModbusCommandConversionError> {
    match payload {
        PayloadValue::List(values) => Ok(values),
        _ => Err(ModbusCommandConversionError::InvalidPayload(
            "payload must be a list".to_string(),
        )),
    }
}

fn decode_numeric_list<T, F>(
    payload: &PayloadValue,
    scalar_error: &'static str,
    map: F,
) -> Result<Vec<T>, ModbusCommandConversionError>
where
    F: Fn(i128) -> Result<T, ModbusCommandConversionError>,
{
    match payload {
        PayloadValue::I64(value) => map(i128::from(*value)).map(|value| vec![value]),
        PayloadValue::U64(value) => map(i128::from(*value)).map(|value| vec![value]),
        PayloadValue::List(values) => values
            .iter()
            .map(|value| match value {
                PayloadValue::I64(value) => map(i128::from(*value)),
                PayloadValue::U64(value) => map(i128::from(*value)),
                _ => Err(ModbusCommandConversionError::InvalidPayload(
                    scalar_error.to_string(),
                )),
            })
            .collect(),
        _ => Err(ModbusCommandConversionError::InvalidPayload(
            scalar_error.to_string(),
        )),
    }
}
