use alloc::{format, string::String, vec::Vec};
use core::time::Duration;

use ferredge_core::prelude::*;
use rmodbus::ModbusProto;

use crate::{
    ModbusDriver, ModbusRequest, ModbusResponse,
    StackDatagramSocket, StackSerialPort, StackSocket,
    convert::endpoint_options,
    codec::{build_modbus_response, decode_ascii_wire_frame},
};

impl Lifecycle for ModbusDriver {
    type Error = String;

    async fn start(&self) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn stop(&self) -> Result<(), Self::Error> {
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
        let max_attempts = reconnect.max_attempts.saturating_add(1);
        let mut last_error = None;

        for attempt in 0..max_attempts {
            match self.execute_once(request).await {
                Ok(frame) => return Ok(frame),
                Err(error) => {
                    last_error = Some(error);
                    if attempt + 1 >= max_attempts {
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

    async fn execute_rtu(
        &self,
        request: &ModbusRequest,
        config: &ModbusRtuEndpointConfig,
    ) -> Result<Vec<u8>, String> {
        let mut port = self.open_serial_port(&config.serial, "Modbus RTU").await?;
        write_serial_request(&mut port, &request.frame, "Modbus RTU").await?;
        read_stream_response_serial(&mut port, request.proto).await
    }

    async fn execute_ascii(
        &self,
        request: &ModbusRequest,
        config: &ModbusAsciiEndpointConfig,
    ) -> Result<Vec<u8>, String> {
        let mut port = self.open_serial_port(&config.serial, "Modbus ASCII").await?;
        write_serial_request(&mut port, &request.frame, "Modbus ASCII").await?;
        let raw_ascii = read_stream_response_serial(&mut port, request.proto).await?;
        decode_ascii_wire_frame(&raw_ascii)
    }

    async fn open_tcp_socket(
        &self,
        config: &ModbusTcpEndpointConfig,
        timeout: Option<Duration>,
    ) -> Result<StackSocket, String> {
        let mut socket = self
            .net
            .connect(&format!("{}:{}", config.addr, config.port))
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
