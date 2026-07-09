use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};
use core::convert::Infallible;
use core::future::Future;
use core::time::Duration;

use ferredge_core::prelude::*;
use rmodbus::ModbusProto;

use crate::{
    ModbusDriver, ModbusRequest, ModbusResponse, StackNet, StackSerial,
    codec::{build_modbus_response, decode_ascii_wire_frame},
    convert::endpoint_options,
};

/// Use this to load the entire datagram into memory before processing,
/// since UDP datagrams are atomic and the Modbus TCP/UDP ADU max size (260 bytes)
/// is small enough to fit comfortably in memory. This also avoids the complexity
/// of incremental parsing and buffering for TCP/UDP, which is complicated by rmodbus's
/// use of u8 for frame length guessing and would require custom handling for valid frames
/// larger than 255 bytes.
const MAX_MODBUS_TCP_UDP_FRAME_LEN: usize = 260;
const MAX_MODBUS_RTU_FRAME_LEN: usize = 256;
const MAX_MODBUS_ASCII_FRAME_LEN: usize = (MAX_MODBUS_RTU_FRAME_LEN * 2) + 3;

/// One Modbus wire transport bound to a device endpoint family.
///
/// A transport owns exactly the stack its endpoints need and defines the persistent state
/// ([`Session`](Self::Session)) it keeps between requests, so a driver instance carries no
/// unused stacks: [`SerialTransport`] needs no network stack at all, [`TcpTransport`] only
/// an [`AsyncNet`], [`UdpTransport`] only an [`AsyncDatagramNet`]. [`StackTransport`]
/// bundles all three and serves every Modbus endpoint — it is the driver default.
pub trait ModbusTransport: Clone + MaybeSend + MaybeSync + 'static {
    /// Persistent connection state kept between requests when the endpoint enables
    /// `persistent_session`. Transports without persistence use [`Infallible`].
    type Session: MaybeSend + 'static;

    /// Executes one raw request/response exchange against the endpoint, reusing or
    /// replenishing `session` as the endpoint's persistence policy dictates.
    fn execute(
        &self,
        endpoint: &DeviceEndpoint,
        request: &ModbusRequest,
        session: &mut Option<Self::Session>,
    ) -> impl Future<Output = Result<Vec<u8>, String>> + MaybeSend;

    /// Closes one persistent session gracefully.
    fn close(&self, session: Self::Session) -> impl Future<Output = ()> + MaybeSend;
}

/// [`ModbusTransport`] for Modbus TCP and RTU-over-TCP endpoints.
#[derive(Clone)]
pub struct TcpTransport<N: AsyncNet = StackNet> {
    net: N,
}

impl<N: AsyncNet> TcpTransport<N> {
    pub fn new(net: N) -> Self {
        Self { net }
    }

    async fn open_socket(
        &self,
        addr: &str,
        port: u16,
        timeout: Option<Duration>,
    ) -> Result<N::Socket, String> {
        let mut socket = self
            .net
            .connect(&format_host_port(addr, port))
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
}

impl<N: AsyncNet> ModbusTransport for TcpTransport<N> {
    type Session = N::Socket;

    async fn execute(
        &self,
        endpoint: &DeviceEndpoint,
        request: &ModbusRequest,
        session: &mut Option<Self::Session>,
    ) -> Result<Vec<u8>, String> {
        let (addr, port, persistent, label) = match endpoint {
            DeviceEndpoint::ModbusTCP(config) => (
                config.addr.as_str(),
                config.port,
                config.options.persistent_session,
                "Modbus TCP",
            ),
            DeviceEndpoint::ModbusRTUOverTCP(config) => (
                config.addr.as_str(),
                config.port,
                config.options.persistent_session,
                "Modbus RTU-over-TCP",
            ),
            _ => return Err("device endpoint is not Modbus TCP".to_string()),
        };

        let mut socket = match (persistent, session.take()) {
            (true, Some(socket)) => socket,
            (_, taken) => {
                // A stale session from a previously-persistent config is closed, not reused.
                if let Some(stale) = taken {
                    self.close(stale).await;
                }
                self.open_socket(addr, port, request.timeout).await?
            }
        };

        let result = async {
            write_stream_request(&mut socket, &request.frame, label).await?;
            read_stream_response_socket(&mut socket, request.proto).await
        }
        .await;

        match result {
            Ok(frame) => {
                if persistent {
                    *session = Some(socket);
                }
                Ok(frame)
            }
            Err(error) => {
                let _ = socket.close().await;
                Err(error)
            }
        }
    }

