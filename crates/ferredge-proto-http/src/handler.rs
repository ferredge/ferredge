extern crate alloc;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use ferredge_core::prelude::{AsyncNet, AsyncRuntime, AsyncSocket, write_all_socket};

use crate::HttpRequest;

struct ParsedEndpoint {
    connect_target: String,
    host_header: String,
}

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
    let parsed_endpoint = parse_endpoint(endpoint)?;
    let mut wire_request = format!(
        "{} {} HTTP/1.1\r\nHost: {}\r\n",
        request.method, request.path, parsed_endpoint.host_header
    );

    for (key, value) in &request.headers {
        wire_request.push_str(&format!("{key}: {value}\r\n"));
    }

    if let Some(body) = &request.body {
        wire_request.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }

    wire_request.push_str("Connection: close\r\n\r\n");

    let mut upstream = net
        .connect(parsed_endpoint.connect_target.as_str())
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

fn parse_endpoint(endpoint: &str) -> Result<ParsedEndpoint, anyhow::Error> {
    let (authority_and_path, default_port) = if let Some(rest) = endpoint.strip_prefix("http://") {
        (rest, 80)
    } else if let Some(rest) = endpoint.strip_prefix("https://") {
        (rest, 443)
    } else {
        (endpoint, 80)
    };

    let authority = authority_and_path
        .split('/')
        .next()
        .filter(|authority| !authority.is_empty())
        .ok_or_else(|| anyhow::anyhow!("HTTP endpoint missing authority"))?;

    if authority.starts_with('[') {
        let Some(bracket_end) = authority.find(']') else {
            return Err(anyhow::anyhow!(
                "HTTP endpoint has malformed IPv6 authority"
            ));
        };

        let suffix = &authority[bracket_end + 1..];
        return match suffix {
            "" => Ok(ParsedEndpoint {
                connect_target: format!("{authority}:{default_port}"),
                host_header: authority.to_string(),
            }),
            _ if suffix.starts_with(':') => Ok(ParsedEndpoint {
                connect_target: authority.to_string(),
                host_header: authority.to_string(),
            }),
            _ => Err(anyhow::anyhow!(
                "HTTP endpoint has malformed IPv6 authority"
            )),
        };
    }

    if authority.contains(':') {
        Ok(ParsedEndpoint {
            connect_target: authority.to_string(),
            host_header: authority.to_string(),
        })
    } else {
        Ok(ParsedEndpoint {
            connect_target: format!("{authority}:{default_port}"),
            host_header: authority.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::parse_endpoint;

    #[test]
    fn parse_endpoint_adds_default_ports() {
        let parsed = parse_endpoint("example.com").expect("host should parse");
        assert_eq!(parsed.connect_target, "example.com:80");
        assert_eq!(parsed.host_header, "example.com");

        let parsed = parse_endpoint("http://example.com").expect("http url should parse");
        assert_eq!(parsed.connect_target, "example.com:80");
        assert_eq!(parsed.host_header, "example.com");

        let parsed = parse_endpoint("https://example.com").expect("https url should parse");
        assert_eq!(parsed.connect_target, "example.com:443");
        assert_eq!(parsed.host_header, "example.com");
    }

    #[test]
    fn parse_endpoint_handles_bracketed_ipv6() {
        let parsed = parse_endpoint("[::1]").expect("ipv6 host should parse");
        assert_eq!(parsed.connect_target, "[::1]:80");
        assert_eq!(parsed.host_header, "[::1]");

        let parsed = parse_endpoint("http://[::1]").expect("ipv6 url should parse");
        assert_eq!(parsed.connect_target, "[::1]:80");
        assert_eq!(parsed.host_header, "[::1]");

        let parsed = parse_endpoint("https://[::1]").expect("ipv6 https url should parse");
        assert_eq!(parsed.connect_target, "[::1]:443");
        assert_eq!(parsed.host_header, "[::1]");

        let parsed = parse_endpoint("http://[::1]:8080").expect("ipv6 with port should parse");
        assert_eq!(parsed.connect_target, "[::1]:8080");
        assert_eq!(parsed.host_header, "[::1]:8080");
    }

    #[test]
    fn parse_endpoint_strips_path_before_connect() {
        let parsed =
            parse_endpoint("https://example.com/api/v1").expect("url with path should parse");
        assert_eq!(parsed.connect_target, "example.com:443");
        assert_eq!(parsed.host_header, "example.com");
    }
}
