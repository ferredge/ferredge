extern crate alloc;

use alloc::{string::ToString, vec::Vec};

use ferredge_core::prelude::*;
use rmodbus::{
    ModbusFrameBuf, ModbusProto, generate_ascii_frame, server::{
        ModbusFrame,
        context::ModbusContext,
        storage::ModbusStorageFull,
    },
};

use crate::{
    ModbusCommandRef, ModbusDriver, ModbusRequest,
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
            permission: Some(DeviceResourceAccessPermission::READ | DeviceResourceAccessPermission::WRITE),
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
            permission: Some(DeviceResourceAccessPermission::READ | DeviceResourceAccessPermission::WRITE),
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
            permission: Some(DeviceResourceAccessPermission::READ | DeviceResourceAccessPermission::WRITE),
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
        },
        correlation: None,
    }
}

fn command_write(resource: &str, payload: Vec<u8>) -> Command {
    Command {
        id: "cmd-2".to_string(),
        source_device_id: None,
        target_device_id: "dvc-1".to_string(),
        intent: Intent::Write {
            resource: resource.to_string(),
            payload,
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
        let parsed = rmodbus::parse_ascii_frame(
            &request.frame,
            request.frame.len(),
            &mut frame_buf,
            0,
        )
        .unwrap() as usize;
        &frame_buf[..parsed]
    } else {
        &request.frame
    };

    let mut response = Vec::new();
    let mut frame = ModbusFrame::new(request.unit_id, binary_request, response_proto, &mut response);
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
    let request = ModbusRequest::try_from(ModbusCommandRef {
        device: &driver.dvc,
        command: &command_read("holding_u16"),
    })
    .unwrap();

    assert_eq!(request.proto, ModbusProto::TcpUdp);
    assert_eq!(request.decoder, crate::ModbusResponseDecoder::U16);
    assert!(!request.is_write);
    assert!(!request.frame.is_empty());
}

#[test]
fn decode_tcp_holding_register_response() {
    let driver = make_driver(tcp_endpoint());
    let request = ModbusRequest::try_from(ModbusCommandRef {
        device: &driver.dvc,
        command: &command_read("holding_u16"),
    })
    .unwrap();
    let response = simulate_response(&request, ModbusProto::TcpUdp);
    let payload = decode_modbus_response(&request, &response).unwrap();
    assert_eq!(payload, 0x1234u16.to_be_bytes().to_vec());
}

#[test]
fn decode_rtu_coil_response() {
    let driver = make_driver(rtu_endpoint());
    let request = ModbusRequest::try_from(ModbusCommandRef {
        device: &driver.dvc,
        command: &command_read("coil_bit"),
    })
    .unwrap();
    let response = simulate_response(&request, ModbusProto::Rtu);
    let payload = decode_modbus_response(&request, &response).unwrap();
    assert_eq!(payload, vec![1]);
}

#[test]
fn decode_ascii_string_response() {
    let driver = make_driver(ascii_endpoint());
    let request = ModbusRequest::try_from(ModbusCommandRef {
        device: &driver.dvc,
        command: &command_read("holding_text"),
    })
    .unwrap();
    let response = simulate_response(&request, ModbusProto::Ascii);
    let decoded = decode_ascii_wire_frame(&response).unwrap();
    let payload = decode_modbus_response(&request, &decoded).unwrap();
    assert_eq!(payload, b"hi!".to_vec());
}

#[test]
fn build_write_single_holding_request() {
    let driver = make_driver(tcp_endpoint());
    let request = ModbusRequest::try_from(ModbusCommandRef {
        device: &driver.dvc,
        command: &command_write("holding_u16", 0x4321u16.to_be_bytes().to_vec()),
    })
    .unwrap();

    assert!(request.is_write);
    let response = simulate_response(&request, ModbusProto::TcpUdp);
    let payload = decode_modbus_response(&request, &response).unwrap();
    assert!(payload.is_empty());
}
