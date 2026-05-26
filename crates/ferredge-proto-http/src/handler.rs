extern crate alloc;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use ferredge_core::prelude::{
    AsyncNet, AsyncRuntime, AsyncSocket, NetError, bracket_ipv6_host, format_host_port,
    normalize_host_port, write_all_socket,
};

use crate::{HttpRequest, HttpResponse};

#[derive(Debug)]
struct ParsedEndpoint {
    connect_target: String,
    host_header: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum HttpTransportError {
    #[error("HTTP endpoint missing authority")]
    MissingAuthority,
    #[error("HTTP endpoint has malformed IPv6 authority")]
    MalformedIpv6Authority,
    #[error("failed to connect to endpoint: {0}")]
    Connect(#[source] NetError),
    #[error("failed to send request: {0}")]
    SendRequest(#[source] NetError),
    #[error("failed to send request body: {0}")]
    SendRequestBody(#[source] NetError),
    #[error("failed to flush request: {0}")]
    FlushRequest(#[source] NetError),
    #[error("failed to read response: {0}")]
    ReadResponse(#[source] NetError),
    #[error("invalid HTTP response: {0}")]
    InvalidResponse(String),
}

/// Sends one native HTTP request to endpoint and returns parsed response metadata and body.
pub async fn send_request<N, R>(
    endpoint: &str,
    request: &HttpRequest,
    _runtime: &R,
    net: &N,
) -> Result<HttpResponse, HttpTransportError>
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
        .map_err(HttpTransportError::Connect)?;

    write_all_socket(&mut upstream, wire_request.as_bytes())
        .await
        .map_err(HttpTransportError::SendRequest)?;

    if let Some(body) = &request.body {
        write_all_socket(&mut upstream, body)
            .await
            .map_err(HttpTransportError::SendRequestBody)?;
    }

    upstream
        .flush()
        .await
        .map_err(HttpTransportError::FlushRequest)?;

    let mut response = Vec::new();
    let mut buffer = [0u8; 4096];
    loop {
        let count = upstream
            .read(&mut buffer)
            .await
            .map_err(HttpTransportError::ReadResponse)?;
        if count == 0 {
            break;
        }
        response.extend_from_slice(&buffer[..count]);
    }

    parse_response(&response)
}

fn parse_endpoint(endpoint: &str) -> Result<ParsedEndpoint, HttpTransportError> {
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
        .ok_or(HttpTransportError::MissingAuthority)?;

    if authority.starts_with('[') {
        let Some(bracket_end) = authority.find(']') else {
            return Err(HttpTransportError::MalformedIpv6Authority);
        };

        let suffix = &authority[bracket_end + 1..];
        return match suffix {
            "" => Ok(ParsedEndpoint {
                connect_target: format_host_port(authority, default_port),
                host_header: authority.to_string(),
            }),
            _ if suffix.starts_with(':') => Ok(ParsedEndpoint {
                connect_target: authority.to_string(),
                host_header: authority.to_string(),
            }),
            _ => Err(HttpTransportError::MalformedIpv6Authority),
        };
    }

    if authority.matches(':').count() > 1 {
        Ok(ParsedEndpoint {
            connect_target: format_host_port(authority, default_port),
            host_header: bracket_ipv6_host(authority),
        })
    } else if authority.contains(':') {
        Ok(ParsedEndpoint {
            connect_target: authority.to_string(),
            host_header: authority.to_string(),
        })
    } else {
        Ok(ParsedEndpoint {
            connect_target: normalize_host_port(authority, default_port),
            host_header: authority.to_string(),
        })
    }
}

fn parse_response(response: &[u8]) -> Result<HttpResponse, HttpTransportError> {
    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| {
            HttpTransportError::InvalidResponse("missing header terminator".to_string())
        })?;
    let header_text = String::from_utf8_lossy(&response[..header_end]);
    let mut lines = header_text.lines();
    let status_line = lines
        .next()
        .ok_or_else(|| HttpTransportError::InvalidResponse("missing status line".to_string()))?;
    let status_code = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or_else(|| HttpTransportError::InvalidResponse("invalid status line".to_string()))?;
    let headers = lines
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            Some((name.trim().to_string(), value.trim().to_string()))
        })
        .collect::<Vec<_>>();

    if let Some(expected_len) = headers.iter().find_map(|(name, value)| {
        if name.eq_ignore_ascii_case("content-length") {
            value.parse::<usize>().ok()
        } else {
            None
        }
    }) {
        let body_len = response.len().saturating_sub(header_end + 4);
        if body_len < expected_len {
            return Err(HttpTransportError::InvalidResponse(
                "response body shorter than content-length".to_string(),
            ));
        }
    }
    Ok(HttpResponse {
        status_code,
        headers,
        body: response[header_end + 4..].to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::{HttpTransportError, parse_endpoint, parse_response};

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

        let parsed = parse_endpoint("2001:db8::10").expect("bare ipv6 host should parse");
        assert_eq!(parsed.connect_target, "[2001:db8::10]:80");
        assert_eq!(parsed.host_header, "[2001:db8::10]");
    }

    #[test]
    fn parse_endpoint_strips_path_before_connect() {
        let parsed =
            parse_endpoint("https://example.com/api/v1").expect("url with path should parse");
        assert_eq!(parsed.connect_target, "example.com:443");
        assert_eq!(parsed.host_header, "example.com");
    }

    #[test]
    fn parse_endpoint_rejects_missing_authority() {
        assert_eq!(
            parse_endpoint("http:///api").unwrap_err(),
            HttpTransportError::MissingAuthority
        );
    }

    #[test]
    fn parse_endpoint_rejects_malformed_ipv6_authority() {
        assert_eq!(
            parse_endpoint("http://[::1/api").unwrap_err(),
            HttpTransportError::MalformedIpv6Authority
        );
    }

    #[test]
    fn validate_response_rejects_missing_headers() {
        assert_eq!(
            parse_response(b"oops").unwrap_err(),
            HttpTransportError::InvalidResponse("missing header terminator".to_string())
        );
    }

    #[test]
    fn validate_response_rejects_truncated_body() {
        assert_eq!(
            parse_response(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\nhi")
                .unwrap_err(),
            HttpTransportError::InvalidResponse(
                "response body shorter than content-length".to_string()
            )
        );
    }

    #[test]
    fn parse_response_extracts_status_headers_and_body() {
        let response = parse_response(
            b"HTTP/1.1 201 Created\r\nContent-Type: text/plain\r\nX-Test: ok\r\n\r\nhello",
        )
        .expect("response should parse");
        assert_eq!(response.status_code, 201);
        assert_eq!(
            response.headers,
            vec![
                ("Content-Type".to_string(), "text/plain".to_string()),
                ("X-Test".to_string(), "ok".to_string())
            ]
        );
        assert_eq!(response.body, b"hello");
    }
}
