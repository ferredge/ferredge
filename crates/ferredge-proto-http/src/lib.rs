#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

use alloc::{
    string::{String, ToString},
    vec::Vec,
};

use ferredge_core::prelude::*;

pub mod attributes;
mod handler;
#[cfg(all(feature = "tokio-runtime", feature = "async-std-runtime"))]
compile_error!("ferredge-proto-http supports only one std runtime stack feature at a time");
#[cfg(not(any(
    feature = "tokio-runtime",
    feature = "async-std-runtime",
    feature = "embassy-runtime"
)))]
compile_error!("ferredge-proto-http requires one runtime stack feature");
#[cfg(feature = "tokio-runtime")]
mod runtime_stack {
    pub use ferredge_runtime_tokio::{TokioNet as StackNet, TokioRuntime as StackRuntime};
}
#[cfg(feature = "async-std-runtime")]
mod runtime_stack {
    pub use ferredge_runtime_async_std::{
        AsyncStdNet as StackNet, AsyncStdRuntime as StackRuntime,
    };
}
#[cfg(feature = "embassy-runtime")]
mod runtime_stack {
    pub use ferredge_runtime_embassy::{EmbassyNet as StackNet, EmbassyRuntime as StackRuntime};
}

#[cfg(any(
    feature = "tokio-runtime",
    feature = "async-std-runtime",
    feature = "embassy-runtime"
))]
use runtime_stack::{StackNet, StackRuntime};

/// Native outbound HTTP request used by the HTTP protocol adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    /// HTTP method sent to target endpoint.
    pub method: String,
    /// Request path or slug.
    pub path: String,
    /// Optional request body.
    pub body: Option<Vec<u8>>,
    /// Additional request headers.
    pub headers: Vec<(String, String)>,
}

/// Native HTTP response returned by the HTTP protocol adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    /// Raw response bytes returned from server.
    pub body: Vec<u8>,
}

/// Conversion error raised when routed command cannot be represented as HTTP request.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HttpCommandConversionError {
    /// Routed command intent cannot be executed via HTTP request/response semantics.
    #[error("unsupported intent for HTTP driver")]
    UnsupportedIntent,
    /// Target resource does not exist on concrete device definition.
    #[error("resource {0} not found for HTTP driver")]
    UnknownResource(String),
    /// Payload type cannot be represented as an HTTP request body.
    #[error("invalid HTTP payload: {0}")]
    InvalidPayload(String),
}

/// HTTP protocol adapter implementing lifecycle and request/response capabilities.
#[derive(Clone)]
pub struct HttpDriver {
    /// Device metadata and resource map served by this driver.
    pub dvc: Device<attributes::HttpResourceAttributes>,
    /// Selected runtime used by the live HTTP transport path.
    #[cfg(any(
        feature = "tokio-runtime",
        feature = "async-std-runtime",
        feature = "embassy-runtime"
    ))]
    runtime: StackRuntime,
    /// Selected network adapter used by the live HTTP transport path.
    #[cfg(any(
        feature = "tokio-runtime",
        feature = "async-std-runtime",
        feature = "embassy-runtime"
    ))]
    net: StackNet,
}

impl core::fmt::Debug for HttpDriver {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HttpDriver")
            .field("dvc", &self.dvc)
            .finish()
    }
}

/// Viewer that carries enough context to convert routed command into native HTTP request.
#[derive(Debug, Clone, Copy)]
pub struct HttpCommandRef<'a> {
    /// Driver-side device metadata used to resolve resources.
    pub device: &'a Device<attributes::HttpResourceAttributes>,
    /// Routed command to convert.
    pub command: &'a Command,
}

impl HttpDriver {
    /// Creates a new HTTP driver from device metadata.
    pub fn new(dvc: Device<attributes::HttpResourceAttributes>) -> Self {
        Self {
            dvc,
            #[cfg(any(
                feature = "tokio-runtime",
                feature = "async-std-runtime",
                feature = "embassy-runtime"
            ))]
            runtime: StackRuntime::default(),
            #[cfg(any(
                feature = "tokio-runtime",
                feature = "async-std-runtime",
                feature = "embassy-runtime"
            ))]
            net: StackNet::default(),
        }
    }
}

impl TryFrom<HttpCommandRef<'_>> for HttpRequest {
    type Error = HttpCommandConversionError;

    fn try_from(value: HttpCommandRef<'_>) -> Result<Self, Self::Error> {
        match &value.command.intent {
            Intent::Read { resource }
            | Intent::Invoke {
                operation: resource,
                ..
            } => value
                .device
                .resources
                .get(resource)
                .map(|resource| HttpRequest {
                    method: resource.resource_attributes.method.clone(),
                    path: resource.resource_attributes.slug.clone(),
                    body: None,
                    headers: resource
                        .resource_attributes
                        .headers
                        .clone()
                        .unwrap_or_default(),
                })
                .ok_or_else(|| HttpCommandConversionError::UnknownResource(resource.clone())),
            Intent::Write { resource, payload } => value
                .device
                .resources
                .get(resource)
                .map(|resource| {
                    Ok(HttpRequest {
                        method: resource.resource_attributes.method.clone(),
                        path: resource.resource_attributes.slug.clone(),
                        body: Some(http_body_from_payload(payload)?),
                        headers: resource
                            .resource_attributes
                            .headers
                            .clone()
                            .unwrap_or_default(),
                    })
                })
                .transpose()?
                .ok_or_else(|| HttpCommandConversionError::UnknownResource(resource.clone())),
            Intent::Send { .. } | Intent::Subscribe { .. } | Intent::Unsubscribe { .. } => {
                Err(HttpCommandConversionError::UnsupportedIntent)
            }
        }
    }
}

