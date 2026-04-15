extern crate alloc;

use alloc::{string::ToString, vec::Vec};

use ferredge_core::prelude::*;
use rmodbus::{ErrorKind as RmodbusError, ModbusProto, client::ModbusRequest as RmodbusRequest};

use crate::{
    attributes::{ModbusRegisterKind, ModbusResourceAttributes, ModbusValueCodec},
    codec::encode_wire_frame,
    ModbusCommandConversionError, ModbusCommandRef, ModbusParserSeed, ModbusRequest,
    ModbusResponseDecoder,
};

impl TryFrom<ModbusCommandRef<'_>> for ModbusRequest {
    type Error = ModbusCommandConversionError;

    fn try_from(value: ModbusCommandRef<'_>) -> Result<Self, Self::Error> {
        let proto = proto_from_endpoint(&value.device.endpoint).ok_or_else(|| {
            ModbusCommandConversionError::InvalidResource("device endpoint is not Modbus".to_string())
        })?;
        let options = endpoint_options(&value.device.endpoint).ok_or_else(|| {
            ModbusCommandConversionError::InvalidResource("missing Modbus endpoint options".to_string())
        })?;

        match &value.command.intent {
            Intent::Read { resource } => {
                let resource_def = value
                    .device
                    .resources
                    .get(resource)
                    .ok_or_else(|| ModbusCommandConversionError::UnknownResource(resource.clone()))?;
                build_read_request(resource, &resource_def.resource_attributes, proto, options)
            }
            Intent::Write { resource, payload } => {
                let resource_def = value
                    .device
                    .resources
                    .get(resource)
                    .ok_or_else(|| ModbusCommandConversionError::UnknownResource(resource.clone()))?;
                build_write_request(resource, &resource_def.resource_attributes, payload, proto, options)
            }
            Intent::Invoke { .. }
            | Intent::Send { .. }
            | Intent::Subscribe { .. }
            | Intent::Unsubscribe { .. } => Err(ModbusCommandConversionError::UnsupportedIntent),
        }
    }
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
            ModbusParserSeed::ReadCoils { address: attributes.address, quantity }
        }
        ModbusRegisterKind::DiscreteInput => {
            validate_bit_codec(attributes)?;
            builder
                .generate_get_discretes(attributes.address, quantity, &mut binary_frame)
                .map_err(map_rmodbus_build_error)?;
            ModbusParserSeed::ReadDiscretes { address: attributes.address, quantity }
        }
        ModbusRegisterKind::HoldingRegister => {
            validate_register_codec(attributes)?;
            builder
                .generate_get_holdings(attributes.address, quantity, &mut binary_frame)
                .map_err(map_rmodbus_build_error)?;
            ModbusParserSeed::ReadHoldings { address: attributes.address, quantity }
        }
        ModbusRegisterKind::InputRegister => {
            validate_register_codec(attributes)?;
            builder
                .generate_get_inputs(attributes.address, quantity, &mut binary_frame)
                .map_err(map_rmodbus_build_error)?;
            ModbusParserSeed::ReadInputs { address: attributes.address, quantity }
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
    payload: &[u8],
    proto: ModbusProto,
    options: &ModbusClientOptions,
) -> Result<ModbusRequest, ModbusCommandConversionError> {
    let mut builder = RmodbusRequest::new(options.unit_id, proto);
    let mut binary_frame = Vec::new();
    let parser_seed = match attributes.register_kind {
        ModbusRegisterKind::Coil => build_coil_write_request(
            resource,
            attributes,
            payload,
            &mut builder,
            &mut binary_frame,
        )?,
        ModbusRegisterKind::HoldingRegister => build_holding_write_request(
            resource,
            attributes,
            payload,
            &mut builder,
            &mut binary_frame,
        )?,
        ModbusRegisterKind::DiscreteInput | ModbusRegisterKind::InputRegister => {
            return Err(ModbusCommandConversionError::UnsupportedWrite(resource.to_string()))
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
    payload: &[u8],
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
            let values = payload.to_vec();
            builder
                .generate_set_coils_bulk(attributes.address, &values, binary_frame)
                .map_err(map_rmodbus_build_error)?;
            Ok(ModbusParserSeed::WriteMultipleCoils {
                address: attributes.address,
                values,
            })
        }
        _ => Err(ModbusCommandConversionError::UnsupportedWrite(resource.to_string())),
    }
}