    async fn close(&self, mut session: Self::Session) {
        let _ = session.close().await;
    }
}

/// [`ModbusTransport`] for Modbus UDP endpoints. Datagram exchanges are connectionless,
/// so this transport keeps no session.
#[derive(Clone)]
pub struct UdpTransport<N: AsyncDatagramNet = StackNet> {
    net: N,
}

impl<N: AsyncDatagramNet> UdpTransport<N> {
    pub fn new(net: N) -> Self {
        Self { net }
    }
}

impl<N: AsyncDatagramNet> ModbusTransport for UdpTransport<N> {
    type Session = Infallible;

    async fn execute(
        &self,
        endpoint: &DeviceEndpoint,
        request: &ModbusRequest,
        _session: &mut Option<Self::Session>,
    ) -> Result<Vec<u8>, String> {
        let DeviceEndpoint::ModbusUDP(config) = endpoint else {
            return Err("device endpoint is not Modbus UDP".to_string());
        };

        let mut socket = self
            .net
            .bind_datagram("0.0.0.0:0")
            .await
            .map_err(|e| format!("failed to bind Modbus UDP socket: {e:?}"))?;
        socket
            .set_read_timeout(request.timeout)
            .map_err(|e| format!("failed to set Modbus UDP read timeout: {e:?}"))?;
        socket
            .send_to(&request.frame, &format_host_port(&config.addr, config.port))
            .await
            .map_err(|e| format!("failed to send Modbus UDP request: {e:?}"))?;
        read_datagram_response(&mut socket).await
    }

    async fn close(&self, session: Self::Session) {
        match session {}
    }
}

/// [`ModbusTransport`] for Modbus RTU and ASCII serial endpoints.
#[derive(Clone)]
pub struct SerialTransport<S: AsyncSerial = StackSerial> {
    serial: S,
}

impl<S: AsyncSerial> SerialTransport<S> {
    pub fn new(serial: S) -> Self {
        Self { serial }
    }

    async fn open_port(&self, config: &SerialPortConfig, label: &str) -> Result<S::Port, String> {
        self.serial
            .open(config)
            .await
            .map_err(|e| format!("failed to open {label} serial port: {e:?}"))
    }
}

impl<S: AsyncSerial> ModbusTransport for SerialTransport<S> {
    type Session = S::Port;

    async fn execute(
        &self,
        endpoint: &DeviceEndpoint,
        request: &ModbusRequest,
        session: &mut Option<Self::Session>,
    ) -> Result<Vec<u8>, String> {
        let (config, persistent, label, is_ascii) = match endpoint {
            DeviceEndpoint::ModbusRTU(config) => (
                &config.serial,
                config.options.persistent_session,
                "Modbus RTU",
                false,
            ),
            DeviceEndpoint::ModbusASCII(config) => (
                &config.serial,
                config.options.persistent_session,
                "Modbus ASCII",
                true,
            ),
            _ => return Err("device endpoint is not Modbus serial".to_string()),
        };

        let mut port = match (persistent, session.take()) {
            (true, Some(port)) => port,
            (_, taken) => {
                if let Some(stale) = taken {
                    self.close(stale).await;
                }
                self.open_port(config, label).await?
            }
        };

        let result = async {
            write_serial_request(&mut port, &request.frame, label).await?;
            let raw = read_stream_response_serial(&mut port, request.proto).await?;
            if is_ascii {
                decode_ascii_wire_frame(&raw)
            } else {
                Ok(raw)
            }
        }
        .await;

        match result {
            Ok(frame) => {
                if persistent {
                    *session = Some(port);
                }
                Ok(frame)
            }
            Err(error) => {
                let _ = port.close().await;
                Err(error)
            }
        }
    }

