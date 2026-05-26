extern crate alloc;

use alloc::{borrow::Cow, format, string::ToString, vec::Vec};

use ferredge_core::prelude::PayloadValue;
use rmodbus::{
    ErrorKind as RmodbusError, ModbusFrameBuf, ModbusProto, client::ModbusRequest as RmodbusRequest,
};

use crate::types::ModbusValue;
use crate::{
    ModbusCommandConversionError, ModbusParserSeed, ModbusRequest, ModbusResponse,
    ModbusResponseDecoder,
};

pub(crate) fn build_modbus_response(
    request: &ModbusRequest,
    raw_frame: Vec<u8>,
) -> Result<ModbusResponse, String> {
    Ok(ModbusResponse {
        frame: raw_frame,
        proto: request.proto,
        unit_id: request.unit_id,
        parser_seed: request.parser_seed.clone(),
        decoder: request.decoder.clone(),
    })
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn decode_modbus_response<'a>(
    request: &ModbusRequest,
    frame: &'a [u8],
) -> Result<PayloadValue<'a>, ModbusCommandConversionError> {
    decode_modbus_value(
        request.proto,
        request.unit_id,
        &request.parser_seed,
        &request.decoder,
        frame,
    )
    .map(payload_value_from_modbus_value)
}

pub(crate) fn decode_modbus_value<'a>(
    proto: ModbusProto,
    unit_id: u8,
    parser_seed: &ModbusParserSeed,
    decoder: &ModbusResponseDecoder,
    frame: &'a [u8],
) -> Result<ModbusValue<'a>, ModbusCommandConversionError> {
    let parser = make_parser(proto, unit_id, parser_seed)?;
    match decoder {
        ModbusResponseDecoder::Ack => parser
            .parse_ok(frame)
            .map(|_| ModbusValue::Ack)
            .map_err(map_rmodbus_decode_error),
        ModbusResponseDecoder::Bool | ModbusResponseDecoder::Bits { .. } => {
            let mut values = Vec::new();
            parser
                .parse_bool_u8(frame, &mut values)
                .map_err(map_rmodbus_decode_error)?;
            if matches!(decoder, ModbusResponseDecoder::Bool) {
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
                .map_err(map_rmodbus_decode_error)?;
            Ok(ModbusValue::U16(values))
        }
        ModbusResponseDecoder::I16 => {
            let mut values = Vec::new();
            parser
                .parse_i16(frame, &mut values)
                .map_err(map_rmodbus_decode_error)?;
            Ok(ModbusValue::I16(values))
        }
        ModbusResponseDecoder::U32Be | ModbusResponseDecoder::U32Le => {
            let mut values = Vec::new();
            match decoder {
                ModbusResponseDecoder::U32Be => parser
                    .parse_u32_be(frame, &mut values)
                    .map_err(map_rmodbus_decode_error)?,
                _ => parser
                    .parse_u32_le(frame, &mut values)
                    .map_err(map_rmodbus_decode_error)?,
            }
            Ok(ModbusValue::U32(values))
        }
        ModbusResponseDecoder::I32Be | ModbusResponseDecoder::I32Le => {
            let mut values = Vec::new();
            match decoder {
                ModbusResponseDecoder::I32Be => parser
                    .parse_i32_be(frame, &mut values)
                    .map_err(map_rmodbus_decode_error)?,
                _ => parser
                    .parse_i32_le(frame, &mut values)
                    .map_err(map_rmodbus_decode_error)?,
            }
            Ok(ModbusValue::I32(values))
        }
        ModbusResponseDecoder::F32Be | ModbusResponseDecoder::F32Le => {
            let mut values = Vec::new();
            match decoder {
                ModbusResponseDecoder::F32Be => parser
                    .parse_f32_be(frame, &mut values)
                    .map_err(map_rmodbus_decode_error)?,
                _ => parser
                    .parse_f32_le(frame, &mut values)
                    .map_err(map_rmodbus_decode_error)?,
            }
            Ok(ModbusValue::F32(values))
        }
        ModbusResponseDecoder::Bytes => {
            response_data_slice(proto, frame).map(|slice| ModbusValue::Bytes(Cow::Borrowed(slice)))
        }
        ModbusResponseDecoder::Utf8String => response_data_slice(proto, frame)
            .map(trim_trailing_nuls)
            .and_then(|bytes| {
                core::str::from_utf8(bytes)
                    .map(|value| ModbusValue::Utf8String(Cow::Borrowed(value)))
                    .map_err(|_| {
                        ModbusCommandConversionError::InvalidPayload(
                            "string payload must be valid utf8".to_string(),
                        )
                    })
            }),
    }
}

pub(crate) fn payload_value_from_modbus_value<'a>(value: ModbusValue<'a>) -> PayloadValue<'a> {
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

fn one_or_list_u16<'a>(values: Vec<u16>) -> PayloadValue<'a> {
    one_or_list(values, |value| PayloadValue::U64(u64::from(value)))
}

