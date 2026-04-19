extern crate alloc;

use alloc::{format, string::String, vec::Vec};

use ferredge_core::prelude::PayloadValue;
use rmodbus::{
    ErrorKind as RmodbusError, ModbusFrameBuf, ModbusProto, client::ModbusRequest as RmodbusRequest,
};

use crate::types::ModbusValue;
use crate::{ModbusParserSeed, ModbusRequest, ModbusResponse, ModbusResponseDecoder};

pub(crate) fn build_modbus_response(
    request: &ModbusRequest,
    raw_frame: Vec<u8>,
) -> Result<ModbusResponse, String> {
    let payload = payload_value_from_modbus_value(decode_modbus_value(request, &raw_frame)?);
    Ok(ModbusResponse {
        frame: raw_frame,
        payload,
    })
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn decode_modbus_response(
    request: &ModbusRequest,
    frame: &[u8],
) -> Result<PayloadValue, String> {
    decode_modbus_value(request, frame).map(payload_value_from_modbus_value)
}

pub(crate) fn decode_modbus_value(
    request: &ModbusRequest,
    frame: &[u8],
) -> Result<ModbusValue, String> {
    let parser = make_parser(request)?;
    match &request.decoder {
        ModbusResponseDecoder::Ack => parser
            .parse_ok(frame)
            .map(|_| ModbusValue::Ack)
            .map_err(map_rmodbus_error),
        ModbusResponseDecoder::Bool | ModbusResponseDecoder::Bits { .. } => {
            let mut values = Vec::new();
            parser
                .parse_bool_u8(frame, &mut values)
                .map_err(map_rmodbus_error)?;
            if matches!(request.decoder, ModbusResponseDecoder::Bool) {
                Ok(ModbusValue::Bool(
                    values.first().copied().unwrap_or_default() != 0,
                ))
            } else {
                Ok(ModbusValue::Bits(
                    values.into_iter().map(|value| value != 0).collect(),
                ))
            }
        }
        ModbusResponseDecoder::U16 => {
            let mut values = Vec::new();
            parser
                .parse_u16(frame, &mut values)
                .map_err(map_rmodbus_error)?;
            Ok(ModbusValue::U16(values))
        }
        ModbusResponseDecoder::I16 => {
            let mut values = Vec::new();
            parser
                .parse_i16(frame, &mut values)
                .map_err(map_rmodbus_error)?;
            Ok(ModbusValue::I16(values))
        }
        ModbusResponseDecoder::U32Be | ModbusResponseDecoder::U32Le => {
            let mut values = Vec::new();
            match request.decoder {
                ModbusResponseDecoder::U32Be => parser
                    .parse_u32_be(frame, &mut values)
                    .map_err(map_rmodbus_error)?,
                _ => parser
                    .parse_u32_le(frame, &mut values)
                    .map_err(map_rmodbus_error)?,
            }
            Ok(ModbusValue::U32(values))
        }
        ModbusResponseDecoder::I32Be | ModbusResponseDecoder::I32Le => {
            let mut values = Vec::new();
            match request.decoder {
                ModbusResponseDecoder::I32Be => parser
                    .parse_i32_be(frame, &mut values)
                    .map_err(map_rmodbus_error)?,
                _ => parser
                    .parse_i32_le(frame, &mut values)
                    .map_err(map_rmodbus_error)?,
            }
            Ok(ModbusValue::I32(values))
        }
        ModbusResponseDecoder::F32Be | ModbusResponseDecoder::F32Le => {
            let mut values = Vec::new();
            match request.decoder {
                ModbusResponseDecoder::F32Be => parser
                    .parse_f32_be(frame, &mut values)
                    .map_err(map_rmodbus_error)?,
                _ => parser
                    .parse_f32_le(frame, &mut values)
                    .map_err(map_rmodbus_error)?,
            }
            Ok(ModbusValue::F32(values))
        }
        ModbusResponseDecoder::Bytes => parser
            .parse_slice(frame)
            .map(|slice| ModbusValue::Bytes(slice.to_vec()))
            .map_err(map_rmodbus_error),
        ModbusResponseDecoder::Utf8String => {
            let mut value = String::new();
            parser
                .parse_string(frame, &mut value)
                .map_err(map_rmodbus_error)?;
            Ok(ModbusValue::Utf8String(value))
        }
    }
}

pub(crate) fn payload_value_from_modbus_value(value: ModbusValue) -> PayloadValue {
    match value {
        ModbusValue::Ack => PayloadValue::Null,
        ModbusValue::Bool(value) => PayloadValue::Bool(value),
        ModbusValue::Bits(values) => {
            PayloadValue::List(values.into_iter().map(PayloadValue::Bool).collect())
        }
        ModbusValue::U16(values) => one_or_list_u16(values),
        ModbusValue::I16(values) => one_or_list_i16(values),
        ModbusValue::U32(values) => one_or_list_u32(values),
        ModbusValue::I32(values) => one_or_list_i32(values),
        ModbusValue::F32(values) => one_or_list_f32(values),
        ModbusValue::Bytes(bytes) => PayloadValue::Bytes(bytes),
        ModbusValue::Utf8String(value) => PayloadValue::String(value),
    }
}

fn one_or_list_u16(values: Vec<u16>) -> PayloadValue {
    one_or_list(values, |value| PayloadValue::U64(u64::from(value)))
}

fn one_or_list_i16(values: Vec<i16>) -> PayloadValue {
    one_or_list(values, |value| PayloadValue::I64(i64::from(value)))
}

fn one_or_list_u32(values: Vec<u32>) -> PayloadValue {
    one_or_list(values, |value| PayloadValue::U64(u64::from(value)))
}

fn one_or_list_i32(values: Vec<i32>) -> PayloadValue {
    one_or_list(values, |value| PayloadValue::I64(i64::from(value)))
}

fn one_or_list_f32(values: Vec<f32>) -> PayloadValue {
    one_or_list(values, |value| PayloadValue::F64(f64::from(value)))
}

fn one_or_list<T, F>(values: Vec<T>, map: F) -> PayloadValue
where
    F: Fn(T) -> PayloadValue,
{
    let mut values = values;
    if values.len() == 1 {
        map(values.remove(0))
    } else {
        PayloadValue::List(values.into_iter().map(map).collect())
    }
}

fn make_parser(request: &ModbusRequest) -> Result<RmodbusRequest, String> {
    let mut parser = RmodbusRequest::new(request.unit_id, request.proto);
    let mut sink = Vec::new();
    match &request.parser_seed {
        ModbusParserSeed::ReadCoils { address, quantity } => parser
            .generate_get_coils(*address, *quantity, &mut sink)
            .map_err(map_rmodbus_error)?,
        ModbusParserSeed::ReadDiscretes { address, quantity } => parser
            .generate_get_discretes(*address, *quantity, &mut sink)
            .map_err(map_rmodbus_error)?,
        ModbusParserSeed::ReadHoldings { address, quantity } => parser
            .generate_get_holdings(*address, *quantity, &mut sink)
            .map_err(map_rmodbus_error)?,
        ModbusParserSeed::ReadInputs { address, quantity } => parser
            .generate_get_inputs(*address, *quantity, &mut sink)
            .map_err(map_rmodbus_error)?,
        ModbusParserSeed::WriteSingleCoil { address, value } => parser
            .generate_set_coil(*address, *value, &mut sink)
            .map_err(map_rmodbus_error)?,
        ModbusParserSeed::WriteSingleHolding { address, value } => parser
            .generate_set_holding(*address, *value, &mut sink)
            .map_err(map_rmodbus_error)?,
        ModbusParserSeed::WriteMultipleCoils { address, values } => parser
            .generate_set_coils_bulk(*address, values, &mut sink)
            .map_err(map_rmodbus_error)?,
        ModbusParserSeed::WriteMultipleHoldings { address, values } => parser
            .generate_set_holdings_bulk(*address, values, &mut sink)
            .map_err(map_rmodbus_error)?,
        ModbusParserSeed::WriteMultipleHoldingsBytes { address, values } => parser
            .generate_set_holdings_bulk_from_slice(*address, values, &mut sink)
            .map_err(map_rmodbus_error)?,
        ModbusParserSeed::WriteString { address, value } => parser
            .generate_set_holdings_string(*address, value, &mut sink)
            .map_err(map_rmodbus_error)?,
    }
    Ok(parser)
}

pub(crate) fn map_rmodbus_error(error: RmodbusError) -> String {
    format!("rmodbus error: {error:?}")
}

pub(crate) fn encode_wire_frame(
    proto: ModbusProto,
    binary_frame: &[u8],
) -> Result<Vec<u8>, RmodbusError> {
    if proto == ModbusProto::Ascii {
        let mut ascii = Vec::new();
        rmodbus::generate_ascii_frame(binary_frame, &mut ascii)?;
        Ok(ascii)
    } else {
        Ok(binary_frame.to_vec())
    }
}

pub(crate) fn decode_ascii_wire_frame(frame: &[u8]) -> Result<Vec<u8>, String> {
    let mut buf: ModbusFrameBuf = [0; 256];
    let parsed = rmodbus::parse_ascii_frame(frame, frame.len(), &mut buf, 0)
        .map_err(map_rmodbus_error)? as usize;
    Ok(buf[..parsed].to_vec())
}