    async fn close(&self, mut session: Self::Session) {
        let _ = session.close().await;
    }
}

/// Persistent session held by [`StackTransport`]: one live stream socket or serial port.
pub enum StackSession<Sock, Port> {
    Tcp(Sock),
    Serial(Port),
}

/// Full-stack [`ModbusTransport`] serving every Modbus endpoint family.
///
/// This is the driver default and matches the runtime stack's network and serial adapters.
/// It composes the lean transports, so devices bound to a single endpoint family can drop
/// the unused stacks entirely by using one of those directly.
#[derive(Clone)]
pub struct StackTransport<N = StackNet, S = StackSerial>
where
    N: AsyncNet + AsyncDatagramNet,
    S: AsyncSerial,
{
    tcp: TcpTransport<N>,
    udp: UdpTransport<N>,
    serial: SerialTransport<S>,
}

impl<N, S> StackTransport<N, S>
where
    N: AsyncNet + AsyncDatagramNet,
    S: AsyncSerial,
{
    pub fn new(net: N, serial: S) -> Self {
        Self {
            tcp: TcpTransport::new(net.clone()),
            udp: UdpTransport::new(net),
            serial: SerialTransport::new(serial),
        }
    }
}

impl<N, S> ModbusTransport for StackTransport<N, S>
where
    N: AsyncNet + AsyncDatagramNet,
    S: AsyncSerial,
{
    type Session = StackSession<N::Socket, S::Port>;

    async fn execute(
        &self,
        endpoint: &DeviceEndpoint,
        request: &ModbusRequest,
        session: &mut Option<Self::Session>,
    ) -> Result<Vec<u8>, String> {
        match endpoint {
            DeviceEndpoint::ModbusTCP(_) | DeviceEndpoint::ModbusRTUOverTCP(_) => {
                let mut sub = match session.take() {
                    Some(StackSession::Tcp(socket)) => Some(socket),
                    Some(other) => {
                        self.close(other).await;
                        None
                    }
                    None => None,
                };
                let result = self.tcp.execute(endpoint, request, &mut sub).await;
                *session = sub.map(StackSession::Tcp);
                result
            }
            DeviceEndpoint::ModbusUDP(_) => self.udp.execute(endpoint, request, &mut None).await,
            DeviceEndpoint::ModbusRTU(_) | DeviceEndpoint::ModbusASCII(_) => {
                let mut sub = match session.take() {
                    Some(StackSession::Serial(port)) => Some(port),
                    Some(other) => {
                        self.close(other).await;
                        None
                    }
                    None => None,
                };
                let result = self.serial.execute(endpoint, request, &mut sub).await;
                *session = sub.map(StackSession::Serial);
                result
            }
            _ => Err("device endpoint is not Modbus".to_string()),
        }
    }

    async fn close(&self, session: Self::Session) {
        match session {
            StackSession::Tcp(socket) => self.tcp.close(socket).await,
            StackSession::Serial(port) => self.serial.close(port).await,
        }
    }
}

impl<T: ModbusTransport> Lifecycle for ModbusDriver<T> {
    type Error = String;

    async fn start(&self) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn stop(&self) -> Result<(), Self::Error> {
        self.close_persistent_session().await
    }
}

impl<T: ModbusTransport> RequestResponse for ModbusDriver<T> {
    type Request = ModbusRequest;
    type Response = ModbusResponse;
    type Error = String;

