extern crate alloc;

use alloc::{format, string::String, vec::Vec};

use rmodbus::{
    ErrorKind as RmodbusError, ModbusFrameBuf, ModbusProto, client::ModbusRequest as RmodbusRequest,
};

use crate::{ModbusParserSeed, ModbusRequest, ModbusResponse, ModbusResponseDecoder};

pub(crate) fn build_modbus_response(
    request: &ModbusRequest,
    raw_frame: Vec<u8>,
) -> Result<ModbusResponse, String> {
    let payload = decode_modbus_response(request, &raw_frame)?;
    Ok(ModbusResponse {
        frame: raw_frame,
        payload,
    })
}

pub(crate) fn decode_modbus_response(
    request: &ModbusRequest,
    frame: &[u8],
) -> Result<Vec<u8>, String> {
    let parser = make_parser(request)?;
    match &request.decoder {
        ModbusResponseDecoder::Ack => parser
            .parse_ok(frame)
            .map(|_| Vec::new())
            .map_err(map_rmodbus_error),
        ModbusResponseDecoder::Bool | ModbusResponseDecoder::Bits { .. } => {
            let mut values = Vec::new();
            parser
                .parse_bool_u8(frame, &mut values)
                .map_err(map_rmodbus_error)?;
            Ok(values)
        }
        ModbusResponseDecoder::U16 => {
            let mut values = Vec::new();
            parser
                .parse_u16(frame, &mut values)
                .map_err(map_rmodbus_error)?;
            Ok(values.into_iter().flat_map(u16::to_be_bytes).collect())
        }
        ModbusResponseDecoder::I16 => {
            let mut values = Vec::new();
            parser
                .parse_i16(frame, &mut values)
                .map_err(map_rmodbus_error)?;
            Ok(values.into_iter().flat_map(i16::to_be_bytes).collect())
        }
        ModbusResponseDecoder::U32Be => {
            let mut values = Vec::new();
            parser
                .parse_u32_be(frame, &mut values)
                .map_err(map_rmodbus_error)?;
            Ok(values.into_iter().flat_map(u32::to_be_bytes).collect())
        }
        ModbusResponseDecoder::U32Le => {
            let mut values = Vec::new();
            parser
                .parse_u32_le(frame, &mut values)
                .map_err(map_rmodbus_error)?;
            Ok(values.into_iter().flat_map(u32::to_be_bytes).collect())
        }
        ModbusResponseDecoder::I32Be => {
            let mut values = Vec::new();
            parser
                .parse_i32_be(frame, &mut values)
                .map_err(map_rmodbus_error)?;
            Ok(values.into_iter().flat_map(i32::to_be_bytes).collect())
        }
        ModbusResponseDecoder::I32Le => {
            let mut values = Vec::new();
            parser
                .parse_i32_le(frame, &mut values)
                .map_err(map_rmodbus_error)?;
            Ok(values.into_iter().flat_map(i32::to_be_bytes).collect())
        }
        ModbusResponseDecoder::F32Be => {
            let mut values = Vec::new();
            parser
                .parse_f32_be(frame, &mut values)
                .map_err(map_rmodbus_error)?;
            Ok(values
                .into_iter()
                .flat_map(|v| v.to_bits().to_be_bytes())
                .collect())
        }
        ModbusResponseDecoder::F32Le => {
            let mut values = Vec::new();
            parser
                .parse_f32_le(frame, &mut values)
                .map_err(map_rmodbus_error)?;
            Ok(values
                .into_iter()
                .flat_map(|v| v.to_bits().to_be_bytes())
                .collect())
        }
        ModbusResponseDecoder::Bytes => parser
            .parse_slice(frame)
            .map(|slice| slice.to_vec())
            .map_err(map_rmodbus_error),
        ModbusResponseDecoder::Utf8String => {
            let mut value = String::new();
            parser
                .parse_string(frame, &mut value)
                .map_err(map_rmodbus_error)?;
            Ok(value.into_bytes())
        }
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
