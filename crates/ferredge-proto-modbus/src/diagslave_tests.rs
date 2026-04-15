use std::{
    fs,
    net::{TcpListener, TcpStream},
    path::PathBuf,
    process::{Child, Command as ProcessCommand, Stdio},
    sync::OnceLock,
    thread,
    time::{Duration, Instant},
};

use ferredge_core::prelude::*;

use crate::{
    ModbusDriver,
    attributes::{ModbusRegisterKind, ModbusResourceAttributes, ModbusValueCodec},
};

#[cfg(feature = "tokio-runtime")]
use ferredge_runtime_tokio::TokioRuntime as StackRuntime;

#[cfg(feature = "async-std-runtime")]
use ferredge_runtime_async_std::AsyncStdRuntime as StackRuntime;

const DIAGSLAVE_START_TIMEOUT_SECS: u64 = 5;
const DIAGSLAVE_POLL_INTERVAL_MS: u64 = 25;
const DIAGSLAVE_UDP_START_SETTLE_MS: u64 = 300;
const DIAGSLAVE_SERIAL_START_SETTLE_MS: u64 = 300;
const DIAGSLAVE_TCP_START_SETTLE_MS: u64 = 50;
const SOCAT_START_TIMEOUT_SECS: u64 = 5;

#[cfg(unix)]
struct SerialPtyGuard {
    child: Option<Child>,
    master_path: PathBuf,
    slave_path: PathBuf,
}

#[cfg(unix)]
impl SerialPtyGuard {
    fn start() -> Self {
        let nonce = format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time should be after epoch")
                .as_nanos()
        );
        let master_path = std::env::temp_dir().join(format!("ferredge-master-{nonce}.pty"));
        let slave_path = std::env::temp_dir().join(format!("ferredge-slave-{nonce}.pty"));
        let mut guard = Self {
            child: None,
            master_path,
            slave_path,
        };
        guard.start_pair();
        guard
    }

    fn start_pair(&mut self) {
        assert!(self.child.is_none(), "serial pty pair already running");
        let child = ProcessCommand::new("socat")
            .args([
                "-d",
                "-d",
                &format!(
                    "PTY,raw,echo=0,link={},mode=666",
                    self.master_path.display()
                ),
                &format!("PTY,raw,echo=0,link={},mode=666", self.slave_path.display()),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("socat should spawn");
        self.child = Some(child);

        let deadline = Instant::now() + Duration::from_secs(SOCAT_START_TIMEOUT_SECS);
        loop {
            if self.master_path.exists() && self.slave_path.exists() {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "socat should create PTY links before timeout"
            );
            thread::sleep(Duration::from_millis(DIAGSLAVE_POLL_INTERVAL_MS));
        }
    }

    fn master_path(&self) -> String {
        self.master_path.to_string_lossy().into_owned()
    }

    fn slave_path(&self) -> String {
        self.slave_path.to_string_lossy().into_owned()
    }

    fn stop_pair(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = fs::remove_file(&self.master_path);
        let _ = fs::remove_file(&self.slave_path);
    }
}

#[cfg(unix)]
impl Drop for SerialPtyGuard {
    fn drop(&mut self) {
        self.stop_pair();
    }
}

#[cfg(windows)]
struct SerialPtyGuard;

#[cfg(windows)]
impl SerialPtyGuard {
    #[allow(dead_code)]
    fn start() -> Self {
        Self
    }
}

fn block_on<F: core::future::Future>(future: F) -> F::Output {
    static RUNTIME: OnceLock<StackRuntime> = OnceLock::new();
    RUNTIME.get_or_init(StackRuntime::default).block_on(future)
}

fn reserve_free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("free port probe should bind")
        .local_addr()
        .expect("free port probe should have addr")
        .port()
}

struct DiagslaveGuard {
    child: Option<Child>,
    port: u16,
    mode: &'static str,
    serial_port: Option<String>,
}

impl DiagslaveGuard {
    fn start(mode: &'static str) -> Self {
        let port = reserve_free_port();
        let mut guard = Self {
            child: None,
            port,
            mode,
            serial_port: None,
        };
        guard.start_slave();
        guard
    }

    #[cfg(unix)]
    fn start_serial(mode: &'static str, serial_port: &str) -> Self {
        let mut guard = Self {
            child: None,
            port: 0,
            mode,
            serial_port: Some(serial_port.to_string()),
        };
        guard.start_serial_slave(serial_port);
        guard
    }

