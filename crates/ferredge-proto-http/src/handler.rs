extern crate alloc;

use alloc::vec::Vec;
use alloc::format;
use ferredge_core::prelude::{AsyncNet, AsyncRuntime, AsyncSocket, NetError};

use crate::HttpRequest;

/// Sends one native HTTP request to endpoint and returns raw response bytes.
pub async fn send_request<N, R>(
    endpoint: &str,
    request: &HttpRequest,
    _runtime: &R,
    net: &N,
) -> Result<Vec<u8>, anyhow::Error>
where
    N: AsyncNet,
    R: AsyncRuntime,
{
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

    let mut upstream = net
        .connect(endpoint)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to connect to endpoint: {e:?}"))?;

    write_all_socket(&mut upstream, wire_request.as_bytes())
        .await
        .map_err(|e| anyhow::anyhow!("Failed to send request: {e:?}"))?;

    if let Some(body) = &request.body {
        write_all_socket(&mut upstream, body)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to send request body: {e:?}"))?;
    }

    upstream
        .flush()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to flush request: {e:?}"))?;

    let mut response = Vec::new();
    let mut buffer = [0u8; 4096];
    loop {
        let count = upstream
            .read(&mut buffer)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to read response: {e:?}"))?;
        if count == 0 {
            break;
        }
        response.extend_from_slice(&buffer[..count]);
    }

    Ok(response)
}

async fn write_all_socket<S: AsyncSocket>(socket: &mut S, mut buf: &[u8]) -> Result<(), NetError> {
    while !buf.is_empty() {
        let written = socket.write(buf).await?;
        if written == 0 {
            return Err(NetError::Closed);
        }
        buf = &buf[written..];
    }
    Ok(())
}