fn one_or_list_i16<'a>(values: Vec<i16>) -> PayloadValue<'a> {
    one_or_list(values, |value| PayloadValue::I64(i64::from(value)))
}

fn one_or_list_u32<'a>(values: Vec<u32>) -> PayloadValue<'a> {
    one_or_list(values, |value| PayloadValue::U64(u64::from(value)))
}

fn one_or_list_i32<'a>(values: Vec<i32>) -> PayloadValue<'a> {
    one_or_list(values, |value| PayloadValue::I64(i64::from(value)))
}

fn one_or_list_f32<'a>(values: Vec<f32>) -> PayloadValue<'a> {
    one_or_list(values, |value| PayloadValue::F64(f64::from(value)))
}

#[inline(always)]
fn one_or_list<'a, T, F>(values: Vec<T>, map: F) -> PayloadValue<'a>
where
    T: Copy,
    F: Fn(T) -> PayloadValue<'a>,
{
    if let [value] = values.as_slice() {
        map(*value)
    } else {
        PayloadValue::List(values.into_iter().map(map).collect())
    }
}

fn make_parser(
    proto: ModbusProto,
    unit_id: u8,
    parser_seed: &ModbusParserSeed,
) -> Result<RmodbusRequest, ModbusCommandConversionError> {
    let mut parser = RmodbusRequest::new(unit_id, proto);
    let mut sink = Vec::new();
    match parser_seed {
        ModbusParserSeed::ReadCoils { address, quantity } => parser
            .generate_get_coils(*address, *quantity, &mut sink)
            .map_err(map_rmodbus_decode_error)?,
        ModbusParserSeed::ReadDiscretes { address, quantity } => parser
            .generate_get_discretes(*address, *quantity, &mut sink)
            .map_err(map_rmodbus_decode_error)?,
        ModbusParserSeed::ReadHoldings { address, quantity } => parser
            .generate_get_holdings(*address, *quantity, &mut sink)
            .map_err(map_rmodbus_decode_error)?,
        ModbusParserSeed::ReadInputs { address, quantity } => parser
            .generate_get_inputs(*address, *quantity, &mut sink)
            .map_err(map_rmodbus_decode_error)?,
        ModbusParserSeed::WriteSingleCoil { address, value } => parser
            .generate_set_coil(*address, *value, &mut sink)
            .map_err(map_rmodbus_decode_error)?,
        ModbusParserSeed::WriteSingleHolding { address, value } => parser
            .generate_set_holding(*address, *value, &mut sink)
            .map_err(map_rmodbus_decode_error)?,
        ModbusParserSeed::WriteMultipleCoils { address, values } => parser
            .generate_set_coils_bulk(*address, values, &mut sink)
            .map_err(map_rmodbus_decode_error)?,
        ModbusParserSeed::WriteMultipleHoldings { address, values } => parser
            .generate_set_holdings_bulk(*address, values, &mut sink)
            .map_err(map_rmodbus_decode_error)?,
        ModbusParserSeed::WriteMultipleHoldingsBytes { address, values } => parser
            .generate_set_holdings_bulk_from_slice(*address, values, &mut sink)
            .map_err(map_rmodbus_decode_error)?,
        ModbusParserSeed::WriteString { address, value } => parser
            .generate_set_holdings_string(*address, value, &mut sink)
            .map_err(map_rmodbus_decode_error)?,
    }
    Ok(parser)
}

fn trim_trailing_nuls(bytes: &[u8]) -> &[u8] {
    let end = bytes
        .iter()
        .rposition(|byte| *byte != 0)
        .map(|index| index + 1)
        .unwrap_or(0);
    &bytes[..end]
}

fn response_data_slice<'a>(
    proto: ModbusProto,
    frame: &'a [u8],
) -> Result<&'a [u8], ModbusCommandConversionError> {
    let (count_index, data_index) = match proto {
        ModbusProto::TcpUdp => (8usize, 9usize),
        ModbusProto::Rtu | ModbusProto::Ascii => (2usize, 3usize),
    };
    let byte_count = *frame.get(count_index).ok_or_else(|| {
        ModbusCommandConversionError::InvalidPayload(
            "response frame shorter than byte count header".to_string(),
        )
    })? as usize;
    let data_end = data_index.checked_add(byte_count).ok_or_else(|| {
        ModbusCommandConversionError::InvalidPayload("response byte count overflow".to_string())
    })?;
    if frame.len() < data_end {
        return Err(ModbusCommandConversionError::InvalidPayload(
            "response frame shorter than declared payload".to_string(),
        ));
    }
    Ok(&frame[data_index..data_end])
}

fn map_rmodbus_decode_error(error: RmodbusError) -> ModbusCommandConversionError {
    ModbusCommandConversionError::InvalidPayload(map_rmodbus_error(error))
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
