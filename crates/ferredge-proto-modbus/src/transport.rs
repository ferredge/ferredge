use alloc::{format, string::String, vec::Vec};
use core::time::Duration;

use ferredge_core::prelude::*;
use rmodbus::ModbusProto;

use crate::{
    ModbusDriver, ModbusRequest, ModbusResponse,
    StackDatagramSocket, StackSerialPort, StackSocket,
    convert::endpoint_options,
    codec::{build_modbus_response, decode_ascii_wire_frame},
    types::PersistentSession,
};

impl Lifecycle for ModbusDriver {
    type Error = String;

    async fn start(&self) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn stop(&self) -> Result<(), Self::Error> {
        self.close_persistent_session().await?;
        Ok(())
    }
}

impl RequestResponse for ModbusDriver {
    type Request = ModbusRequest;
    type Response = ModbusResponse;
    type Error = String;

    async fn execute(&self, request: Self::Request) -> Result<Self::Response, Self::Error> {
        let raw_frame = self.execute_on_endpoint(&request).await?;
        build_modbus_response(&request, raw_frame)
    }
}

impl ModbusDriver {
    pub(crate) async fn execute_on_endpoint(&self, request: &ModbusRequest) -> Result<Vec<u8>, String> {
        let options = endpoint_options(&self.dvc.endpoint)
            .ok_or_else(|| "missing Modbus endpoint options".to_string())?;
        let reconnect = &options.reconnect;
        let max_attempts = if request.is_write && !reconnect.retry_writes {
            1
        } else {
            reconnect.max_attempts.saturating_add(1)
        };
        let mut last_error = None;

        for attempt in 0..max_attempts {
            match self.execute_once(request).await {
                Ok(frame) => return Ok(frame),
                Err(error) => {
                    let retryable = is_retryable_transport_error(&error);
                    last_error = Some(error);
                    if attempt + 1 >= max_attempts || !retryable {
                        break;
                    }
                    if let Some(delay) = reconnect.delay_for_attempt(attempt + 1) {
                        self.runtime.sleep(delay).await;
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| "Modbus execute failed".to_string()))
    }

    async fn execute_once(&self, request: &ModbusRequest) -> Result<Vec<u8>, String> {
        match &self.dvc.endpoint {
            DeviceEndpoint::ModbusTCP(config) => self.execute_tcp(request, config).await,
            DeviceEndpoint::ModbusRTUOverTCP(config) => self.execute_rtu_over_tcp(request, config).await,
            DeviceEndpoint::ModbusUDP(config) => self.execute_udp(request, config).await,
            DeviceEndpoint::ModbusRTU(config) => self.execute_rtu(request, config).await,
            DeviceEndpoint::ModbusASCII(config) => self.execute_ascii(request, config).await,
            _ => Err("device endpoint is not Modbus".to_string()),
        }
    }

    async fn execute_tcp(
        &self,
        request: &ModbusRequest,
        config: &ModbusTcpEndpointConfig,
    ) -> Result<Vec<u8>, String> {
        if config.options.persistent_session {
            return self.execute_tcp_persistent(request, config).await;
        }
        let mut socket = self.open_tcp_socket(config, request.timeout).await?;
        write_stream_request(&mut socket, &request.frame, "Modbus TCP").await?;
        read_stream_response_socket(&mut socket, request.proto).await
    }

    async fn execute_udp(
        &self,
        request: &ModbusRequest,
        config: &ModbusUdpEndpointConfig,
    ) -> Result<Vec<u8>, String> {
        let mut socket = self
            .net
            .bind_datagram("0.0.0.0:0")
            .await
            .map_err(|e| format!("failed to bind Modbus UDP socket: {e:?}"))?;
        socket
            .send_to(&request.frame, &format!("{}:{}", config.addr, config.port))
            .await
            .map_err(|e| format!("failed to send Modbus UDP request: {e:?}"))?;
        read_datagram_response(&mut socket).await
    }

    async fn execute_rtu_over_tcp(
        &self,
        request: &ModbusRequest,
        config: &ModbusRtuOverTcpEndpointConfig,
    ) -> Result<Vec<u8>, String> {
        if config.options.persistent_session {
            return self.execute_tcp_persistent_on_addr(request, &config.addr, config.port).await;
        }
        let mut socket = self
            .open_tcp_socket_addr(&config.addr, config.port, request.timeout)
            .await?;
        write_stream_request(&mut socket, &request.frame, "Modbus RTU-over-TCP").await?;
        read_stream_response_socket(&mut socket, request.proto).await
    }

    async fn execute_rtu(
        &self,
        request: &ModbusRequest,
        config: &ModbusRtuEndpointConfig,
    ) -> Result<Vec<u8>, String> {
        if config.options.persistent_session {
            return self.execute_serial_persistent(
                request,
                &config.serial,
                "Modbus RTU",
                PersistentSessionKind::Rtu,
            )
            .await;
        }
        let mut port = self.open_serial_port(&config.serial, "Modbus RTU").await?;
        write_serial_request(&mut port, &request.frame, "Modbus RTU").await?;
        read_stream_response_serial(&mut port, request.proto).await
    }

    async fn execute_ascii(
        &self,
        request: &ModbusRequest,
        config: &ModbusAsciiEndpointConfig,
    ) -> Result<Vec<u8>, String> {
        if config.options.persistent_session {
            return self.execute_serial_persistent(
                request,
                &config.serial,
                "Modbus ASCII",
                PersistentSessionKind::Ascii,
            )
            .await;
        }
        let mut port = self.open_serial_port(&config.serial, "Modbus ASCII").await?;
        write_serial_request(&mut port, &request.frame, "Modbus ASCII").await?;
        let raw_ascii = read_stream_response_serial(&mut port, request.proto).await?;
        decode_ascii_wire_frame(&raw_ascii)
    }

    async fn execute_tcp_persistent(
        &self,
        request: &ModbusRequest,
        config: &ModbusTcpEndpointConfig,
    ) -> Result<Vec<u8>, String> {
        self.execute_tcp_persistent_on_addr(request, &config.addr, config.port)
            .await
    }

    async fn execute_tcp_persistent_on_addr(
        &self,
        request: &ModbusRequest,
        addr: &str,
        port: u16,
    ) -> Result<Vec<u8>, String> {
        let mut socket = match self.take_persistent_session().await? {
            Some(PersistentSession::Tcp(socket)) => socket,
            Some(other) => {
                self.close_session(other).await;
                self.open_tcp_socket_addr(addr, port, request.timeout).await?
            }
            None => self.open_tcp_socket_addr(addr, port, request.timeout).await?,
        };

        let result = async {
            write_stream_request(&mut socket, &request.frame, "Modbus TCP").await?;
            read_stream_response_socket(&mut socket, request.proto).await
        }
        .await;

        match result {
            Ok(frame) => {
                self.store_persistent_session(PersistentSession::Tcp(socket)).await?;
                Ok(frame)
            }
            Err(error) => {
                self.close_session(PersistentSession::Tcp(socket)).await;
                Err(error)
            }
        }
    }

    async fn execute_serial_persistent(
        &self,
        request: &ModbusRequest,
        config: &SerialPortConfig,
        label: &str,
        kind: PersistentSessionKind,
    ) -> Result<Vec<u8>, String> {
        let mut port = match self.take_persistent_session().await? {
            Some(PersistentSession::Rtu(port)) if kind == PersistentSessionKind::Rtu => port,
            Some(PersistentSession::Ascii(port)) if kind == PersistentSessionKind::Ascii => port,
            Some(other) => {
                self.close_session(other).await;
                self.open_serial_port(config, label).await?
            }
            None => self.open_serial_port(config, label).await?,
        };

        let result = async {
            write_serial_request(&mut port, &request.frame, label).await?;
            let raw = read_stream_response_serial(&mut port, request.proto).await?;
            if kind == PersistentSessionKind::Ascii {
                decode_ascii_wire_frame(&raw)
            } else {
                Ok(raw)
            }
        }
        .await;

        match result {
            Ok(frame) => {
                let session = match kind {
                    PersistentSessionKind::Rtu => PersistentSession::Rtu(port),
                    PersistentSessionKind::Ascii => PersistentSession::Ascii(port),
                };
                self.store_persistent_session(session).await?;
                Ok(frame)
            }
            Err(error) => {
                let session = match kind {
                    PersistentSessionKind::Rtu => PersistentSession::Rtu(port),
                    PersistentSessionKind::Ascii => PersistentSession::Ascii(port),
                };
                self.close_session(session).await;
                Err(error)
            }
        }
    }

    async fn open_tcp_socket(
        &self,
        config: &ModbusTcpEndpointConfig,
        timeout: Option<Duration>,
    ) -> Result<StackSocket, String> {
        self.open_tcp_socket_addr(&config.addr, config.port, timeout).await
    }

    async fn open_tcp_socket_addr(
        &self,
        addr: &str,
        port: u16,
        timeout: Option<Duration>,
    ) -> Result<StackSocket, String> {
        let mut socket = self
            .net
            .connect(&format!("{addr}:{port}"))
            .await
            .map_err(|e| format!("failed to connect Modbus TCP socket: {e:?}"))?;
        socket
            .set_read_timeout(timeout)
            .map_err(|e| format!("failed to set Modbus TCP read timeout: {e:?}"))?;
        socket
            .set_write_timeout(timeout)
            .map_err(|e| format!("failed to set Modbus TCP write timeout: {e:?}"))?;
        Ok(socket)
    }

    async fn open_serial_port(
        &self,
        config: &SerialPortConfig,
        label: &str,
    ) -> Result<StackSerialPort, String> {
        self.serial
            .open(config)
            .await
            .map_err(|e| format!("failed to open {label} serial port: {e:?}"))
    }

    async fn take_persistent_session(&self) -> Result<Option<PersistentSession>, String> {
        let mut guard = self
            .persistent_session
            .lock()
            .await
            .map_err(|_| "failed to lock Modbus persistent session".to_string())?;
        Ok(guard.take())
    }

    async fn store_persistent_session(&self, session: PersistentSession) -> Result<(), String> {
        let mut guard = self
            .persistent_session
            .lock()
            .await
            .map_err(|_| "failed to lock Modbus persistent session".to_string())?;
        *guard = Some(session);
        Ok(())
    }

    async fn close_persistent_session(&self) -> Result<(), String> {
        if let Some(session) = self.take_persistent_session().await? {
            self.close_session(session).await;
        }
        Ok(())
    }

    async fn close_session(&self, mut session: PersistentSession) {
        match &mut session {
            PersistentSession::Tcp(socket) => {
                let _ = socket.close().await;
            }
            PersistentSession::Rtu(port) | PersistentSession::Ascii(port) => {
                let _ = port.close().await;
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PersistentSessionKind {
    Rtu,
    Ascii,
}

async fn write_stream_request(
    socket: &mut StackSocket,
    frame: &[u8],
    label: &str,
) -> Result<(), String> {
    socket
        .write(frame)
        .await
        .map_err(|e| format!("failed to write {label} request: {e:?}"))?;
    socket
        .flush()
        .await
        .map_err(|e| format!("failed to flush {label} request: {e:?}"))
}

async fn write_serial_request(
    port: &mut StackSerialPort,
    frame: &[u8],
    label: &str,
) -> Result<(), String> {
    port.write(frame)
        .await
        .map_err(|e| format!("failed to write {label} request: {e:?}"))?;
    port.flush()
        .await
        .map_err(|e| format!("failed to flush {label} request: {e:?}"))
}

async fn read_stream_response_socket(
    socket: &mut StackSocket,
    proto: ModbusProto,
) -> Result<Vec<u8>, String> {
    let mut frame = Vec::new();
    let mut buf = [0u8; 64];

    loop {
        let read_count = match socket.read(&mut buf).await {
            Ok(read_count) => read_count,
            Err(NetError::Closed) if !frame.is_empty() => break,
            Err(e) => return Err(format!("failed to read Modbus socket response: {e:?}")),
        };
        if read_count == 0 {
            break;
        }
        frame.extend_from_slice(&buf[..read_count]);
        if frame_complete(&frame, proto) || frame.len() >= 256 {
            break;
        }
    }

    Ok(trim_frame(frame, proto))
}

async fn read_stream_response_serial(
    port: &mut StackSerialPort,
    proto: ModbusProto,
) -> Result<Vec<u8>, String> {
    let mut frame = Vec::new();
    let mut buf = [0u8; 64];

    loop {
        let read_count = match port.read(&mut buf).await {
            Ok(read_count) => read_count,
            Err(SerialError::Closed) if !frame.is_empty() => break,
            Err(e) => return Err(format!("failed to read Modbus serial response: {e:?}")),
        };
        if read_count == 0 {
            break;
        }
        frame.extend_from_slice(&buf[..read_count]);
        if frame_complete(&frame, proto) || frame.len() >= 256 {
            break;
        }
    }

    Ok(trim_frame(frame, proto))
}

async fn read_datagram_response(socket: &mut StackDatagramSocket) -> Result<Vec<u8>, String> {
    let mut buf = [0u8; 256];
    let (size, _) = socket
        .recv_from(&mut buf)
        .await
        .map_err(|e| format!("failed to receive Modbus UDP response: {e:?}"))?;
    Ok(buf[..size].to_vec())
}

fn min_guess_len(proto: ModbusProto) -> u8 {
    match proto {
        ModbusProto::TcpUdp => 6,
        ModbusProto::Rtu => 5,
        ModbusProto::Ascii => 7,
    }
}

fn frame_complete(frame: &[u8], proto: ModbusProto) -> bool {
    frame.len() >= usize::from(min_guess_len(proto))
        && rmodbus::guess_response_frame_len(frame, proto)
            .map(|expected_len| frame.len() >= usize::from(expected_len))
            .unwrap_or(false)
}

fn trim_frame(mut frame: Vec<u8>, proto: ModbusProto) -> Vec<u8> {
    if let Ok(expected_len) = rmodbus::guess_response_frame_len(&frame, proto) {
        frame.truncate(usize::from(expected_len));
    }
    frame
}

fn is_retryable_transport_error(error: &str) -> bool {
    error.starts_with("failed to connect ")
        || error.starts_with("failed to bind ")
        || error.starts_with("failed to open ")
        || error.starts_with("failed to send ")
        || error.starts_with("failed to write ")
        || error.starts_with("failed to flush ")
        || error.starts_with("failed to read ")
}
