#[cfg(feature = "std")]
use {
    httparse::{Request, Status},
    std::{
        io::{Read, Write},
        net::TcpStream,
    },
};

use crate::attributes::HttpResourceAttributes;

#[cfg(feature = "std")]
pub fn send_request(
    mut stream: &TcpStream,
    endpoint: &str,
    attrs: &HttpResourceAttributes,
) -> Result<Option<Vec<u8>>, anyhow::Error> {
    let mut buffer = [0; 512];
    let bytes_read = stream.read(&mut buffer).map_err(|e| anyhow::anyhow!(e))?;

    let mut headers = [httparse::EMPTY_HEADER; 16];
    let mut req = Request::new(&mut headers);
    let status = req
        .parse(&buffer[..bytes_read])
        .map_err(|e| anyhow::anyhow!(e))?;
    match status {
        Status::Complete(_) => {
            // Build the HTTP request
            let mut request = format!(
                "{} {} HTTP/1.1\r\nHost: {}\r\n",
                attrs.method, attrs.slug, endpoint
            );

            // Add custom headers if provided
            if let Some(headers) = &attrs.headers {
                for (key, value) in headers {
                    request.push_str(&format!("{}: {}\r\n", key, value));
                }
            }

            // Add default headers
            request.push_str("Connection: close\r\n");
            request.push_str("\r\n");

            // Connect to the endpoint and send the request
            let mut upstream = TcpStream::connect(endpoint)
                .map_err(|e| anyhow::anyhow!("Failed to connect to endpoint: {}", e))?;

            upstream
                .write_all(request.as_bytes())
                .map_err(|e| anyhow::anyhow!("Failed to send request: {}", e))?;

            // Read the response
            let mut response = Vec::new();
            upstream
                .read_to_end(&mut response)
                .map_err(|e| anyhow::anyhow!("Failed to read response: {}", e))?;

            Ok(Some(response))
        }
        Status::Partial => Err(anyhow::anyhow!("Incomplete HTTP request")),
    }
}

