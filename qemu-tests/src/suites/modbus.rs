//! Runs the Modbus driver on bare metal through the lean [`SerialTransport`]: an
//! RTU-only device carries no network stack at all. The RTU exchange goes through
//! the real open/write/read transport path: the loopback echoes the request,
//! and the frame is crafted so its own echo parses as a complete RTU response
//! (byte-count field 3 → expected response length 8 == request length).

use alloc::{string::ToString, vec, vec::Vec};

use ferredge_core::prelude::*;
use ferredge_proto_modbus::{
    ModbusDriver, ModbusParserSeed, ModbusRequest, ModbusResponseDecoder, SerialTransport,
};
use ferredge_runtime_embassy::{EmbassyRuntime, EmbassySerial};

use crate::fakes::LoopbackSerial;

pub async fn run(runtime: &EmbassyRuntime) {
    let serial: EmbassySerial<LoopbackSerial> = EmbassySerial::new();
    serial.register("/dev/uart1", LoopbackSerial::default());

    let driver = ModbusDriver::with_transport(
        Device {
            id: "modbus-device-1".to_string(),
            name: "Modbus Device".to_string(),
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

    driver.start().await.expect("start should succeed");
    log::debug!("modbus RTU loopback driver started on /dev/uart1");

    let request = ModbusRequest {
        resource: "coils".to_string(),
        is_write: false,
        frame: vec![0x01, 0x01, 0x03, 0x00, 0x00, 0x18, 0xAA, 0xBB],
        proto: rmodbus::ModbusProto::Rtu,
        unit_id: 1,
        parser_seed: ModbusParserSeed::ReadCoils {
            address: 0x0300,
            quantity: 24,
        },
        decoder: ModbusResponseDecoder::Bits { quantity: 24 },
        timeout: None,
    };
    log::debug!("sending RTU frame {:02x?}", request.frame);
    let response = driver
        .execute(request.clone())
        .await
        .expect("execute should succeed");
    log::debug!("received RTU frame {:02x?}", response.frame);
    assert_eq!(response.frame, request.frame);

    driver.stop().await.expect("stop should succeed");
}
