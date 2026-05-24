extern crate alloc;

use alloc::{string::ToString, vec::Vec};

use ferredge_core::prelude::*;
use rmodbus::{
    ModbusFrameBuf, ModbusProto, generate_ascii_frame,
    server::{ModbusFrame, context::ModbusContext, storage::ModbusStorageFull},
};

use crate::{
    ModbusDriver, ModbusRequest,
    attributes::{ModbusRegisterKind, ModbusResourceAttributes, ModbusValueCodec},
    codec::{decode_ascii_wire_frame, decode_modbus_response},
};

fn make_driver(endpoint: DeviceEndpoint) -> ModbusDriver {
    let mut resources = Map::default();
    resources.insert(
        "holding_u16".to_string(),
        DeviceResource {
            name: "holding_u16".to_string(),
            resource_attributes: ModbusResourceAttributes {
                address: 100,
                register_kind: ModbusRegisterKind::HoldingRegister,
                codec: ModbusValueCodec::U16,
                quantity: None,
                description: None,
            },
            unit: None,
            permission: Some(
                DeviceResourceAccessPermission::READ | DeviceResourceAccessPermission::WRITE,
            ),
        },
    );
    resources.insert(
        "coil_bit".to_string(),
        DeviceResource {
            name: "coil_bit".to_string(),
            resource_attributes: ModbusResourceAttributes {
                address: 12,
                register_kind: ModbusRegisterKind::Coil,
                codec: ModbusValueCodec::Bool,
                quantity: None,
                description: None,
            },
            unit: None,
            permission: Some(
                DeviceResourceAccessPermission::READ | DeviceResourceAccessPermission::WRITE,
            ),
        },
    );
    resources.insert(
        "holding_text".to_string(),
        DeviceResource {
            name: "holding_text".to_string(),
            resource_attributes: ModbusResourceAttributes {
                address: 120,
                register_kind: ModbusRegisterKind::HoldingRegister,
                codec: ModbusValueCodec::Utf8String,
                quantity: Some(4),
                description: None,
            },
            unit: None,
            permission: Some(
                DeviceResourceAccessPermission::READ | DeviceResourceAccessPermission::WRITE,
            ),
        },
    );
    resources.insert(
        "holding_bytes".to_string(),
        DeviceResource {
            name: "holding_bytes".to_string(),
            resource_attributes: ModbusResourceAttributes {
                address: 120,
                register_kind: ModbusRegisterKind::HoldingRegister,
                codec: ModbusValueCodec::Bytes,
                quantity: Some(4),
                description: None,
            },
            unit: None,
            permission: Some(
                DeviceResourceAccessPermission::READ | DeviceResourceAccessPermission::WRITE,
            ),
        },
    );
    resources.insert(
        "coil_bits".to_string(),
        DeviceResource {
            name: "coil_bits".to_string(),
            resource_attributes: ModbusResourceAttributes {
                address: 20,
                register_kind: ModbusRegisterKind::Coil,
                codec: ModbusValueCodec::Bits,
                quantity: Some(3),
                description: None,
            },
            unit: None,
            permission: Some(
                DeviceResourceAccessPermission::READ | DeviceResourceAccessPermission::WRITE,
            ),
        },
    );
    resources.insert(
        "input_u16".to_string(),
        DeviceResource {
            name: "input_u16".to_string(),
            resource_attributes: ModbusResourceAttributes {
                address: 200,
                register_kind: ModbusRegisterKind::InputRegister,
                codec: ModbusValueCodec::U16,
                quantity: None,
                description: None,
            },
            unit: None,
            permission: Some(DeviceResourceAccessPermission::READ),
        },
    );
    resources.insert(
        "discrete_bool".to_string(),
        DeviceResource {
            name: "discrete_bool".to_string(),
            resource_attributes: ModbusResourceAttributes {
                address: 30,
                register_kind: ModbusRegisterKind::DiscreteInput,
                codec: ModbusValueCodec::Bool,
                quantity: None,
                description: None,
            },
            unit: None,
            permission: Some(DeviceResourceAccessPermission::READ),
        },
    );
    resources.insert(
        "holding_invalid_bits".to_string(),
        DeviceResource {
            name: "holding_invalid_bits".to_string(),
            resource_attributes: ModbusResourceAttributes {
                address: 300,
                register_kind: ModbusRegisterKind::HoldingRegister,
                codec: ModbusValueCodec::Bits,
                quantity: Some(2),
                description: None,
            },
            unit: None,
            permission: Some(DeviceResourceAccessPermission::READ),
        },
    );
    resources.insert(
        "coil_invalid_u16".to_string(),
        DeviceResource {
            name: "coil_invalid_u16".to_string(),
            resource_attributes: ModbusResourceAttributes {
                address: 40,
                register_kind: ModbusRegisterKind::Coil,
                codec: ModbusValueCodec::U16,
                quantity: None,
                description: None,
            },
            unit: None,
            permission: Some(DeviceResourceAccessPermission::READ),
        },
    );
    resources.insert(
        "holding_zero_quantity".to_string(),
        DeviceResource {
            name: "holding_zero_quantity".to_string(),
            resource_attributes: ModbusResourceAttributes {
                address: 400,
                register_kind: ModbusRegisterKind::HoldingRegister,
                codec: ModbusValueCodec::U16,
                quantity: Some(0),
                description: None,
            },
            unit: None,
            permission: Some(DeviceResourceAccessPermission::READ),
        },
    );
    resources.insert(
        "holding_bytes_missing_quantity".to_string(),
        DeviceResource {
            name: "holding_bytes_missing_quantity".to_string(),
            resource_attributes: ModbusResourceAttributes {
                address: 500,
                register_kind: ModbusRegisterKind::HoldingRegister,
                codec: ModbusValueCodec::Bytes,
                quantity: None,
                description: None,
            },
            unit: None,
            permission: Some(DeviceResourceAccessPermission::READ),
        },
    );

    ModbusDriver::new(Device {
        id: "dvc-1".to_string(),
        name: "modbus-test".to_string(),
        status: DeviceStatus::Online,
        endpoint,
        metadata: None,
        max_connections: None,
        resources,
        message_endpoints: Vec::new(),
    })
}