    fn start_slave(&mut self) {
        assert!(self.child.is_none(), "diagslave already running");
        let child = ProcessCommand::new("diagslave")
            .args(["-m", self.mode, "-a", "1", "-p", &self.port.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("diagslave should spawn");
        self.child = Some(child);

        if self.mode == "udp" {
            thread::sleep(Duration::from_millis(DIAGSLAVE_UDP_START_SETTLE_MS));
            return;
        }

        let deadline = Instant::now() + Duration::from_secs(DIAGSLAVE_START_TIMEOUT_SECS);
        loop {
            let ready = matches!(self.mode, "tcp" | "enc")
                && TcpStream::connect(("127.0.0.1", self.port)).is_ok();
            if ready {
                thread::sleep(Duration::from_millis(DIAGSLAVE_TCP_START_SETTLE_MS));
                break;
            }
            assert!(
                Instant::now() < deadline,
                "diagslave should start before timeout"
            );
            thread::sleep(Duration::from_millis(DIAGSLAVE_POLL_INTERVAL_MS));
        }
    }

    #[cfg(unix)]
    fn start_serial_slave(&mut self, serial_port: &str) {
        assert!(self.child.is_none(), "diagslave already running");
        let child = ProcessCommand::new("diagslave")
            .args([
                "-m",
                self.mode,
                "-a",
                "1",
                "-b",
                "9600",
                "-d",
                "8",
                "-s",
                "1",
                "-p",
                "none",
                serial_port,
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("diagslave serial mode should spawn");
        self.child = Some(child);
        thread::sleep(Duration::from_millis(DIAGSLAVE_SERIAL_START_SETTLE_MS));
    }

    fn stop_slave(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    fn restart(&mut self) {
        self.stop_slave();
        #[cfg(unix)]
        if let Some(serial_port) = self.serial_port.clone() {
            self.start_serial_slave(&serial_port);
            return;
        }
        self.start_slave();
    }
}

impl Drop for DiagslaveGuard {
    fn drop(&mut self) {
        self.stop_slave();
    }
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
        read_timeout: Some(Duration::from_millis(500)),
        write_timeout: Some(Duration::from_millis(500)),
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

fn write_command(resource: &str, payload: Vec<u8>) -> Command {
    Command {
        id: format!("write-{resource}"),
        source_device_id: None,
        target_device_id: "diag-1".to_string(),
        intent: Intent::Write {
            resource: resource.to_string(),
            payload,
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
        },
        correlation: None,
    }
}

#[test]
fn diagslave_tcp_write_then_read_holding() {
    let guard = DiagslaveGuard::start("tcp");
    let driver = make_tcp_driver(guard.port, ModbusClientOptions::default());

    block_on(async {
        driver
            .execute_command(&write_command(
                "holding_u16",
                0x3456u16.to_be_bytes().to_vec(),
            ))
            .await
            .expect("holding write should succeed");
        let response = driver
            .execute_command(&read_command("holding_u16"))
            .await
            .expect("holding read should succeed");
        assert_eq!(response.payload, 0x3456u16.to_be_bytes().to_vec());
    });
}

#[test]
fn diagslave_udp_write_then_read_coil() {
    let guard = DiagslaveGuard::start("udp");
    let driver = make_driver(DeviceEndpoint::modbus_udp(ModbusUdpEndpointConfig {
        addr: "127.0.0.1".to_string(),
        port: guard.port,
        options: ModbusClientOptions::default(),
    }));

    block_on(async {
        driver
            .execute_command(&write_command("coil_bit", vec![1]))
            .await
            .expect("coil write should succeed");
        let response = driver
            .execute_command(&read_command("coil_bit"))
            .await
            .expect("coil read should succeed");
        assert_eq!(response.payload, vec![1]);
    });
}

#[test]
fn diagslave_rtu_over_tcp_write_then_read_holding() {
    let guard = DiagslaveGuard::start("enc");
    let driver = make_rtu_over_tcp_driver(guard.port, ModbusClientOptions::default());

    block_on(async {
        driver
            .execute_command(&write_command(
                "holding_u16",
                0x5678u16.to_be_bytes().to_vec(),
            ))
            .await
            .expect("holding write should succeed");
        let response = driver
            .execute_command(&read_command("holding_u16"))
            .await
            .expect("holding read should succeed");
        assert_eq!(response.payload, 0x5678u16.to_be_bytes().to_vec());
    });
}

#[cfg(unix)]
#[test]
fn diagslave_rtu_over_pty_write_then_read_holding() {
    let pty = SerialPtyGuard::start();
    let _guard = DiagslaveGuard::start_serial("rtu", &pty.slave_path());
    let driver = make_rtu_driver(pty.master_path(), ModbusClientOptions::default());

    block_on(async {
        driver
            .execute_command(&write_command(
                "holding_u16",
                0x4567u16.to_be_bytes().to_vec(),
            ))
            .await
            .expect("holding write should succeed");
        let response = driver
            .execute_command(&read_command("holding_u16"))
            .await
            .expect("holding read should succeed");
        assert_eq!(response.payload, 0x4567u16.to_be_bytes().to_vec());
    });
}

#[cfg(unix)]
#[test]
fn diagslave_ascii_over_pty_write_then_read_coil() {
    let pty = SerialPtyGuard::start();
    let _guard = DiagslaveGuard::start_serial("ascii", &pty.slave_path());
    let driver = make_driver(DeviceEndpoint::modbus_ascii(ModbusAsciiEndpointConfig {
        serial: serial_port_config(pty.master_path()),
        options: ModbusClientOptions::default(),
    }));

    block_on(async {
        driver
            .execute_command(&write_command("coil_bit", vec![1]))
            .await
            .expect("coil write should succeed");
        let response = driver
            .execute_command(&read_command("coil_bit"))
            .await
            .expect("coil read should succeed");
        assert_eq!(response.payload, vec![1]);
    });
}

#[test]
fn diagslave_tcp_non_persistent_recovers_after_restart() {
    let mut guard = DiagslaveGuard::start("tcp");
    let driver = make_tcp_driver(guard.port, modbus_options(false, 0));

    block_on(async {
        let before = driver
            .execute_command(&read_command("holding_u16"))
            .await
            .expect("pre-restart read should succeed");
        assert_eq!(before.payload.len(), 2);
    });

    guard.restart();

    block_on(async {
        let after = driver
            .execute_command(&read_command("holding_u16"))
            .await
            .expect("post-restart read should succeed");
        assert_eq!(after.payload.len(), 2);
    });
}

#[test]
fn diagslave_tcp_persistent_recovers_after_restart() {
    let mut guard = DiagslaveGuard::start("tcp");
    let driver = make_tcp_driver(guard.port, modbus_options(true, 2));

    block_on(async {
        let before = driver
            .execute_command(&read_command("holding_u16"))
            .await
            .expect("pre-restart read should succeed");
        assert_eq!(before.payload.len(), 2);
    });

    guard.restart();

    block_on(async {
        let after = driver
            .execute_command(&read_command("holding_u16"))
            .await
            .expect("persistent reconnect read should succeed");
        assert_eq!(after.payload.len(), 2);
    });
}

#[test]
fn diagslave_rtu_over_tcp_non_persistent_recovers_after_restart() {
    let mut guard = DiagslaveGuard::start("enc");
    let driver = make_rtu_over_tcp_driver(guard.port, modbus_options(false, 0));

    block_on(async {
        let before = driver
            .execute_command(&read_command("holding_u16"))
            .await
            .expect("pre-restart RTU-over-TCP read should succeed");
        assert_eq!(before.payload.len(), 2);
    });

    guard.restart();

    block_on(async {
        let after = driver
            .execute_command(&read_command("holding_u16"))
            .await
            .expect("post-restart RTU-over-TCP read should succeed");
        assert_eq!(after.payload.len(), 2);
    });
}

#[test]
fn diagslave_rtu_over_tcp_persistent_recovers_after_restart() {
    let mut guard = DiagslaveGuard::start("enc");
    let driver = make_rtu_over_tcp_driver(guard.port, modbus_options(true, 2));

    block_on(async {
        let before = driver
            .execute_command(&read_command("holding_u16"))
            .await
            .expect("pre-restart RTU-over-TCP read should succeed");
        assert_eq!(before.payload.len(), 2);
    });

    guard.restart();

    block_on(async {
        let after = driver
            .execute_command(&read_command("holding_u16"))
            .await
            .expect("persistent RTU-over-TCP reconnect read should succeed");
        assert_eq!(after.payload.len(), 2);
    });
}

#[cfg(unix)]
#[test]
fn diagslave_rtu_non_persistent_recovers_after_restart() {
    let pty = SerialPtyGuard::start();
    let mut guard = DiagslaveGuard::start_serial("rtu", &pty.slave_path());
    let driver = make_rtu_driver(pty.master_path(), modbus_options(false, 0));

    block_on(async {
        let before = driver
            .execute_command(&read_command("holding_u16"))
            .await
            .expect("pre-restart RTU read should succeed");
        assert_eq!(before.payload.len(), 2);
    });

    guard.restart();

    block_on(async {
        let after = driver
            .execute_command(&read_command("holding_u16"))
            .await
            .expect("post-restart RTU read should succeed");
        assert_eq!(after.payload.len(), 2);
    });
}

#[cfg(unix)]
#[test]
fn diagslave_rtu_persistent_recovers_after_restart() {
    let pty = SerialPtyGuard::start();
    let mut guard = DiagslaveGuard::start_serial("rtu", &pty.slave_path());
    let driver = make_rtu_driver(pty.master_path(), modbus_options(true, 2));

    block_on(async {
        let before = driver
            .execute_command(&read_command("holding_u16"))
            .await
            .expect("pre-restart RTU read should succeed");
        assert_eq!(before.payload.len(), 2);
    });

    guard.restart();

    block_on(async {
        let after = driver
            .execute_command(&read_command("holding_u16"))
            .await
            .expect("persistent RTU reconnect read should succeed");
        assert_eq!(after.payload.len(), 2);
    });
}

#[cfg(windows)]
#[test]
#[ignore = "requires user-provided paired COM ports on Windows; Unix live serial tests use socat-backed PTYs"]
fn diagslave_rtu_over_pty_write_then_read_holding() {}

#[cfg(windows)]
#[test]
#[ignore = "requires user-provided paired COM ports on Windows; Unix live serial tests use socat-backed PTYs"]
fn diagslave_ascii_over_pty_write_then_read_coil() {}
