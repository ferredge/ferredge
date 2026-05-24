use std::{process::Command as ProcessCommand, time::Duration};

use ferredge_test_support::{
    diagslave::DiagslaveGuard, process::require_command, runtime::block_on, serial::SerialPtyGuard,
};

use ferredge_core::prelude::*;

use crate::{
    ModbusDriver,
    attributes::{ModbusRegisterKind, ModbusResourceAttributes, ModbusValueCodec},
};
const MAX_COIL_START_ADDR: u16 = 4013;
const MAX_READ_COIL_COUNT: u16 = 2000;
const MAX_WRITE_COIL_COUNT: usize = 1968;
const OFFSET_COIL_START_ADDR: u16 = 137;
const OFFSET_READ_COIL_COUNT: u16 = 37;
const OFFSET_WRITE_COIL_COUNT: usize = 29;

enum ModpollEndpoint {
    Tcp { mode: &'static str, port: u16 },
    Serial { mode: &'static str, path: String },
}

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
        "coil_max_read".to_string(),
        DeviceResource {
            name: "coil_max_read".to_string(),
            resource_attributes: ModbusResourceAttributes {
                address: MAX_COIL_START_ADDR,
                register_kind: ModbusRegisterKind::Coil,
                codec: ModbusValueCodec::Bits,
                quantity: Some(MAX_READ_COIL_COUNT),
                description: None,
            },
            unit: None,
            permission: Some(
                DeviceResourceAccessPermission::READ | DeviceResourceAccessPermission::WRITE,
            ),
        },
    );
    resources.insert(
        "coil_max_write".to_string(),
        DeviceResource {
            name: "coil_max_write".to_string(),
            resource_attributes: ModbusResourceAttributes {
                address: MAX_COIL_START_ADDR,
                register_kind: ModbusRegisterKind::Coil,
                codec: ModbusValueCodec::Bits,
                quantity: Some(MAX_WRITE_COIL_COUNT as u16),
                description: None,
            },
            unit: None,
            permission: Some(
                DeviceResourceAccessPermission::READ | DeviceResourceAccessPermission::WRITE,
            ),
        },
    );
    resources.insert(
        "coil_offset_read".to_string(),
        DeviceResource {
            name: "coil_offset_read".to_string(),
            resource_attributes: ModbusResourceAttributes {
                address: OFFSET_COIL_START_ADDR,
                register_kind: ModbusRegisterKind::Coil,
                codec: ModbusValueCodec::Bits,
                quantity: Some(OFFSET_READ_COIL_COUNT),
                description: None,
            },
            unit: None,
            permission: Some(
                DeviceResourceAccessPermission::READ | DeviceResourceAccessPermission::WRITE,
            ),
        },
    );
    resources.insert(
        "coil_offset_write".to_string(),
        DeviceResource {
            name: "coil_offset_write".to_string(),
            resource_attributes: ModbusResourceAttributes {
                address: OFFSET_COIL_START_ADDR,
                register_kind: ModbusRegisterKind::Coil,
                codec: ModbusValueCodec::Bits,
                quantity: Some(OFFSET_WRITE_COIL_COUNT as u16),
                description: None,
            },
            unit: None,
            permission: Some(
                DeviceResourceAccessPermission::READ | DeviceResourceAccessPermission::WRITE,
            ),
        },
    );

    ModbusDriver::new(Device {
        id: "diag-1".to_string(),
        name: "diagslave".to_string(),
        status: DeviceStatus::Online,
        endpoint,
        metadata: None,
        max_connections: None,
        resources,
        message_endpoints: Vec::new(),
    })
}

fn serial_port_config(path: String) -> SerialPortConfig {
    SerialPortConfig {
        path,
        baudrate: 9600,
        data_bits: SerialDataBits::Eight,
        parity: SerialParity::None,
        stop_bits: SerialStopBits::One,
        flow_control: SerialFlowControl::None,
        read_timeout: Some(Duration::from_secs(2)),
        write_timeout: Some(Duration::from_secs(2)),
    }
}