fn tcp_endpoint() -> DeviceEndpoint {
    DeviceEndpoint::modbus_tcp(ModbusTcpEndpointConfig {
        addr: "127.0.0.1".to_string(),
        port: 502,
        options: ModbusClientOptions::default(),
    })
}

fn rtu_endpoint() -> DeviceEndpoint {
    DeviceEndpoint::modbus_rtu(ModbusRtuEndpointConfig {
        serial: SerialPortConfig {
            path: "/dev/ttyUSB0".to_string(),
            ..SerialPortConfig::default()
        },
        options: ModbusClientOptions::default(),
    })
}

fn rtu_over_tcp_endpoint() -> DeviceEndpoint {
    DeviceEndpoint::modbus_rtu_over_tcp(ModbusRtuOverTcpEndpointConfig {
        addr: "127.0.0.1".to_string(),
        port: 502,
        options: ModbusClientOptions::default(),
    })
}

fn ascii_endpoint() -> DeviceEndpoint {
    DeviceEndpoint::modbus_ascii(ModbusAsciiEndpointConfig {
        serial: SerialPortConfig {
            path: "/dev/ttyUSB0".to_string(),
            ..SerialPortConfig::default()
        },
        options: ModbusClientOptions::default(),
    })
}

fn command_read(resource: &str) -> Command {
    Command {
        id: "cmd-1".to_string(),
        source_device_id: None,
        target_device_id: "dvc-1".to_string(),
        intent: Intent::Read {
            resource: resource.to_string(),
            options: RequestOptions::default(),
        },
        correlation: None,
    }
}

fn command_write(resource: &str, payload: PayloadValue<'static>) -> Command {
    Command {
        id: "cmd-2".to_string(),
        source_device_id: None,
        target_device_id: "dvc-1".to_string(),
        intent: Intent::Write {
            resource: resource.to_string(),
            payload,
            options: RequestOptions::default(),
        },
        correlation: None,
    }
}