    async fn execute(&self, request: Self::Request) -> Result<Self::Response, Self::Error> {
        let raw_frame = self.execute_on_endpoint(&request).await?;
        build_modbus_response(&request, raw_frame)
    }
}

impl<T: ModbusTransport> ModbusDriver<T> {
    pub(crate) async fn execute_on_endpoint(
        &self,
        request: &ModbusRequest,
    ) -> Result<Vec<u8>, String> {
        let options = endpoint_options(&self.dvc.endpoint)
            .ok_or_else(|| "missing Modbus endpoint options".to_string())?;
        let reconnect = &options.reconnect;
        let max_attempts = request_attempt_budget(reconnect, request.is_write);
        let mut last_error = None;

        // The session lock is held across attempts: Modbus endpoints are strictly
        // request-response, so concurrent executes must serialize anyway.
        let mut session = self
            .persistent_session
            .lock()
            .await
            .map_err(|_| "failed to lock Modbus persistent session".to_string())?;

        for attempt in 0..max_attempts {
            match self
                .transport
                .execute(&self.dvc.endpoint, request, &mut *session)
                .await
            {
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

    async fn close_persistent_session(&self) -> Result<(), String> {
        let mut guard = self
            .persistent_session
            .lock()
            .await
            .map_err(|_| "failed to lock Modbus persistent session".to_string())?;
        if let Some(session) = guard.take() {
            self.transport.close(session).await;
        }
        Ok(())
    }
}

async fn write_stream_request<Sock: AsyncSocket>(
    socket: &mut Sock,
    frame: &[u8],
    label: &str,
) -> Result<(), String> {
    write_all_socket(socket, frame)
        .await
        .map_err(|e| format!("failed to write {label} request: {e:?}"))?;
    socket
        .flush()
        .await
        .map_err(|e| format!("failed to flush {label} request: {e:?}"))
}

async fn write_serial_request<P: AsyncSerialPort>(
    port: &mut P,
    frame: &[u8],
    label: &str,
) -> Result<(), String> {
    write_all_serial_port(port, frame)
        .await
        .map_err(|e| format!("failed to write {label} request: {e:?}"))?;
    port.flush()
        .await
        .map_err(|e| format!("failed to flush {label} request: {e:?}"))
}

async fn read_stream_response_socket<Sock: AsyncSocket>(
    socket: &mut Sock,
    proto: ModbusProto,
) -> Result<Vec<u8>, String> {
    let mut frame = Vec::new();
    let mut buf = [0u8; 64];
    let frame_len_limit = max_frame_len(proto);

    loop {
        let read_count = match socket.read(&mut buf).await {
            Ok(read_count) => read_count,
            Err(NetError::Closed) if !frame.is_empty() => break,
            Err(e) => return Err(format!("failed to read Modbus socket response: {e:?}")),
        };
        if read_count == 0 {
            if frame.is_empty() {
                return Err("failed to read Modbus socket response: Closed".to_string());
            }
            break;
        }
        if frame.len().saturating_add(read_count) > frame_len_limit {
            return Err(format!(
                "failed to read Modbus socket response: frame exceeded {frame_len_limit} bytes"
            ));
        }
        frame.extend_from_slice(&buf[..read_count]);
        if frame_complete(&frame, proto) {
            break;
        }
    }

    Ok(trim_frame(frame, proto))
}

async fn read_stream_response_serial<P: AsyncSerialPort>(
    port: &mut P,
    proto: ModbusProto,
) -> Result<Vec<u8>, String> {
    let mut frame = Vec::new();
    let mut buf = [0u8; 64];
    let frame_len_limit = max_frame_len(proto);

    loop {
        let read_count = match port.read(&mut buf).await {
            Ok(read_count) => read_count,
            Err(SerialError::Closed) if !frame.is_empty() => break,
            Err(e) => return Err(format!("failed to read Modbus serial response: {e:?}")),
        };
        if read_count == 0 {
            if frame.is_empty() {
                return Err("failed to read Modbus serial response: Closed".to_string());
            }
            break;
        }
        if frame.len().saturating_add(read_count) > frame_len_limit {
            return Err(format!(
                "failed to read Modbus serial response: frame exceeded {frame_len_limit} bytes"
            ));
        }
        frame.extend_from_slice(&buf[..read_count]);
        if frame_complete(&frame, proto) {
            break;
        }
    }

    Ok(trim_frame(frame, proto))
}

async fn read_datagram_response<D: AsyncDatagramSocket>(socket: &mut D) -> Result<Vec<u8>, String> {
    let mut buf = [0u8; MAX_MODBUS_TCP_UDP_FRAME_LEN];
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

fn max_frame_len(proto: ModbusProto) -> usize {
    match proto {
        ModbusProto::TcpUdp => MAX_MODBUS_TCP_UDP_FRAME_LEN,
        ModbusProto::Rtu => MAX_MODBUS_RTU_FRAME_LEN,
        ModbusProto::Ascii => MAX_MODBUS_ASCII_FRAME_LEN,
    }
}

fn frame_complete(frame: &[u8], proto: ModbusProto) -> bool {
    match proto {
        // Work around rmodbus TCP/UDP length guessing: guess_response_frame_len returns u8 and
        // treats valid MBAP lengths above 255 bytes as FrameBroken. Large coil/register reads can
        // exceed that, so completion has to be driven by the MBAP length field directly.
        ModbusProto::TcpUdp => {
            tcp_udp_frame_len(frame).is_some_and(|expected_len| frame.len() >= expected_len)
        }
        // Work around rmodbus ASCII length guessing: guess_response_frame_len returns u8 and
        // treats valid ASCII wire frames longer than 255 bytes as FrameBroken. Large coil reads
        // can exceed that on the wire, so completion has to be driven by the ASCII terminator.
        ModbusProto::Ascii => ascii_frame_complete(frame),
        _ => {
            frame.len() >= usize::from(min_guess_len(proto))
                && rmodbus::guess_response_frame_len(frame, proto)
                    .map(|expected_len| frame.len() >= usize::from(expected_len))
                    .unwrap_or(false)
        }
    }
}

fn trim_frame(mut frame: Vec<u8>, proto: ModbusProto) -> Vec<u8> {
    if frame.is_empty() {
        return frame;
    }
    if proto == ModbusProto::TcpUdp {
        if let Some(frame_end) = tcp_udp_frame_len(&frame) {
            frame.truncate(frame_end);
        }
        return frame;
    }
    if proto == ModbusProto::Ascii {
        // Keep only the first complete ASCII frame. This mirrors the terminator-based completion
        // above and avoids depending on rmodbus's u8-bounded wire-length helper for ASCII.
        if let Some(frame_end) = ascii_frame_end(&frame) {
            frame.truncate(frame_end);
        }
        return frame;
    }
    if let Ok(expected_len) = rmodbus::guess_response_frame_len(&frame, proto) {
        frame.truncate(usize::from(expected_len));
    }
    frame
}

fn ascii_frame_complete(frame: &[u8]) -> bool {
    frame.starts_with(b":")
        && ascii_frame_end(frame)
            .is_some_and(|frame_end| frame_end >= usize::from(min_guess_len(ModbusProto::Ascii)))
}

fn tcp_udp_frame_len(frame: &[u8]) -> Option<usize> {
    if frame.len() < usize::from(min_guess_len(ModbusProto::TcpUdp)) {
        return None;
    }

    let protocol_id = u16::from_be_bytes([frame[2], frame[3]]);
    if protocol_id != 0 {
        return None;
    }

    Some(usize::from(u16::from_be_bytes([frame[4], frame[5]])) + 6)
}

fn ascii_frame_end(frame: &[u8]) -> Option<usize> {
    frame
        .windows(2)
        .position(|window| window == b"\r\n")
        .map(|idx| idx + 2)
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

fn request_attempt_budget(reconnect: &ModbusReconnectConfig, is_write: bool) -> u32 {
    if !reconnect.enabled || (is_write && !reconnect.retry_writes) {
        1
    } else {
        reconnect.max_attempts.saturating_add(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_completion_handles_frames_larger_than_u8_wire_len_limit() {
        let mut binary = vec![1, 1, 125];
        binary.extend(core::iter::repeat_n(0xAA, 125));

        let mut ascii = Vec::new();
        rmodbus::generate_ascii_frame(&binary, &mut ascii).expect("ascii frame should generate");

        assert!(ascii.len() > usize::from(u8::MAX));
        assert!(rmodbus::guess_response_frame_len(&ascii, ModbusProto::Ascii).is_err());
        assert!(frame_complete(&ascii, ModbusProto::Ascii));
        assert_eq!(trim_frame(ascii.clone(), ModbusProto::Ascii), ascii);
    }

    #[test]
    fn tcp_completion_handles_frames_larger_than_u8_wire_len_limit() {
        let mut tcp = vec![0, 1, 0, 0, 0, 253, 1, 1, 250];
        tcp.extend(core::iter::repeat_n(0xAA, 250));

        assert!(tcp.len() > usize::from(u8::MAX));
        assert!(rmodbus::guess_response_frame_len(&tcp, ModbusProto::TcpUdp).is_err());
        assert!(frame_complete(&tcp, ModbusProto::TcpUdp));
        assert_eq!(trim_frame(tcp.clone(), ModbusProto::TcpUdp), tcp);
    }

    #[test]
    fn disabled_reconnect_forces_single_attempt() {
        let reconnect = ModbusReconnectConfig {
            enabled: false,
            max_attempts: 3,
            retry_writes: true,
            ..ModbusReconnectConfig::default()
        };

        assert_eq!(request_attempt_budget(&reconnect, false), 1);
        assert_eq!(request_attempt_budget(&reconnect, true), 1);
    }

    #[test]
    fn write_retries_respect_retry_writes_flag() {
        let reconnect = ModbusReconnectConfig {
            enabled: true,
            max_attempts: 3,
            retry_writes: false,
            ..ModbusReconnectConfig::default()
        };

        assert_eq!(request_attempt_budget(&reconnect, true), 1);
        assert_eq!(request_attempt_budget(&reconnect, false), 4);
    }

    #[test]
    fn max_frame_len_matches_protocol_limits() {
        assert_eq!(max_frame_len(ModbusProto::TcpUdp), 260);
        assert_eq!(max_frame_len(ModbusProto::Rtu), 256);
        assert_eq!(max_frame_len(ModbusProto::Ascii), 515);
    }
}