fn build_holding_write_request(
    resource: &str,
    attributes: &ModbusResourceAttributes,
    payload: &[u8],
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
            let values = payload.to_vec();
            builder
                .generate_set_holdings_bulk_from_slice(attributes.address, &values, binary_frame)
                .map_err(map_rmodbus_build_error)?;
            Ok(ModbusParserSeed::WriteMultipleHoldingsBytes {
                address: attributes.address,
                values,
            })
        }
        ModbusValueCodec::Utf8String => {
            let value = core::str::from_utf8(payload)
                .map_err(|_| ModbusCommandConversionError::InvalidPayload("utf8 string expected"))?
                .to_string();
            builder
                .generate_set_holdings_string(attributes.address, &value, binary_frame)
                .map_err(map_rmodbus_build_error)?;
            Ok(ModbusParserSeed::WriteString {
                address: attributes.address,
                value,
            })
        }
        ModbusValueCodec::Bool | ModbusValueCodec::Bits => {
            Err(ModbusCommandConversionError::UnsupportedWrite(resource.to_string()))
        }
    }
}

pub(crate) fn proto_from_endpoint(endpoint: &DeviceEndpoint) -> Option<ModbusProto> {
    match endpoint {
        DeviceEndpoint::ModbusTCP(_) | DeviceEndpoint::ModbusUDP(_) => Some(ModbusProto::TcpUdp),
        DeviceEndpoint::ModbusRTU(_) => Some(ModbusProto::Rtu),
        DeviceEndpoint::ModbusASCII(_) => Some(ModbusProto::Ascii),
        _ => None,
    }
}

pub(crate) fn endpoint_options(endpoint: &DeviceEndpoint) -> Option<&ModbusClientOptions> {
    match endpoint {
        DeviceEndpoint::ModbusTCP(config) => Some(&config.options),
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
        ModbusValueCodec::Bytes | ModbusValueCodec::Utf8String => Err(
            ModbusCommandConversionError::InvalidResource(
                "quantity is required for raw bytes and string codecs".to_string(),
            ),
        ),
    }
}

fn decoder_for_codec(attributes: &ModbusResourceAttributes, quantity: u16) -> ModbusResponseDecoder {
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
        ModbusValueCodec::Bool | ModbusValueCodec::Bits => Err(
            ModbusCommandConversionError::InvalidResource(
                "register resources cannot use Bool or Bits codec".to_string(),
            ),
        ),
        _ => Ok(()),
    }
}

fn map_rmodbus_build_error(error: RmodbusError) -> ModbusCommandConversionError {
    ModbusCommandConversionError::InvalidPayload(match error {
        RmodbusError::OOB => "out of bounds",
        RmodbusError::FrameBroken => "frame broken",
        RmodbusError::FrameCRCError => "frame crc error",
        RmodbusError::Utf8Error => "utf8 error",
        _ => "rmodbus error",
    })
}

fn decode_bool(payload: &[u8]) -> Result<bool, ModbusCommandConversionError> {
    match payload {
        [value] => Ok(*value != 0),
        _ => Err(ModbusCommandConversionError::InvalidPayload(
            "bool payload must be exactly one byte",
        )),
    }
}

fn decode_u16(payload: &[u8]) -> Result<u16, ModbusCommandConversionError> {
    if payload.len() != 2 {
        return Err(ModbusCommandConversionError::InvalidPayload(
            "u16 payload must be exactly two bytes",
        ));
    }
    Ok(u16::from_be_bytes([payload[0], payload[1]]))
}

fn words_from_payload(
    attributes: &ModbusResourceAttributes,
    payload: &[u8],
) -> Result<Vec<u16>, ModbusCommandConversionError> {
    match attributes.codec {
        ModbusValueCodec::U32Be
        | ModbusValueCodec::I32Be
        | ModbusValueCodec::F32Be => words_from_bytes_be(payload),
        ModbusValueCodec::U32Le
        | ModbusValueCodec::I32Le
        | ModbusValueCodec::F32Le => words_from_bytes_le(payload),
        _ => Err(ModbusCommandConversionError::InvalidPayload(
            "word conversion requires 32-bit codec",
        )),
    }
}

fn words_from_bytes_be(payload: &[u8]) -> Result<Vec<u16>, ModbusCommandConversionError> {
    if payload.len() % 4 != 0 {
        return Err(ModbusCommandConversionError::InvalidPayload(
            "32-bit payload must be a multiple of four bytes",
        ));
    }
    Ok(payload
        .chunks_exact(4)
        .flat_map(|chunk| {
            [
                u16::from_be_bytes([chunk[0], chunk[1]]),
                u16::from_be_bytes([chunk[2], chunk[3]]),
            ]
        })
        .collect())
}

fn words_from_bytes_le(payload: &[u8]) -> Result<Vec<u16>, ModbusCommandConversionError> {
    if payload.len() % 4 != 0 {
        return Err(ModbusCommandConversionError::InvalidPayload(
            "32-bit payload must be a multiple of four bytes",
        ));
    }
    Ok(payload
        .chunks_exact(4)
        .flat_map(|chunk| {
            [
                u16::from_be_bytes([chunk[2], chunk[3]]),
                u16::from_be_bytes([chunk[0], chunk[1]]),
            ]
        })
        .collect())
}