fn simulate_response(request: &ModbusRequest, response_proto: ModbusProto) -> Vec<u8> {
    let mut ctx = ModbusStorageFull::new();
    ctx.set_holding(100, 0x1234).unwrap();
    ctx.set_coil(12, true).unwrap();
    ctx.set_holding(120, 0x6869).unwrap();
    ctx.set_holding(121, 0x2100).unwrap();

    let mut frame_buf: ModbusFrameBuf = [0; 256];
    let binary_request = if request.proto == ModbusProto::Ascii {
        let parsed =
            rmodbus::parse_ascii_frame(&request.frame, request.frame.len(), &mut frame_buf, 0)
                .unwrap() as usize;
        &frame_buf[..parsed]
    } else {
        &request.frame
    };

    let mut response = Vec::new();
    let mut frame = ModbusFrame::new(
        request.unit_id,
        binary_request,
        response_proto,
        &mut response,
    );
    frame.parse().unwrap();
    if frame.processing_required {
        if frame.readonly {
            frame.process_read(&ctx).unwrap();
        } else {
            frame.process_write(&mut ctx).unwrap();
        }
    }
    if frame.response_required {
        frame.finalize_response().unwrap();
    }
    if response_proto == ModbusProto::Ascii {
        let mut ascii = Vec::new();
        generate_ascii_frame(&response, &mut ascii).unwrap();
        ascii
    } else {
        response
    }
}

fn simulate_response_with_context<F>(
    request: &ModbusRequest,
    response_proto: ModbusProto,
    configure: F,
) -> Vec<u8>
where
    F: FnOnce(&mut ModbusStorageFull),
{
    let mut ctx = ModbusStorageFull::new();
    configure(&mut ctx);

    let mut frame_buf: ModbusFrameBuf = [0; 256];
    let binary_request = if request.proto == ModbusProto::Ascii {
        let parsed =
            rmodbus::parse_ascii_frame(&request.frame, request.frame.len(), &mut frame_buf, 0)
                .unwrap() as usize;
        &frame_buf[..parsed]
    } else {
        &request.frame
    };

    let mut response = Vec::new();
    let mut frame = ModbusFrame::new(
        request.unit_id,
        binary_request,
        response_proto,
        &mut response,
    );
    frame.parse().unwrap();
    if frame.processing_required {
        if frame.readonly {
            frame.process_read(&ctx).unwrap();
        } else {
            frame.process_write(&mut ctx).unwrap();
        }
    }
    if frame.response_required {
        frame.finalize_response().unwrap();
    }
    if response_proto == ModbusProto::Ascii {
        let mut ascii = Vec::new();
        generate_ascii_frame(&response, &mut ascii).unwrap();
        ascii
    } else {
        response
    }
}

#[test]
fn build_read_request_for_holding_register() {
    let driver = make_driver(tcp_endpoint());
    let request = driver.bridge_request(command_read("holding_u16")).unwrap();

    assert_eq!(request.proto, ModbusProto::TcpUdp);
    assert_eq!(request.decoder, crate::ModbusResponseDecoder::U16);
    assert!(!request.is_write);
    assert!(!request.frame.is_empty());
}

#[test]
fn decode_tcp_holding_register_response() {
    let driver = make_driver(tcp_endpoint());
    let request = driver.bridge_request(command_read("holding_u16")).unwrap();
    let response = simulate_response(&request, ModbusProto::TcpUdp);
    let payload = decode_modbus_response(&request, &response).unwrap();
    assert_eq!(payload, PayloadValue::U64(0x1234));
}

#[test]
fn decode_rtu_coil_response() {
    let driver = make_driver(rtu_endpoint());
    let request = driver.bridge_request(command_read("coil_bit")).unwrap();
    let response = simulate_response(&request, ModbusProto::Rtu);
    let payload = decode_modbus_response(&request, &response).unwrap();
    assert_eq!(payload, PayloadValue::Bool(true));
}

#[test]
fn build_rtu_over_tcp_request_uses_rtu_proto() {
    let driver = make_driver(rtu_over_tcp_endpoint());
    let request = driver.bridge_request(command_read("holding_u16")).unwrap();

    assert_eq!(request.proto, ModbusProto::Rtu);
    assert!(!request.is_write);
}

#[test]
fn decode_ascii_string_response() {
    let driver = make_driver(ascii_endpoint());
    let request = driver.bridge_request(command_read("holding_text")).unwrap();
    let response = simulate_response(&request, ModbusProto::Ascii);
    let decoded = decode_ascii_wire_frame(&response).unwrap();
    let payload = decode_modbus_response(&request, &decoded).unwrap();
    assert_eq!(payload, PayloadValue::String("hi!".to_string().into()));
}

