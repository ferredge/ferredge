//! Modbus legs of the harness: RTU against diagslave over the pty-bridged UART1, and
//! Modbus TCP against a second diagslave over the LAN9118.

use alloc::{string::ToString, vec::Vec};
use core::time::Duration;

use ferredge_core::prelude::*;
use ferredge_proto_modbus::{
    ModbusDriver, ModbusParserSeed, ModbusRequest, ModbusResponseDecoder, SerialTransport,
    TcpTransport,
};
use ferredge_runtime_embassy::{EmbassyNet, EmbassyRuntime, EmbassySerial};

use crate::hw::uart::{CmsdkUart, UART1_BASE};

use super::HOST_ADDR;

/// Reads coils from diagslave (`-m rtu -a 1`) over the bridged UART1 with a real,
/// CRC-valid frame; a fresh diagslave reports every coil as off.
pub async fn rtu(runtime: &EmbassyRuntime) {
    let serial: EmbassySerial<CmsdkUart> = EmbassySerial::new();
    serial.register("/dev/uart1", CmsdkUart::new(UART1_BASE));

    let driver = ModbusDriver::with_transport(
        Device {
            id: "harness-modbus".to_string(),
            name: "diagslave".to_string(),
            status: DeviceStatus::Online,
            endpoint: DeviceEndpoint::ModbusRTU(ModbusRtuEndpointConfig {
                serial: SerialPortConfig {
                    path: "/dev/uart1".to_string(),
                    ..SerialPortConfig::default()
                },
                options: ModbusClientOptions::default(),
            }),
            metadata: None,
            max_connections: Some(1),
            resources: Map::default(),
            message_endpoints: Vec::new(),
        },
        runtime.clone(),
        SerialTransport::new(serial),
    );
    driver.start().await.expect("modbus start should succeed");
    log::debug!("querying diagslave over bridged UART1");

    let mut frame = Vec::new();
    let mut generator = rmodbus::client::ModbusRequest::new(1, rmodbus::ModbusProto::Rtu);
    generator
        .generate_get_coils(0, 8, &mut frame)
        .expect("frame generation should succeed");
    log::debug!("diagslave RTU request frame {frame:02x?}");

    let response = driver
        .execute(ModbusRequest {
            resource: "coils".to_string(),
            is_write: false,
            frame,
            proto: rmodbus::ModbusProto::Rtu,
            unit_id: 1,
            parser_seed: ModbusParserSeed::ReadCoils {
                address: 0,
                quantity: 8,
            },
            decoder: ModbusResponseDecoder::Bits { quantity: 8 },
            timeout: Some(Duration::from_secs(5)),
        })
        .await
        .expect("diagslave should answer the read");
    log::debug!("diagslave RTU response frame {:02x?}", response.frame);
    // unit 1, function 1, one data byte, all coils off on a fresh diagslave.
    assert_eq!(&response.frame[..4], &[0x01, 0x01, 0x01, 0x00]);

    driver.stop().await.expect("modbus stop should succeed");
}

/// Reads coils from the host's TCP diagslave (`-m tcp -p 41502`) through the lean
/// [`TcpTransport`]: a Modbus-TCP-only device carries no serial or UDP stack.
pub async fn tcp(runtime: &EmbassyRuntime, net: EmbassyNet) {
    let driver = ModbusDriver::with_transport(
        Device {
            id: "harness-modbus-tcp".to_string(),
            name: "diagslave-tcp".to_string(),
            status: DeviceStatus::Online,
            endpoint: DeviceEndpoint::ModbusTCP(ModbusTcpEndpointConfig {
                addr: HOST_ADDR.to_string(),
                port: 41502,
                options: ModbusClientOptions::default(),
            }),
            metadata: None,
            max_connections: Some(1),
            resources: Map::default(),
            message_endpoints: Vec::new(),
        },
        runtime.clone(),
        TcpTransport::new(net),
    );
    driver
        .start()
        .await
        .expect("modbus TCP start should succeed");
    log::debug!("querying diagslave at {HOST_ADDR}:41502");

    let mut frame = Vec::new();
    let mut generator = rmodbus::client::ModbusRequest::new(1, rmodbus::ModbusProto::TcpUdp);
    generator
        .generate_get_coils(0, 8, &mut frame)
        .expect("frame generation should succeed");
    log::debug!("diagslave TCP request frame {frame:02x?}");

    let response = driver
        .execute(ModbusRequest {
            resource: "coils".to_string(),
            is_write: false,
            frame,
            proto: rmodbus::ModbusProto::TcpUdp,
            unit_id: 1,
            parser_seed: ModbusParserSeed::ReadCoils {
                address: 0,
                quantity: 8,
            },
            decoder: ModbusResponseDecoder::Bits { quantity: 8 },
            timeout: Some(Duration::from_secs(5)),
        })
        .await
        .expect("diagslave should answer the read");
    log::debug!("diagslave TCP response frame {:02x?}", response.frame);
    // Past the 6-byte MBAP prefix: unit 1, function 1, one data byte, all coils
    // off on a fresh diagslave.
    assert_eq!(&response.frame[6..10], &[0x01, 0x01, 0x01, 0x00]);

    driver.stop().await.expect("modbus TCP stop should succeed");
}