fn http_body_from_payload(payload: &PayloadValue) -> Result<Vec<u8>, HttpCommandConversionError> {
    match payload {
        PayloadValue::Bytes(bytes) => Ok(bytes.clone()),
        PayloadValue::String(value) => Ok(value.clone().into_bytes()),
        _ => Err(HttpCommandConversionError::InvalidPayload(
            "HTTP bodies currently support only string or bytes payloads".to_string(),
        )),
    }
}

impl Lifecycle for HttpDriver {
    type Error = String;

    async fn start(&self) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn stop(&self) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl RequestResponse for HttpDriver {
    type Request = HttpRequest;
    type Response = HttpResponse;
    type Error = String;

    async fn execute(&self, request: Self::Request) -> Result<Self::Response, Self::Error> {
        let endpoint = match &self.dvc.endpoint {
            DeviceEndpoint::Http(config) => config.url.as_str(),
            _ => return Err("device endpoint is not HTTP".to_string()),
        };

        #[cfg(any(
            feature = "tokio-runtime",
            feature = "async-std-runtime",
            feature = "embassy-runtime"
        ))]
        {
            return handler::send_request(endpoint, &request, &self.runtime, &self.net)
                .await
                .map(|body| HttpResponse { body })
                .map_err(|e| e.to_string());
        }

        #[allow(unreachable_code)]
        Err("HTTP driver runtime stack is unavailable".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferredge_core::prelude::{
        BrokerAddress, BrokerChannelKind, BrokerMessageOptions, DeviceEndpoint,
        DeviceResourceAccessPermission, DeviceStatus, HttpEndpointConfig, Intent, Map,
    };

    fn make_driver() -> HttpDriver {
        let mut resources = Map::default();
        resources.insert(
            "temp".to_string(),
            DeviceResource {
                name: "temp".to_string(),
                resource_attributes: attributes::HttpResourceAttributes {
                    slug: "/api/temp".to_string(),
                    method: "GET".to_string(),
                    headers: Some(vec![("Accept".to_string(), "application/json".to_string())]),
                },
                unit: Some("C".to_string()),
                permission: Some(DeviceResourceAccessPermission::READ),
            },
        );
        resources.insert(
            "setpoint".to_string(),
            DeviceResource {
                name: "setpoint".to_string(),
                resource_attributes: attributes::HttpResourceAttributes {
                    slug: "/api/setpoint".to_string(),
                    method: "POST".to_string(),
                    headers: None,
                },
                unit: Some("C".to_string()),
                permission: Some(DeviceResourceAccessPermission::WRITE),
            },
        );

        HttpDriver::new(Device {
            id: "device-1".to_string(),
            name: "HTTP Device".to_string(),
            status: DeviceStatus::Online,
            endpoint: DeviceEndpoint::http(HttpEndpointConfig {
                url: "127.0.0.1:8080".to_string(),
            }),
            metadata: None,
            max_connections: Some(4),
            resources,
            message_endpoints: Vec::new(),
        })
    }

    #[test]
    fn read_command_converts_to_http_request() {
        let driver = make_driver();
        let command = Command {
            id: "cmd-1".to_string(),
            source_device_id: None,
            target_device_id: "device-1".to_string(),
            intent: Intent::Read {
                resource: "temp".to_string(),
            },
            correlation: None,
        };

        let request = HttpRequest::try_from(HttpCommandRef {
            device: &driver.dvc,
            command: &command,
        })
        .expect("read intent should convert");

        assert_eq!(request.method, "GET");
        assert_eq!(request.path, "/api/temp");
        assert_eq!(request.body, None);
        assert_eq!(
            request.headers,
            vec![("Accept".to_string(), "application/json".to_string())]
        );
    }

    #[test]
    fn write_command_converts_to_http_request_with_body() {
        let driver = make_driver();
        let command = Command {
            id: "cmd-2".to_string(),
            source_device_id: None,
            target_device_id: "device-1".to_string(),
            intent: Intent::Write {
                resource: "setpoint".to_string(),
                payload: PayloadValue::String("42".to_string()),
            },
            correlation: None,
        };

        let request = HttpRequest::try_from(HttpCommandRef {
            device: &driver.dvc,
            command: &command,
        })
        .expect("write intent should convert");

        assert_eq!(request.method, "POST");
        assert_eq!(request.path, "/api/setpoint");
        assert_eq!(request.body, Some(b"42".to_vec()));
    }

    #[test]
    fn broker_send_intent_is_rejected_for_http() {
        let driver = make_driver();
        let command = Command {
            id: "cmd-3".to_string(),
            source_device_id: None,
            target_device_id: "device-1".to_string(),
            intent: Intent::Send {
                channel: BrokerAddress {
                    name: "sensors.temp".to_string(),
                    kind: Some(BrokerChannelKind::Topic),
                },
                payload: PayloadValue::String("42".to_string()),
                options: BrokerMessageOptions::default(),
            },
            correlation: None,
        };

        let error = HttpRequest::try_from(HttpCommandRef {
            device: &driver.dvc,
            command: &command,
        })
        .expect_err("broker send intent should be unsupported");

        assert_eq!(error, HttpCommandConversionError::UnsupportedIntent);
    }
}