fn modbus_options(persistent_session: bool, max_attempts: u32) -> ModbusClientOptions {
    ModbusClientOptions {
        persistent_session,
        reconnect: ModbusReconnectConfig {
            enabled: true,
            initial_delay: Some(Duration::from_millis(50)),
            max_delay: Some(Duration::from_millis(100)),
            strategy: BackoffStrategy::Fixed,
            multiplier: 1,
            max_attempts,
            retry_writes: false,
        },
        ..ModbusClientOptions::default()
    }
}

fn make_tcp_driver(port: u16, options: ModbusClientOptions) -> ModbusDriver {
    make_driver(DeviceEndpoint::modbus_tcp(ModbusTcpEndpointConfig {
        addr: "127.0.0.1".to_string(),
        port,
        options,
    }))
}

fn make_rtu_over_tcp_driver(port: u16, options: ModbusClientOptions) -> ModbusDriver {
    make_driver(DeviceEndpoint::modbus_rtu_over_tcp(
        ModbusRtuOverTcpEndpointConfig {
            addr: "127.0.0.1".to_string(),
            port,
            options,
        },
    ))
}

#[cfg(unix)]
fn make_rtu_driver(path: String, options: ModbusClientOptions) -> ModbusDriver {
    make_driver(DeviceEndpoint::modbus_rtu(ModbusRtuEndpointConfig {
        serial: serial_port_config(path),
        options,
    }))
}

fn write_command(resource: &str, payload: PayloadValue<'static>) -> Command {
    Command {
        id: format!("write-{resource}"),
        source_device_id: None,
        target_device_id: "diag-1".to_string(),
        intent: Intent::Write {
            resource: resource.to_string(),
            payload,
            options: RequestOptions::default(),
        },
        correlation: None,
    }
}

fn read_command(resource: &str) -> Command {
    Command {
        id: format!("read-{resource}"),
        source_device_id: None,
        target_device_id: "diag-1".to_string(),
        intent: Intent::Read {
            resource: resource.to_string(),
            options: RequestOptions::default(),
        },
        correlation: None,
    }
}

fn coil_payload(count: usize) -> Vec<bool> {
    (0..count).map(|i| (i * 7) % 11 < 5).collect()
}

fn expected_coil_read_payload(write_count: usize, read_count: u16) -> Vec<bool> {
    let mut payload = coil_payload(write_count);
    payload.resize(read_count as usize, false);
    payload
}

fn payload_value_from_coils(coils: &[bool]) -> PayloadValue<'static> {
    PayloadValue::List(
        coils
            .iter()
            .copied()
            .map(PayloadValue::Bool)
            .collect::<Vec<_>>()
            .into(),
    )
}

async fn assert_driver_coil_round_trip(
    driver: &ModbusDriver,
    write_resource: &str,
    read_resource: &str,
    write_count: usize,
    read_count: u16,
) -> Vec<bool> {
    let expected = expected_coil_read_payload(write_count, read_count);
    driver
        .execute_command(write_command(
            write_resource,
            payload_value_from_coils(&coil_payload(write_count)),
        ))
        .await
        .expect("coil write should succeed");
    let response = driver
        .execute_command(read_command(read_resource))
        .await
        .expect("coil read should succeed");
    assert_eq!(
        response.payload().unwrap().into_owned(),
        payload_value_from_coils(&expected)
    );
    expected
}

