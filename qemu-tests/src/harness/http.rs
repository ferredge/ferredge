//! HTTP leg of the harness: fetches a static file from the host HTTP server through
//! the LAN9118 link.

use alloc::{string::ToString, vec::Vec};

use ferredge_core::prelude::*;
use ferredge_proto_http::{HttpDriver, HttpRequest};
use ferredge_runtime_embassy::{EmbassyNet, EmbassyRuntime};

use super::HOST_ADDR;

pub async fn run(runtime: &EmbassyRuntime, net: EmbassyNet) {
    let driver = HttpDriver::with_stack(
        Device {
            id: "harness-http".to_string(),
            name: "http.server".to_string(),
            status: DeviceStatus::Online,
            endpoint: DeviceEndpoint::Http(HttpEndpointConfig {
                url: alloc::format!("http://{HOST_ADDR}:48080"),
            }),
            metadata: None,
            max_connections: Some(1),
            resources: Map::default(),
            message_endpoints: Vec::new(),
        },
        runtime.clone(),
        net,
    );

    log::debug!("GET http://{HOST_ADDR}:48080/hello.txt");
    let response = driver
        .execute(HttpRequest {
            method: "GET".to_string(),
            path: "/hello.txt".to_string(),
            body: None,
            headers: Vec::new(),
        })
        .await
        .expect("http request should succeed");
    log::debug!(
        "http status {}, body {:?}",
        response.status_code,
        core::str::from_utf8(&response.body).unwrap_or("<non-utf8>")
    );
    assert_eq!(response.status_code, 200);
    assert_eq!(response.body, b"hello from the ferredge harness\n");
}