#[test]
fn build_response_defers_payload_decode() {
    let driver = make_driver(tcp_endpoint());
    let request = driver.bridge_request(command_read("holding_u16")).unwrap();
    let response = simulate_response(&request, ModbusProto::TcpUdp);
    let native = crate::codec::build_modbus_response(&request, response).unwrap();

    assert_eq!(native.payload().unwrap(), PayloadValue::U64(0x1234));
}

#[test]
fn bytes_response_payload_borrows_from_frame() {
    let driver = make_driver(tcp_endpoint());
    let request = driver
        .bridge_request(command_read("holding_bytes"))
        .unwrap();
    let response = simulate_response(&request, ModbusProto::TcpUdp);
    let native = crate::codec::build_modbus_response(&request, response).unwrap();

    match native.payload().unwrap() {
        PayloadValue::Bytes(bytes) => {
            assert!(matches!(bytes, std::borrow::Cow::Borrowed(_)));
            assert_eq!(bytes.as_ref(), b"hi!\0\0\0\0\0");
        }
        other => panic!("expected bytes payload, got {other:?}"),
    }
}

#[test]
fn string_response_payload_borrows_from_frame() {
    let driver = make_driver(tcp_endpoint());
    let request = driver.bridge_request(command_read("holding_text")).unwrap();
    let response = simulate_response(&request, ModbusProto::TcpUdp);
    let native = crate::codec::build_modbus_response(&request, response).unwrap();

    match native.payload().unwrap() {
        PayloadValue::String(value) => {
            assert!(matches!(value, std::borrow::Cow::Borrowed(_)));
            assert_eq!(value.as_ref(), "hi!");
        }
        other => panic!("expected string payload, got {other:?}"),
    }
}

#[test]
fn bits_response_still_decodes_to_same_values() {
    let driver = make_driver(tcp_endpoint());
    let request = driver.bridge_request(command_read("coil_bits")).unwrap();
    let response = simulate_response(&request, ModbusProto::TcpUdp);
    let payload = decode_modbus_response(&request, &response).unwrap();

    assert_eq!(
        payload,
        PayloadValue::List(
            vec![
                PayloadValue::Bool(false),
                PayloadValue::Bool(false),
                PayloadValue::Bool(false),
            ]
            .into(),
        )
    );
}

#[test]
fn lazy_string_decode_error_surfaces_on_payload_access() {
    let driver = make_driver(tcp_endpoint());
    let request = driver.bridge_request(command_read("holding_text")).unwrap();
    let response = simulate_response_with_context(&request, ModbusProto::TcpUdp, |ctx| {
        ctx.set_holding(120, 0xFFFF).unwrap();
        ctx.set_holding(121, 0x0000).unwrap();
    });
    let native = crate::codec::build_modbus_response(&request, response).unwrap();

    let error = native.payload().unwrap_err();
    assert_eq!(
        error,
        crate::ModbusCommandConversionError::InvalidPayload(
            "string payload must be valid utf8".to_string()
        )
    );
}

#[test]
fn build_write_single_holding_request() {
    let driver = make_driver(tcp_endpoint());
    let request = driver
        .bridge_request(command_write("holding_u16", PayloadValue::U64(0x4321)))
        .unwrap();

    assert!(request.is_write);
    let response = simulate_response(&request, ModbusProto::TcpUdp);
    let payload = decode_modbus_response(&request, &response).unwrap();
    assert_eq!(payload, PayloadValue::Null);
}

#[test]
fn build_write_single_coil_request() {
    let driver = make_driver(tcp_endpoint());
    let request = driver
        .bridge_request(command_write("coil_bit", PayloadValue::Bool(true)))
        .unwrap();

    assert!(request.is_write);
    assert_eq!(
        request.parser_seed,
        crate::ModbusParserSeed::WriteSingleCoil {
            address: 12,
            value: true,
        }
    );
    let response = simulate_response(&request, ModbusProto::TcpUdp);
    let payload = decode_modbus_response(&request, &response).unwrap();
    assert_eq!(payload, PayloadValue::Null);
}

