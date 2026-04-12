#[cfg(feature = "std")]
use std::{
    io::{Read, Write},
    net::TcpStream,
};

use crate::HttpRequest;

/// Sends one native HTTP request to endpoint and returns raw response bytes.
#[cfg(feature = "std")]
pub fn send_request(endpoint: &str, request: &HttpRequest) -> Result<Vec<u8>, anyhow::Error> {
    let mut wire_request = format!(
        "{} {} HTTP/1.1\r\nHost: {}\r\n",
        request.method, request.path, endpoint
    );

    for (key, value) in &request.headers {
        wire_request.push_str(&format!("{key}: {value}\r\n"));
    }

    if let Some(body) = &request.body {
        wire_request.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }

    wire_request.push_str("Connection: close\r\n\r\n");

    let mut upstream = TcpStream::connect(endpoint)
        .map_err(|e| anyhow::anyhow!("Failed to connect to endpoint: {e}"))?;

    upstream
        .write_all(wire_request.as_bytes())
        .map_err(|e| anyhow::anyhow!("Failed to send request: {e}"))?;

    if let Some(body) = &request.body {
        upstream
            .write_all(body)
            .map_err(|e| anyhow::anyhow!("Failed to send request body: {e}"))?;
    }

    let mut response = Vec::new();
    upstream
        .read_to_end(&mut response)
        .map_err(|e| anyhow::anyhow!("Failed to read response: {e}"))?;

    Ok(response)
}

/// Placeholder no_std handler until HTTP transport support is introduced there.
#[cfg(not(feature = "std"))]
pub fn send_request(_endpoint: &str, _request: &HttpRequest) -> Result<Vec<u8>, anyhow::Error> {
    Err(anyhow::anyhow!(
        "HTTP driver not implemented for no_std environment"
    ))
}