#[cfg(not(feature = "std"))]
pub fn send_request(
    _stream: &(),
    _endpoint: &str,
    _attrs: &HttpResourceAttributes,
) -> Result<Option<Vec<u8>>, anyhow::Error> {
    Err(anyhow::anyhow!(
        "HTTP driver not implemented for no_std environment"
    ))
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use std::io::Write;
    use std::net::{TcpListener, TcpStream};
    use std::thread;
    use std::time::Duration;

    /// Helper function to create a mock HTTP server that responds to requests
    fn create_mock_server(response: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("Failed to bind to address");
        let addr = listener.local_addr().expect("Failed to get local address");

        thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buffer = [0; 512];
                let _ = stream.read(&mut buffer);
                let _ = stream.write_all(response.as_bytes());
            }
        });

        // Give the server time to start
        thread::sleep(Duration::from_millis(50));

        addr.to_string()
    }

    /// Helper function to create a client stream with a mock request
    fn create_mock_client_stream(request: &str) -> TcpStream {
        let listener = TcpListener::bind("127.0.0.1:0").expect("Failed to bind");
        let addr = listener.local_addr().expect("Failed to get local address");

        // Spawn a thread to write the request
        let request_owned = request.to_string();
        thread::spawn(move || {
            let mut stream = TcpStream::connect(addr).expect("Failed to connect");
            stream
                .write_all(request_owned.as_bytes())
                .expect("Failed to write");
        });

        // Accept the connection
        thread::sleep(Duration::from_millis(50));
        let (stream, _) = listener.accept().expect("Failed to accept");
        stream
    }

    #[test]
    fn test_send_request_basic_get() {
        let response = "HTTP/1.1 200 OK\r\nContent-Length: 13\r\n\r\nHello, World!";
        let endpoint = create_mock_server(response);

        let client_stream =
            create_mock_client_stream("GET /test HTTP/1.1\r\nHost: example.com\r\n\r\n");

        let attrs = HttpResourceAttributes {
            slug: "/api/test".to_string(),
            method: "GET".to_string(),
            headers: None,
        };

        let result = send_request(&client_stream, &endpoint, &attrs);
        assert!(result.is_ok());

        let response_data = result.unwrap();

        assert!(response_data.is_some());
        let binding = response_data.unwrap();
        let response_str = String::from_utf8_lossy(&binding);
        assert!(response_str.contains("200 OK"));
        assert!(response_str.contains("Hello, World!"));
    }

    #[test]
    fn test_send_request_with_custom_headers() {
        let response = "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nOK";
        let endpoint = create_mock_server(response);

        let client_stream =
            create_mock_client_stream("GET /test HTTP/1.1\r\nHost: example.com\r\n\r\n");

        let attrs = HttpResourceAttributes {
            slug: "/api/data".to_string(),
            method: "GET".to_string(),
            headers: Some(vec![
                ("Authorization".to_string(), "Bearer token123".to_string()),
                ("Content-Type".to_string(), "application/json".to_string()),
            ]),
        };

        let result = send_request(&client_stream, &endpoint, &attrs);
        assert!(result.is_ok());
        assert!(result.unwrap().is_some());
    }

    #[test]
    fn test_send_request_post_method() {
        let response = "HTTP/1.1 201 Created\r\nContent-Length: 7\r\n\r\nCreated";
        let endpoint = create_mock_server(response);

        let client_stream =
            create_mock_client_stream("POST /test HTTP/1.1\r\nHost: example.com\r\n\r\n");

        let attrs = HttpResourceAttributes {
            slug: "/api/create".to_string(),
            method: "POST".to_string(),
            headers: Some(vec![(
                "Content-Type".to_string(),
                "application/json".to_string(),
            )]),
        };

        let result = send_request(&client_stream, &endpoint, &attrs);
        assert!(result.is_ok());

        let response_data = result.unwrap();
        assert!(response_data.is_some());

        let binding = response_data.unwrap();
        let response_str = String::from_utf8_lossy(&binding);
        assert!(response_str.contains("201 Created"));
    }

    #[test]
    fn test_send_request_partial_request() {
        // Send an incomplete HTTP request
        let client_stream = create_mock_client_stream("GET /test HTTP/1.1\r\n");

        let attrs = HttpResourceAttributes {
            slug: "/api/test".to_string(),
            method: "GET".to_string(),
            headers: None,
        };

        // This should fail because the request is incomplete
        let result = send_request(&client_stream, "127.0.0.1:9999", &attrs);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Incomplete HTTP request")
        );
    }

    #[test]
    fn test_send_request_invalid_endpoint() {
        let client_stream =
            create_mock_client_stream("GET /test HTTP/1.1\r\nHost: example.com\r\n\r\n");

        let attrs = HttpResourceAttributes {
            slug: "/api/test".to_string(),
            method: "GET".to_string(),
            headers: None,
        };

        // Use an invalid endpoint that should fail to connect
        let result = send_request(&client_stream, "127.0.0.1:99999", &attrs);
        assert!(result.is_err());
    }

    #[test]
    fn test_send_request_empty_response() {
        let response = "HTTP/1.1 204 No Content\r\n\r\n";
        let endpoint = create_mock_server(response);

        let client_stream =
            create_mock_client_stream("DELETE /test HTTP/1.1\r\nHost: example.com\r\n\r\n");

        let attrs = HttpResourceAttributes {
            slug: "/api/delete/123".to_string(),
            method: "DELETE".to_string(),
            headers: None,
        };

        let result = send_request(&client_stream, &endpoint, &attrs);
        assert!(result.is_ok());

        let response_data = result.unwrap();
        assert!(response_data.is_some());

        let binding = response_data.unwrap();
        let response_str = String::from_utf8_lossy(&binding);
        assert!(response_str.contains("204 No Content"));
    }
}