#[test]
fn build_write_multiple_coils_request() {
    let driver = make_driver(tcp_endpoint());
    let request = driver
        .bridge_request(command_write(
            "coil_bits",
            PayloadValue::List(
                vec![
                    PayloadValue::Bool(true),
                    PayloadValue::Bool(false),
                    PayloadValue::Bool(true),
                ]
                .into(),
            ),
        ))
        .unwrap();

    assert!(request.is_write);
    assert_eq!(
        request.parser_seed,
        crate::ModbusParserSeed::WriteMultipleCoils {
            address: 20,
            values: vec![1, 0, 1],
        }
    );
    let response = simulate_response(&request, ModbusProto::TcpUdp);
    let payload = decode_modbus_response(&request, &response).unwrap();
    assert_eq!(payload, PayloadValue::Null);
}

#[test]
fn write_to_input_register_is_rejected() {
    let driver = make_driver(tcp_endpoint());
    let error = driver
        .bridge_request(command_write("input_u16", PayloadValue::U64(9)))
        .unwrap_err();

    assert_eq!(
        error,
        crate::ModbusCommandConversionError::UnsupportedWrite("input_u16".to_string())
    );
}

#[test]
fn write_to_discrete_input_is_rejected() {
    let driver = make_driver(tcp_endpoint());
    let error = driver
        .bridge_request(command_write("discrete_bool", PayloadValue::Bool(true)))
        .unwrap_err();

    assert_eq!(
        error,
        crate::ModbusCommandConversionError::UnsupportedWrite("discrete_bool".to_string())
    );
}

#[test]
fn holding_register_with_bit_codec_is_rejected() {
    let driver = make_driver(tcp_endpoint());
    let error = driver
        .bridge_request(command_read("holding_invalid_bits"))
        .unwrap_err();

    assert_eq!(
        error,
        crate::ModbusCommandConversionError::InvalidResource(
            "register resources cannot use Bool or Bits codec".to_string()
        )
    );
}

#[test]
fn coil_with_register_codec_is_rejected() {
    let driver = make_driver(tcp_endpoint());
    let error = driver
        .bridge_request(command_read("coil_invalid_u16"))
        .unwrap_err();

    assert_eq!(
        error,
        crate::ModbusCommandConversionError::InvalidResource(
            "bit resources require Bool or Bits codec".to_string()
        )
    );
}

#[test]
fn zero_quantity_is_rejected() {
    let driver = make_driver(tcp_endpoint());
    let error = driver
        .bridge_request(command_read("holding_zero_quantity"))
        .unwrap_err();

    assert_eq!(
        error,
        crate::ModbusCommandConversionError::InvalidResource(
            "quantity must be greater than zero".to_string()
        )
    );
}

#[test]
fn bytes_without_quantity_is_rejected() {
    let driver = make_driver(tcp_endpoint());
    let error = driver
        .bridge_request(command_read("holding_bytes_missing_quantity"))
        .unwrap_err();

    assert_eq!(
        error,
        crate::ModbusCommandConversionError::InvalidResource(
            "quantity is required for raw bytes and string codecs".to_string()
        )
    );
}

#[test]
fn invalid_coil_payload_is_rejected() {
    let driver = make_driver(tcp_endpoint());
    let error = driver
        .bridge_request(command_write(
            "coil_bit",
            PayloadValue::String("true".to_string().into()),
        ))
        .unwrap_err();

    assert_eq!(
        error,
        crate::ModbusCommandConversionError::InvalidPayload(
            "bool payload must be a boolean".to_string()
        )
    );
}

#[test]
fn invalid_holding_payload_is_rejected() {
    let driver = make_driver(tcp_endpoint());
    let error = driver
        .bridge_request(command_write(
            "holding_u16",
            PayloadValue::List(vec![PayloadValue::U64(1), PayloadValue::U64(2)].into()),
        ))
        .unwrap_err();

    assert_eq!(
        error,
        crate::ModbusCommandConversionError::InvalidPayload(
            "u16 payload must be a single 16-bit value".to_string()
        )
    );
}

#[test]
fn invalid_string_payload_is_rejected() {
    let driver = make_driver(tcp_endpoint());
    let error = driver
        .bridge_request(command_write("holding_text", PayloadValue::U64(1)))
        .unwrap_err();

    assert_eq!(
        error,
        crate::ModbusCommandConversionError::InvalidPayload(
            "string payload must be utf8 text".to_string()
        )
    );
}