fn assert_modpoll_coil_read(
    endpoint: &ModpollEndpoint,
    start_addr: u16,
    read_count: u16,
    expected: &[bool],
) {
    require_command("modpoll");
    let mut command = ProcessCommand::new("modpoll");
    command.args([
        "-1",
        "-a",
        "1",
        "-0",
        "-r",
        &start_addr.to_string(),
        "-c",
        &read_count.to_string(),
        "-t",
        "0",
        "-o",
        "2",
    ]);
    match endpoint {
        ModpollEndpoint::Tcp { mode, port } => {
            command.args(["-m", mode, "-p", &port.to_string(), "127.0.0.1"]);
        }
        ModpollEndpoint::Serial { mode, path } => {
            command.args([
                "-m", mode, "-b", "9600", "-d", "8", "-s", "1", "-p", "none", path,
            ]);
        }
    }

    let output = command
        .output()
        .expect("modpoll should run for diagslave validation");
    if !output.status.success() {
        panic!(
            "modpoll read should succeed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }

    let actual = parse_modpoll_coils(&output.stdout);
    assert_eq!(actual, expected);
}

fn parse_modpoll_coils(stdout: &[u8]) -> Vec<bool> {
    String::from_utf8_lossy(stdout)
        .lines()
        .filter_map(|line| {
            if !line.starts_with('[') {
                return None;
            }
            let (_, value) = line.rsplit_once(':')?;
            Some(
                value
                    .trim()
                    .parse::<u8>()
                    .expect("modpoll coil output should parse")
                    != 0,
            )
        })
        .collect()
}

#[test]
fn diagslave_tcp_write_then_read_max_coils() {
    let guard = DiagslaveGuard::start("tcp");
    let driver = make_driver(DeviceEndpoint::modbus_tcp(ModbusTcpEndpointConfig {
        addr: "127.0.0.1".to_string(),
        port: guard.port(),
        options: ModbusClientOptions::default(),
    }));
    let modpoll = ModpollEndpoint::Tcp {
        mode: "tcp",
        port: guard.port(),
    };

    let expected = block_on(async {
        assert_driver_coil_round_trip(
            &driver,
            "coil_max_write",
            "coil_max_read",
            MAX_WRITE_COIL_COUNT,
            MAX_READ_COIL_COUNT,
        )
        .await
    });
    assert_modpoll_coil_read(
        &modpoll,
        MAX_COIL_START_ADDR,
        MAX_READ_COIL_COUNT,
        &expected,
    );
}

#[test]
fn native_diagslave_tcp_execute_command_roundtrip_holding_register() {
    let guard = DiagslaveGuard::start("tcp");
    let driver = make_tcp_driver(guard.port(), ModbusClientOptions::default());

    block_on(driver.execute_command(write_command("holding_u16", PayloadValue::U64(0x4321))))
        .expect("native holding write should succeed");
    let response = block_on(driver.execute_command(read_command("holding_u16")))
        .expect("native holding read should succeed");

    assert_eq!(response.into_payload().unwrap(), PayloadValue::U64(0x4321));
}

#[test]
fn diagslave_udp_write_then_read_max_coils() {
    let guard = DiagslaveGuard::start("udp");
    let driver = make_driver(DeviceEndpoint::modbus_udp(ModbusUdpEndpointConfig {
        addr: "127.0.0.1".to_string(),
        port: guard.port(),
        options: ModbusClientOptions::default(),
    }));
    let modpoll = ModpollEndpoint::Tcp {
        mode: "udp",
        port: guard.port(),
    };

    let expected = block_on(async {
        assert_driver_coil_round_trip(
            &driver,
            "coil_max_write",
            "coil_max_read",
            MAX_WRITE_COIL_COUNT,
            MAX_READ_COIL_COUNT,
        )
        .await
    });
    assert_modpoll_coil_read(
        &modpoll,
        MAX_COIL_START_ADDR,
        MAX_READ_COIL_COUNT,
        &expected,
    );
}

#[test]
fn diagslave_rtu_over_tcp_write_then_read_max_coils() {
    let guard = DiagslaveGuard::start("enc");
    let driver = make_driver(DeviceEndpoint::modbus_rtu_over_tcp(
        ModbusRtuOverTcpEndpointConfig {
            addr: "127.0.0.1".to_string(),
            port: guard.port(),
            options: ModbusClientOptions::default(),
        },
    ));
    let modpoll = ModpollEndpoint::Tcp {
        mode: "enc",
        port: guard.port(),
    };

    let expected = block_on(async {
        assert_driver_coil_round_trip(
            &driver,
            "coil_max_write",
            "coil_max_read",
            MAX_WRITE_COIL_COUNT,
            MAX_READ_COIL_COUNT,
        )
        .await
    });
    assert_modpoll_coil_read(
        &modpoll,
        MAX_COIL_START_ADDR,
        MAX_READ_COIL_COUNT,
        &expected,
    );
}

#[cfg(unix)]
#[test]
fn diagslave_rtu_over_pty_write_then_read_max_coils() {
    let pty = SerialPtyGuard::start();
    let _guard = DiagslaveGuard::start_serial("rtu", &pty.slave_path());
    let master_path = pty.master_path();
    let driver = make_driver(DeviceEndpoint::modbus_rtu(ModbusRtuEndpointConfig {
        serial: serial_port_config(master_path.clone()),
        options: ModbusClientOptions::default(),
    }));
    let modpoll = ModpollEndpoint::Serial {
        mode: "rtu",
        path: master_path,
    };

    let expected = block_on(async {
        assert_driver_coil_round_trip(
            &driver,
            "coil_max_write",
            "coil_max_read",
            MAX_WRITE_COIL_COUNT,
            MAX_READ_COIL_COUNT,
        )
        .await
    });
    assert_modpoll_coil_read(
        &modpoll,
        MAX_COIL_START_ADDR,
        MAX_READ_COIL_COUNT,
        &expected,
    );
}

#[cfg(unix)]
#[test]
fn diagslave_ascii_over_pty_write_then_read_max_coils() {
    let pty = SerialPtyGuard::start();
    let _guard = DiagslaveGuard::start_serial("ascii", &pty.slave_path());
    let master_path = pty.master_path();
    // Large ASCII PTY exchanges can briefly leave the socat-backed master unavailable on CI.
    // Keep the test non-persistent, but allow a few bounded reopen retries for the follow-up read.
    let driver = make_driver(DeviceEndpoint::modbus_ascii(ModbusAsciiEndpointConfig {
        serial: serial_port_config(master_path.clone()),
        options: modbus_options(false, 3),
    }));
    let modpoll = ModpollEndpoint::Serial {
        mode: "ascii",
        path: master_path,
    };

    let expected = block_on(async {
        assert_driver_coil_round_trip(
            &driver,
            "coil_max_write",
            "coil_max_read",
            MAX_WRITE_COIL_COUNT,
            MAX_READ_COIL_COUNT,
        )
        .await
    });
    assert_modpoll_coil_read(
        &modpoll,
        MAX_COIL_START_ADDR,
        MAX_READ_COIL_COUNT,
        &expected,
    );
}

#[test]
fn diagslave_tcp_write_then_read_offset_coils() {
    let guard = DiagslaveGuard::start("tcp");
    let driver = make_driver(DeviceEndpoint::modbus_tcp(ModbusTcpEndpointConfig {
        addr: "127.0.0.1".to_string(),
        port: guard.port(),
        options: ModbusClientOptions::default(),
    }));
    let modpoll = ModpollEndpoint::Tcp {
        mode: "tcp",
        port: guard.port(),
    };

    let expected = block_on(async {
        assert_driver_coil_round_trip(
            &driver,
            "coil_offset_write",
            "coil_offset_read",
            OFFSET_WRITE_COIL_COUNT,
            OFFSET_READ_COIL_COUNT,
        )
        .await
    });
    assert_modpoll_coil_read(
        &modpoll,
        OFFSET_COIL_START_ADDR,
        OFFSET_READ_COIL_COUNT,
        &expected,
    );
}

#[test]
fn diagslave_udp_write_then_read_offset_coils() {
    let guard = DiagslaveGuard::start("udp");
    let driver = make_driver(DeviceEndpoint::modbus_udp(ModbusUdpEndpointConfig {
        addr: "127.0.0.1".to_string(),
        port: guard.port(),
        options: ModbusClientOptions::default(),
    }));
    let modpoll = ModpollEndpoint::Tcp {
        mode: "udp",
        port: guard.port(),
    };

    let expected = block_on(async {
        assert_driver_coil_round_trip(
            &driver,
            "coil_offset_write",
            "coil_offset_read",
            OFFSET_WRITE_COIL_COUNT,
            OFFSET_READ_COIL_COUNT,
        )
        .await
    });
    assert_modpoll_coil_read(
        &modpoll,
        OFFSET_COIL_START_ADDR,
        OFFSET_READ_COIL_COUNT,
        &expected,
    );
}

#[test]
fn diagslave_rtu_over_tcp_write_then_read_offset_coils() {
    let guard = DiagslaveGuard::start("enc");
    let driver = make_driver(DeviceEndpoint::modbus_rtu_over_tcp(
        ModbusRtuOverTcpEndpointConfig {
            addr: "127.0.0.1".to_string(),
            port: guard.port(),
            options: ModbusClientOptions::default(),
        },
    ));
    let modpoll = ModpollEndpoint::Tcp {
        mode: "enc",
        port: guard.port(),
    };

    let expected = block_on(async {
        assert_driver_coil_round_trip(
            &driver,
            "coil_offset_write",
            "coil_offset_read",
            OFFSET_WRITE_COIL_COUNT,
            OFFSET_READ_COIL_COUNT,
        )
        .await
    });
    assert_modpoll_coil_read(
        &modpoll,
        OFFSET_COIL_START_ADDR,
        OFFSET_READ_COIL_COUNT,
        &expected,
    );
}

#[cfg(unix)]
#[test]
fn diagslave_rtu_over_pty_write_then_read_offset_coils() {
    let pty = SerialPtyGuard::start();
    let _guard = DiagslaveGuard::start_serial("rtu", &pty.slave_path());
    let master_path = pty.master_path();
    let driver = make_driver(DeviceEndpoint::modbus_rtu(ModbusRtuEndpointConfig {
        serial: serial_port_config(master_path.clone()),
        options: ModbusClientOptions::default(),
    }));
    let modpoll = ModpollEndpoint::Serial {
        mode: "rtu",
        path: master_path,
    };

    let expected = block_on(async {
        assert_driver_coil_round_trip(
            &driver,
            "coil_offset_write",
            "coil_offset_read",
            OFFSET_WRITE_COIL_COUNT,
            OFFSET_READ_COIL_COUNT,
        )
        .await
    });
    assert_modpoll_coil_read(
        &modpoll,
        OFFSET_COIL_START_ADDR,
        OFFSET_READ_COIL_COUNT,
        &expected,
    );
}

#[cfg(unix)]
#[test]
fn diagslave_ascii_over_pty_write_then_read_offset_coils() {
    let pty = SerialPtyGuard::start();
    let _guard = DiagslaveGuard::start_serial("ascii", &pty.slave_path());
    let master_path = pty.master_path();
    let driver = make_driver(DeviceEndpoint::modbus_ascii(ModbusAsciiEndpointConfig {
        serial: serial_port_config(master_path.clone()),
        options: ModbusClientOptions::default(),
    }));
    let modpoll = ModpollEndpoint::Serial {
        mode: "ascii",
        path: master_path,
    };

    let expected = block_on(async {
        assert_driver_coil_round_trip(
            &driver,
            "coil_offset_write",
            "coil_offset_read",
            OFFSET_WRITE_COIL_COUNT,
            OFFSET_READ_COIL_COUNT,
        )
        .await
    });
    assert_modpoll_coil_read(
        &modpoll,
        OFFSET_COIL_START_ADDR,
        OFFSET_READ_COIL_COUNT,
        &expected,
    );
}

#[test]
fn diagslave_tcp_non_persistent_recovers_after_restart() {
    let mut guard = DiagslaveGuard::start("tcp");
    let driver = make_tcp_driver(guard.port(), modbus_options(false, 0));

    block_on(async {
        driver
            .execute_command(read_command("holding_u16"))
            .await
            .expect("pre-restart read should succeed");
    });

    guard.restart();

    block_on(async {
        driver
            .execute_command(read_command("holding_u16"))
            .await
            .expect("post-restart read should succeed");
    });
}

#[test]
fn diagslave_tcp_persistent_recovers_after_restart() {
    let mut guard = DiagslaveGuard::start("tcp");
    let driver = make_tcp_driver(guard.port(), modbus_options(true, 2));

    block_on(async {
        driver
            .execute_command(read_command("holding_u16"))
            .await
            .expect("pre-restart read should succeed");
    });

    guard.restart();

    block_on(async {
        driver
            .execute_command(read_command("holding_u16"))
            .await
            .expect("persistent reconnect read should succeed");
    });
}

#[test]
fn diagslave_rtu_over_tcp_non_persistent_recovers_after_restart() {
    let mut guard = DiagslaveGuard::start("enc");
    let driver = make_rtu_over_tcp_driver(guard.port(), modbus_options(false, 0));

    block_on(async {
        driver
            .execute_command(read_command("holding_u16"))
            .await
            .expect("pre-restart RTU-over-TCP read should succeed");
    });

    guard.restart();

    block_on(async {
        driver
            .execute_command(read_command("holding_u16"))
            .await
            .expect("post-restart RTU-over-TCP read should succeed");
    });
}

#[test]
fn diagslave_rtu_over_tcp_persistent_recovers_after_restart() {
    let mut guard = DiagslaveGuard::start("enc");
    let driver = make_rtu_over_tcp_driver(guard.port(), modbus_options(true, 2));

    block_on(async {
        driver
            .execute_command(read_command("holding_u16"))
            .await
            .expect("pre-restart RTU-over-TCP read should succeed");
    });

    guard.restart();

    block_on(async {
        driver
            .execute_command(read_command("holding_u16"))
            .await
            .expect("persistent RTU-over-TCP reconnect read should succeed");
    });
}

#[cfg(unix)]
#[test]
fn diagslave_rtu_non_persistent_recovers_after_restart() {
    let pty = SerialPtyGuard::start();
    let mut guard = DiagslaveGuard::start_serial("rtu", &pty.slave_path());
    let driver = make_rtu_driver(pty.master_path(), modbus_options(false, 0));

    block_on(async {
        driver
            .execute_command(read_command("holding_u16"))
            .await
            .expect("pre-restart RTU read should succeed");
    });

    guard.restart();

    block_on(async {
        driver
            .execute_command(read_command("holding_u16"))
            .await
            .expect("post-restart RTU read should succeed");
    });
}

#[cfg(unix)]
#[test]
fn diagslave_rtu_persistent_recovers_after_restart() {
    let pty = SerialPtyGuard::start();
    let mut guard = DiagslaveGuard::start_serial("rtu", &pty.slave_path());
    let driver = make_rtu_driver(pty.master_path(), modbus_options(true, 2));

    block_on(async {
        driver
            .execute_command(read_command("holding_u16"))
            .await
            .expect("pre-restart RTU read should succeed");
    });

    guard.restart();

    block_on(async {
        driver
            .execute_command(read_command("holding_u16"))
            .await
            .expect("persistent RTU reconnect read should succeed");
    });
}

#[cfg(windows)]
#[test]
#[ignore = "requires user-provided paired COM ports on Windows; Unix live serial tests use socat-backed PTYs"]
fn diagslave_rtu_over_pty_write_then_read_max_coils() {}

#[cfg(windows)]
#[test]
#[ignore = "requires user-provided paired COM ports on Windows; Unix live serial tests use socat-backed PTYs"]
fn diagslave_ascii_over_pty_write_then_read_max_coils() {}
