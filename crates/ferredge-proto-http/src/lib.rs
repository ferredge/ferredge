#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

use alloc::{
    borrow::Cow,
    format,
    string::{String, ToString},
    vec::Vec,
};

use ferredge_bridge::{
    BridgeMessage, BridgeOp, BridgeOutbound, BridgePlannerError, NativeOutbound, ProtocolDecoder,
    ProtocolEncoder, ProtocolPlanner, RequestResponseAction, planner,
};
use ferredge_core::prelude::*;

pub mod attributes;
mod endpoint;
mod handler;
mod meta;

pub use endpoint::{HttpDevice, HttpEndpointConfig};
pub use meta::{HttpMeta, KEY_CONTENT_TYPE, KEY_METHOD, KEY_PATH, KEY_STATUS_CODE};
#[cfg(any(
    all(feature = "tokio-runtime", feature = "async-std-runtime"),
    all(feature = "tokio-runtime", feature = "embassy-runtime"),
    all(feature = "async-std-runtime", feature = "embassy-runtime")
))]
compile_error!("ferredge-proto-http supports only one runtime stack feature at a time");
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
    /// Parsed HTTP response status code.
    pub status_code: u16,
    /// Parsed HTTP response headers.
    pub headers: Vec<(String, String)>,
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
    /// Bridge layer rejected semantic planning.
    #[error("invalid bridge request: {0}")]
    Bridge(#[from] BridgePlannerError),
    /// Bridge message kind cannot be represented as outbound HTTP request.
    #[error("bridge message does not describe an HTTP request")]
    InvalidBridgeMessage,
    /// Options carried a transport capture for a different protocol.
    #[error("options extensions capture belongs to protocol {0}, not HTTP")]
    ForeignExtensions(ProtocolId),
    /// Extensions capture entry does not fit the HTTP wire field width.
    #[error("extensions capture entry {key} has out-of-range value {value}")]
    InvalidExtensionValue { key: &'static str, value: u64 },
    /// Extensions capture entry stores a value type that does not match its documented key.
    #[error("extensions capture entry {key} has a mismatched value type")]
    InvalidExtensionType { key: &'static str },
}

/// HTTP protocol adapter implementing lifecycle and request/response capabilities.
#[derive(Clone)]
pub struct HttpDriver {
    /// Device metadata and resource map served by this driver.
    pub dvc: HttpDevice,
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

pub struct HttpRequestEncoder<'a> {
    device: &'a HttpDevice,
}

/// Outbound planner for HTTP request/response semantics.
pub struct HttpCommandPlanner;

/// Borrowed native outbound HTTP plan shared by direct and bridge-driven planning.
#[derive(Debug, Clone, PartialEq)]
pub struct HttpNativePlan<'a> {
    resource: Cow<'a, str>,
    action: RequestResponseAction,
    payload: Option<PayloadValue<'a>>,
    method: Option<Cow<'a, str>>,
    path: Option<Cow<'a, str>>,
    content_type: Option<Cow<'a, str>>,
    headers: Vec<(Cow<'a, str>, Cow<'a, str>)>,
}

impl HttpNativePlan<'_> {
    pub fn into_owned(self) -> HttpNativePlan<'static> {
        HttpNativePlan {
            resource: Cow::Owned(self.resource.into_owned()),
            action: self.action,
            payload: self.payload.map(PayloadValue::into_owned),
            method: self.method.map(|value| Cow::Owned(value.into_owned())),
            path: self.path.map(|value| Cow::Owned(value.into_owned())),
            content_type: self
                .content_type
                .map(|value| Cow::Owned(value.into_owned())),
            headers: self
                .headers
                .into_iter()
                .map(|(key, value)| (Cow::Owned(key.into_owned()), Cow::Owned(value.into_owned())))
                .collect(),
        }
    }
}

/// Inbound HTTP response decoder bound to one originating command.
pub struct HttpResponseDecoder<'a> {
    device: &'a HttpDevice,
    command: &'a Command<'a>,
}

/// Decoded inbound HTTP response plus bound command context.
pub struct HttpDecodedResponse<'a, 'ctx> {
    device: &'ctx HttpDevice,
    command: &'ctx Command<'ctx>,
    response: &'a HttpResponse,
}

impl HttpDriver {
    /// Creates a new HTTP driver from device metadata.
    ///
    /// Uses the ambient runtime stack, so it is only available on the std runtimes; the
    /// embassy stack has no ambient executor or network stack — construct with
    /// [`HttpDriver::with_stack`] there.
    #[cfg(any(feature = "tokio-runtime", feature = "async-std-runtime"))]
    pub fn new(dvc: HttpDevice) -> Self {
        Self::with_stack(dvc, StackRuntime::default(), StackNet::default())
    }

    /// Creates a new HTTP driver from device metadata plus an explicit runtime stack.
    pub fn with_stack(dvc: HttpDevice, runtime: StackRuntime, net: StackNet) -> Self {
        Self { dvc, runtime, net }
    }

    pub fn native_request(
        &self,
        command: Command<'_>,
    ) -> Result<HttpRequest, HttpCommandConversionError> {
        let planner = HttpCommandPlanner;
        let plan =
            <HttpCommandPlanner as ProtocolPlanner<NativeOutbound>>::plan(&planner, command)?;
        <HttpRequestEncoder<'_> as ProtocolEncoder<
            NativeOutbound,
            HttpNativePlan<'_>,
            HttpRequest,
        >>::encode(&HttpRequestEncoder { device: &self.dvc }, plan)
    }

    pub fn bridge_request(
        &self,
        command: Command<'_>,
    ) -> Result<HttpRequest, HttpCommandConversionError> {
        let planner = HttpCommandPlanner;
        let bridge =
            <HttpCommandPlanner as ProtocolPlanner<BridgeOutbound>>::plan(&planner, command)?;
        let native = HttpNativePlan::try_from(&bridge)?;
        <HttpRequestEncoder<'_> as ProtocolEncoder<
            NativeOutbound,
            HttpNativePlan<'_>,
            HttpRequest,
        >>::encode(&HttpRequestEncoder { device: &self.dvc }, native)
    }
}

impl ProtocolPlanner<BridgeOutbound> for HttpCommandPlanner {
    type Error = BridgePlannerError;
    type Planned<'a> = BridgeMessage<'a>;

    fn plan<'a>(&self, command: Command<'a>) -> Result<BridgeMessage<'a>, Self::Error> {
        planner::command_to_request_response(command)
    }
}

impl ProtocolPlanner<NativeOutbound> for HttpCommandPlanner {
    type Error = HttpCommandConversionError;
    type Planned<'a> = HttpNativePlan<'a>;

    fn plan<'a>(&self, command: Command<'a>) -> Result<HttpNativePlan<'a>, Self::Error> {
        let (resource, action, payload, options) = match command.intent {
            Intent::Read { resource, options } => (
                Cow::from(resource),
                RequestResponseAction::Read,
                None,
                options,
            ),
            Intent::Write {
                resource,
                payload,
                options,
            } => (
                Cow::from(resource),
                RequestResponseAction::Write,
                Some(payload),
                options,
            ),
            Intent::Invoke {
                operation,
                args,
                options,
            } => (
                Cow::from(operation),
                RequestResponseAction::Invoke,
                args,
                options,
            ),
            _ => return Err(HttpCommandConversionError::UnsupportedIntent),
        };

        let mut capture = split_http_extensions(options.extensions)?;
        let mut headers = options.headers;
        if let Some(capture_headers) = capture.headers.take() {
            headers.merge(capture_headers);
        }
        Ok(HttpNativePlan {
            resource,
            action,
            payload,
            method: options.method.map(Cow::Owned).or(capture.method),
            path: options.path.map(Cow::Owned).or(capture.path),
            content_type: options
                .content_type
                .map(Cow::Owned)
                .or(capture.content_type),
            headers: headers
                .0
                .into_iter()
                .map(|header| (header.key, header.value))
                .collect(),
        })
    }
}

/// Splits an options extensions capture into a typed HTTP view, rejecting foreign protocols and malformed captures.
fn split_http_extensions(
    extensions: Option<TransportMeta<'_>>,
) -> Result<HttpMeta<'_>, HttpCommandConversionError> {
    match extensions {
        Some(meta) => HttpMeta::try_from_transport_meta(meta),
        None => Ok(HttpMeta::default()),
    }
}

impl<'a> HttpResponseDecoder<'a> {
    pub fn new(device: &'a HttpDevice, command: &'a Command<'a>) -> Self {
        Self { device, command }
    }

    fn decode_response<'b>(&self, response: &'b HttpResponse) -> HttpDecodedResponse<'b, 'a> {
        HttpDecodedResponse {
            device: self.device,
            command: self.command,
            response,
        }
    }
}

impl<'a> HttpRequestEncoder<'a> {
    pub fn new(device: &'a HttpDevice) -> Self {
        Self { device }
    }
}

impl<'p> ProtocolEncoder<NativeOutbound, HttpNativePlan<'p>, HttpRequest>
    for HttpRequestEncoder<'_>
{
    type Error = HttpCommandConversionError;

    fn encode(&self, planned: HttpNativePlan<'p>) -> Result<HttpRequest, Self::Error> {
        let resource_def = self
            .device
            .resources
            .get(planned.resource.as_ref())
            .ok_or_else(|| {
                HttpCommandConversionError::UnknownResource(planned.resource.to_string())
            })?;
        // Every action encodes an optional payload body the same way, known or future.
        let body = planned
            .payload
            .as_ref()
            .map(http_body_from_payload)
            .transpose()?;
        let mut method = resource_def.resource_attributes.method.clone();
        let request_path = planned
            .path
            .as_deref()
            .unwrap_or(resource_def.resource_attributes.slug.as_str())
            .to_string();
        let mut headers = resource_def
            .resource_attributes
            .headers
            .clone()
            .unwrap_or_default();

        if let Some(override_method) = &planned.method {
            method = override_method.to_string();
        }
        for (key, value) in &planned.headers {
            merge_header(&mut headers, key.to_string(), value.to_string());
        }
        if let Some(content_type) = &planned.content_type {
            merge_header(
                &mut headers,
                "Content-Type".to_string(),
                content_type.to_string(),
            );
        }

        Ok(HttpRequest {
            method,
            path: request_path,
            body,
            headers,
        })
    }
}

impl<'ctx> ProtocolDecoder<HttpResponse> for HttpResponseDecoder<'ctx> {
    type Error = HttpCommandConversionError;
    type Decoded<'a>
        = HttpDecodedResponse<'a, 'ctx>
    where
        HttpResponse: 'a;

    fn decode<'a>(&self, native: &'a HttpResponse) -> Result<Self::Decoded<'a>, Self::Error> {
        Ok(self.decode_response(native))
    }
}

impl<'a, 'ctx> TryFrom<HttpDecodedResponse<'a, 'ctx>> for RoutedMessage<'a>
where
    'ctx: 'a,
{
    type Error = HttpCommandConversionError;

    fn try_from(value: HttpDecodedResponse<'a, 'ctx>) -> Result<Self, Self::Error> {
        let self_ = value;
        let (resource, options) = match &self_.command.intent {
            Intent::Read { resource, options }
            | Intent::Write {
                resource, options, ..
            } => (resource.as_ref(), options),
            Intent::Invoke {
                operation, options, ..
            } => (operation.as_ref(), options),
            _ => return Err(HttpCommandConversionError::UnsupportedIntent),
        };

        let content_type = header_value(&self_.response.headers, "content-type");
        let payload = http_payload_from_response_body(&self_.response.body, content_type);
        let transport = HttpMeta {
            method: options.method.as_deref().map(Cow::Borrowed),
            path: options.path.as_deref().map(Cow::Borrowed),
            status_code: Some(self_.response.status_code),
            content_type: content_type.map(Cow::Borrowed),
            headers: (!self_.response.headers.is_empty()).then(|| {
                Headers::from(
                    self_
                        .response
                        .headers
                        .iter()
                        .map(|(key, value)| {
                            (Cow::Borrowed(key.as_str()), Cow::Borrowed(value.as_str()))
                        })
                        .collect::<Vec<_>>(),
                )
            }),
        }
        .into_transport_meta();

        Ok(RoutedMessage::Result(RoutedResult::new(
            EndpointRef::new(self_.device.id.clone(), ProtocolId::HTTP),
            CommandResult::new(
                self_.command.id.clone(),
                self_.device.id.clone(),
                if self_.response.status_code < 400 {
                    DeliveryState::Completed
                } else {
                    DeliveryState::Rejected
                },
                Some(payload),
                (self_.response.status_code >= 400)
                    .then(|| format!("http status {}", self_.response.status_code)),
                self_
                    .command
                    .correlation
                    .as_ref()
                    .map(Correlation::as_borrowed)
                    .or_else(|| {
                        Some(Correlation::new(
                            self_.command.id.clone(),
                            Some(Address::Resource(Cow::Borrowed(resource))),
                        ))
                    }),
            ),
            Some(transport),
        )))
    }
}

impl<'a, 'ctx> TryFrom<HttpDecodedResponse<'a, 'ctx>> for BridgeMessage<'a>
where
    'ctx: 'a,
{
    type Error = HttpCommandConversionError;

    fn try_from(value: HttpDecodedResponse<'a, 'ctx>) -> Result<Self, Self::Error> {
        let routed = RoutedMessage::try_from(value)?;
        let RoutedMessage::Result(result) = routed else {
            unreachable!("http decoded response always projects to result")
        };
        Ok(planner::routed_result_to_bridge(result))
    }
}

impl<'a> TryFrom<&BridgeMessage<'a>> for HttpNativePlan<'a> {
    type Error = HttpCommandConversionError;

    fn try_from(message: &BridgeMessage<'a>) -> Result<Self, Self::Error> {
        let BridgeMessage::Command(command) = message else {
            return Err(HttpCommandConversionError::InvalidBridgeMessage);
        };
        let BridgeOp::RequestResponse(operation) = &command.operation else {
            return Err(HttpCommandConversionError::InvalidBridgeMessage);
        };
        let Route::RequestResponse { resource, path } = &command.route else {
            return Err(HttpCommandConversionError::InvalidBridgeMessage);
        };

        let capture = match command.transport.as_ref() {
            Some(transport) => {
                HttpMeta::try_from_transport_meta(transport.as_borrowed())?.into_owned()
            }
            None => HttpMeta::default(),
        };

        Ok(HttpNativePlan {
            resource: resource.clone(),
            action: operation.action.clone(),
            payload: command.payload.clone(),
            method: operation.method.clone().map(Cow::Owned).or(capture.method),
            path: path.clone().or(capture.path),
            content_type: operation
                .content_type
                .clone()
                .map(Cow::Owned)
                .or(capture.content_type),
            headers: command
                .headers
                .as_ref()
                .map(|headers| {
                    headers
                        .iter()
                        .map(|header| (header.key.clone(), header.value.clone()))
                        .collect()
                })
                .unwrap_or_default(),
        })
    }
}

fn http_body_from_payload(
    payload: &PayloadValue<'_>,
) -> Result<Vec<u8>, HttpCommandConversionError> {
    match payload {
        PayloadValue::Bytes(bytes) => Ok(bytes.to_vec()),
        PayloadValue::String(value) => Ok(value.as_bytes().to_vec()),
        PayloadValue::Null => Ok(Vec::new()),
        _ => Err(HttpCommandConversionError::InvalidPayload(
            "HTTP bodies currently support only string or bytes payloads".to_string(),
        )),
    }
}

fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers.iter().find_map(|(key, value)| {
        if key.eq_ignore_ascii_case(name) {
            Some(value.as_str())
        } else {
            None
        }
    })
}

fn http_payload_from_response_body<'a>(
    body: &'a [u8],
    content_type: Option<&str>,
) -> PayloadValue<'a> {
    if content_type.is_some_and(|value| value.starts_with("text/"))
        && let Ok(text) = core::str::from_utf8(body)
    {
        return PayloadValue::String(Cow::Borrowed(text));
    }
    PayloadValue::Bytes(Cow::Borrowed(body))
}

fn merge_header(headers: &mut Vec<(String, String)>, key: String, value: String) {
    if let Some(existing) = headers
        .iter_mut()
        .find(|(existing_key, _)| existing_key.eq_ignore_ascii_case(&key))
    {
        existing.0 = key;
        existing.1 = value;
    } else {
        headers.push((key, value));
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
        let endpoint = self.dvc.endpoint.url.as_str();

        #[cfg(any(
            feature = "tokio-runtime",
            feature = "async-std-runtime",
            feature = "embassy-runtime"
        ))]
        {
            return handler::send_request(endpoint, &request, &self.runtime, &self.net)
                .await
                .map(|response| response)
                .map_err(|e| e.to_string());
        }

        #[allow(unreachable_code)]
        Err("HTTP driver runtime stack is unavailable".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferredge_bridge::ProtocolDecoder;
    use ferredge_core::prelude::{
        BrokerAddress, BrokerChannelKind, BrokerMessageOptions, DeviceResourceAccessPermission,
        DeviceStatus, Intent, Map,
    };
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::mpsc,
        thread,
        time::Duration,
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
            id: "device-1".into(),
            name: "HTTP Device".to_string(),
            status: DeviceStatus::Online,
            endpoint: HttpEndpointConfig {
                url: "127.0.0.1:8080".to_string(),
            },
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
            id: "cmd-1".into(),
            source_device_id: None,
            target_device_id: "device-1".into(),
            intent: Intent::Read {
                resource: "temp".into(),
                options: RequestOptions::default(),
            },
            correlation: None,
        };

        let request = driver
            .bridge_request(command)
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
            id: "cmd-2".into(),
            source_device_id: None,
            target_device_id: "device-1".into(),
            intent: Intent::Write {
                resource: "setpoint".into(),
                payload: PayloadValue::String("42".to_string().into()),
                options: RequestOptions::default(),
            },
            correlation: None,
        };

        let request = driver
            .bridge_request(command)
            .expect("write intent should convert");

        assert_eq!(request.method, "POST");
        assert_eq!(request.path, "/api/setpoint");
        assert_eq!(request.body, Some(b"42".to_vec()));
    }

    #[test]
    fn broker_send_intent_is_rejected_for_http() {
        let driver = make_driver();
        let command = Command {
            id: "cmd-3".into(),
            source_device_id: None,
            target_device_id: "device-1".into(),
            intent: Intent::Send {
                channel: BrokerAddress {
                    name: "sensors.temp".into(),
                    kind: Some(BrokerChannelKind::Topic),
                },
                payload: PayloadValue::String("42".to_string().into()),
                options: BrokerMessageOptions::default(),
            },
            correlation: None,
        };

        let error = driver
            .bridge_request(command)
            .expect_err("broker send intent should be unsupported");

        assert_eq!(
            error,
            HttpCommandConversionError::Bridge(BridgePlannerError::UnsupportedIntent)
        );
    }

    #[test]
    fn execute_reports_connect_failure_for_unopened_port() {
        let driver = make_driver();
        let error = runtime_stack::StackRuntime::default()
            .block_on(driver.execute(HttpRequest {
                method: "GET".to_string(),
                path: "/api/temp".to_string(),
                body: None,
                headers: Vec::new(),
            }))
            .unwrap_err();

        assert!(error.contains("failed to connect to endpoint"));
    }

    #[test]
    fn execute_reports_invalid_response_when_server_closes_early() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("test listener should bind");
        let addr = listener
            .local_addr()
            .expect("test listener should have address");
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("server should accept");
            stream
                .set_read_timeout(Some(Duration::from_secs(1)))
                .expect("server should set read timeout");
            let mut buf = [0u8; 128];
            let _ = stream.read(&mut buf);
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhi")
                .expect("server should write partial response");
        });

        let driver = make_http_driver_for_url(addr.to_string());
        let error = runtime_stack::StackRuntime::default()
            .block_on(driver.execute(HttpRequest {
                method: "GET".to_string(),
                path: "/api/temp".to_string(),
                body: None,
                headers: Vec::new(),
            }))
            .unwrap_err();
        handle.join().expect("server should join");

        assert!(error.contains("invalid HTTP response"));
        assert!(error.contains("response body shorter than content-length"));
    }

    #[test]
    fn execute_passes_through_non_success_status_response() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("test listener should bind");
        let addr = listener
            .local_addr()
            .expect("test listener should have address");
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("server should accept");
            stream
                .set_read_timeout(Some(Duration::from_secs(1)))
                .expect("server should set read timeout");
            let mut buf = [0u8; 128];
            let _ = stream.read(&mut buf);
            stream
                .write_all(
                    b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 4\r\nConnection: close\r\n\r\nnope",
                )
                .expect("server should write response");
        });

        let driver = make_http_driver_for_url(addr.to_string());
        let response = runtime_stack::StackRuntime::default()
            .block_on(driver.execute(HttpRequest {
                method: "GET".to_string(),
                path: "/api/temp".to_string(),
                body: None,
                headers: Vec::new(),
            }))
            .expect("http driver should pass through non-2xx response");
        handle.join().expect("server should join");

        assert_eq!(response.status_code, 503);
        assert_eq!(response.body, b"nope");
    }

    fn make_http_driver_for_url(url: String) -> HttpDriver {
        let mut driver = make_driver();
        driver.dvc.endpoint = HttpEndpointConfig { url };
        driver
    }

    fn run_single_request_server(
        response: &'static [u8],
    ) -> (String, mpsc::Receiver<Vec<u8>>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("test listener should bind");
        let addr = listener.local_addr().expect("listener should have address");
        let (tx, rx) = mpsc::channel();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("server should accept");
            stream
                .set_read_timeout(Some(Duration::from_secs(1)))
                .expect("server should set read timeout");
            let mut request = Vec::new();
            loop {
                let mut buf = [0u8; 1024];
                match stream.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        request.extend_from_slice(&buf[..n]);
                        if request.windows(4).any(|window| window == b"\r\n\r\n") {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            tx.send(request).expect("request should be sent");
            stream
                .write_all(response)
                .expect("server should write response");
        });
        (addr.to_string(), rx, handle)
    }

    #[test]
    fn native_read_command_converts_to_http_request() {
        let driver = make_driver();
        let command = Command {
            id: "cmd-native-read".into(),
            source_device_id: None,
            target_device_id: "device-1".into(),
            intent: Intent::Read {
                resource: "temp".into(),
                options: RequestOptions::default(),
            },
            correlation: None,
        };

        let request = driver
            .native_request(command)
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
    fn native_write_command_converts_to_http_request_with_body() {
        let driver = make_driver();
        let command = Command {
            id: "cmd-native-write".into(),
            source_device_id: None,
            target_device_id: "device-1".into(),
            intent: Intent::Write {
                resource: "setpoint".into(),
                payload: PayloadValue::String("42".to_string().into()),
                options: RequestOptions::default(),
            },
            correlation: None,
        };

        let request = driver
            .native_request(command)
            .expect("write intent should convert");

        assert_eq!(request.method, "POST");
        assert_eq!(request.path, "/api/setpoint");
        assert_eq!(request.body, Some(b"42".to_vec()));
    }

    #[test]
    fn native_and_bridge_http_requests_match_for_supported_commands() {
        let driver = make_driver();
        for command in [
            Command {
                id: "cmd-parity-read".into(),
                source_device_id: None,
                target_device_id: "device-1".into(),
                intent: Intent::Read {
                    resource: "temp".into(),
                    options: RequestOptions::default(),
                },
                correlation: None,
            },
            Command {
                id: "cmd-parity-write".into(),
                source_device_id: None,
                target_device_id: "device-1".into(),
                intent: Intent::Write {
                    resource: "setpoint".into(),
                    payload: PayloadValue::String("42".to_string().into()),
                    options: RequestOptions::default(),
                },
                correlation: None,
            },
            Command {
                id: "cmd-parity-capture".into(),
                source_device_id: None,
                target_device_id: "device-1".into(),
                intent: Intent::Read {
                    resource: "temp".into(),
                    options: RequestOptions {
                        headers: Headers::from(vec![("X-Env".to_string(), "a".to_string())]),
                        extensions: Some(
                            HttpMeta {
                                method: Some("PUT".into()),
                                content_type: Some("text/plain".into()),
                                headers: Some(Headers::from(vec![
                                    ("X-Cap".to_string(), "1".to_string()),
                                    ("x-env".to_string(), "capture".to_string()),
                                ])),
                                ..HttpMeta::default()
                            }
                            .into_transport_meta(),
                        ),
                        ..RequestOptions::default()
                    },
                },
                correlation: None,
            },
        ] {
            let native = driver.native_request(command.clone()).unwrap();
            let bridge = driver.bridge_request(command).unwrap();
            assert_eq!(native, bridge);
        }
    }

    #[test]
    fn native_request_honors_http_capture_fields_with_options_winning() {
        let driver = make_driver();
        let command = Command {
            id: "cmd-native-capture".into(),
            source_device_id: None,
            target_device_id: "device-1".into(),
            intent: Intent::Read {
                resource: "temp".into(),
                options: RequestOptions {
                    headers: Headers::from(vec![("X-Env".to_string(), "a".to_string())]),
                    content_type: Some("application/json".into()),
                    extensions: Some(
                        HttpMeta {
                            method: Some("PUT".into()),
                            content_type: Some("text/plain".into()),
                            headers: Some(Headers::from(vec![
                                ("X-Cap".to_string(), "1".to_string()),
                                ("Content-Type".to_string(), "text/html".to_string()),
                            ])),
                            ..HttpMeta::default()
                        }
                        .into_transport_meta(),
                    ),
                    ..RequestOptions::default()
                },
            },
            correlation: None,
        };

        for request in [
            driver.native_request(command.clone()).unwrap(),
            driver.bridge_request(command).unwrap(),
        ] {
            assert_eq!(request.method, "PUT");
            assert_eq!(request.path, "/api/temp");
            assert_eq!(
                request.headers,
                vec![
                    ("Accept".to_string(), "application/json".to_string()),
                    ("X-Env".to_string(), "a".to_string()),
                    ("X-Cap".to_string(), "1".to_string()),
                    ("Content-Type".to_string(), "application/json".to_string()),
                ]
            );
        }
    }

    #[test]
    fn capture_method_and_path_reach_the_request_unless_options_override_them() {
        let driver = make_driver();
        let capture_only = Command {
            id: "cmd-capture-method-path".into(),
            source_device_id: None,
            target_device_id: "device-1".into(),
            intent: Intent::Read {
                resource: "temp".into(),
                options: RequestOptions {
                    extensions: Some(
                        HttpMeta {
                            method: Some("PUT".into()),
                            path: Some("/capture/override".into()),
                            ..HttpMeta::default()
                        }
                        .into_transport_meta(),
                    ),
                    ..RequestOptions::default()
                },
            },
            correlation: None,
        };
        for request in [
            driver.native_request(capture_only.clone()).unwrap(),
            driver.bridge_request(capture_only).unwrap(),
        ] {
            assert_eq!(request.method, "PUT");
            assert_eq!(request.path, "/capture/override");
        }

        let options_win = Command {
            id: "cmd-options-method-path".into(),
            source_device_id: None,
            target_device_id: "device-1".into(),
            intent: Intent::Read {
                resource: "temp".into(),
                options: RequestOptions {
                    method: Some("PATCH".into()),
                    path: Some("/options/override".into()),
                    extensions: Some(
                        HttpMeta {
                            method: Some("PUT".into()),
                            path: Some("/capture/override".into()),
                            ..HttpMeta::default()
                        }
                        .into_transport_meta(),
                    ),
                    ..RequestOptions::default()
                },
            },
            correlation: None,
        };
        for request in [
            driver.native_request(options_win.clone()).unwrap(),
            driver.bridge_request(options_win).unwrap(),
        ] {
            assert_eq!(request.method, "PATCH");
            assert_eq!(request.path, "/options/override");
        }
    }

    #[test]
    fn malformed_status_code_capture_is_rejected_identically_on_both_paths() {
        let driver = make_driver();
        let mut capture = TransportMeta::new(ProtocolId::HTTP);
        capture.set(KEY_METHOD, MetaValue::Str("PUT".into()));
        capture.set(KEY_STATUS_CODE, MetaValue::U64(u64::from(u16::MAX) + 1));
        let command = Command {
            id: "cmd-malformed-status".into(),
            source_device_id: None,
            target_device_id: "device-1".into(),
            intent: Intent::Read {
                resource: "temp".into(),
                options: RequestOptions {
                    extensions: Some(capture),
                    ..RequestOptions::default()
                },
            },
            correlation: None,
        };

        for error in [
            driver
                .native_request(command.clone())
                .expect_err("native path must reject a malformed capture"),
            driver
                .bridge_request(command)
                .expect_err("bridge path must reject a malformed capture"),
        ] {
            assert_eq!(
                error,
                HttpCommandConversionError::InvalidExtensionValue {
                    key: KEY_STATUS_CODE,
                    value: u64::from(u16::MAX) + 1,
                }
            );
        }
    }

    #[test]
    fn wrong_typed_known_key_errors_on_result_path_and_yields_none_on_view() {
        let mut capture = TransportMeta::new(ProtocolId::HTTP);
        capture.set(KEY_STATUS_CODE, MetaValue::Str("200".into()));

        assert_eq!(
            HttpMeta::try_from_transport_meta(capture.as_borrowed()),
            Err(HttpCommandConversionError::InvalidExtensionType {
                key: KEY_STATUS_CODE,
            })
        );
        assert_eq!(HttpMeta::view(&capture), None);
        assert_eq!(HttpMeta::from_transport_meta(capture), None);
    }

    #[test]
    fn http_requests_reject_foreign_extensions_capture_on_both_paths() {
        let driver = make_driver();
        let command = Command {
            id: "cmd-foreign-capture".into(),
            source_device_id: None,
            target_device_id: "device-1".into(),
            intent: Intent::Read {
                resource: "temp".into(),
                options: RequestOptions {
                    extensions: Some(TransportMeta::new(ProtocolId::MQTT)),
                    ..RequestOptions::default()
                },
            },
            correlation: None,
        };

        for error in [
            driver
                .native_request(command.clone())
                .expect_err("native path must reject a foreign capture"),
            driver
                .bridge_request(command)
                .expect_err("bridge path must reject a foreign capture"),
        ] {
            assert!(matches!(
                &error,
                HttpCommandConversionError::ForeignExtensions(protocol)
                    if *protocol == ProtocolId::MQTT
            ));
        }
    }

    #[test]
    fn native_http_roundtrip_decodes_completed_routed_result() {
        let command = Command {
            id: "cmd-roundtrip".into(),
            source_device_id: None,
            target_device_id: "device-1".into(),
            intent: Intent::Read {
                resource: "temp".into(),
                options: RequestOptions::default(),
            },
            correlation: None,
        };
        let (addr, rx, handle) = run_single_request_server(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nX-Test: ok\r\nContent-Length: 2\r\nConnection: close\r\n\r\n42",
        );
        let driver = make_http_driver_for_url(addr);
        let request = driver.native_request(command.clone()).unwrap();
        let response = runtime_stack::StackRuntime::default()
            .block_on(driver.execute(request))
            .expect("http request should succeed");
        let raw_request = rx.recv().expect("server should capture request");
        handle.join().expect("server should join");

        let request_text = String::from_utf8(raw_request).expect("request should be utf8");
        assert!(request_text.starts_with("GET /api/temp HTTP/1.1\r\n"));
        assert!(request_text.contains("\r\nAccept: application/json\r\n"));

        let decoded = HttpResponseDecoder::new(&driver.dvc, &command)
            .decode(&response)
            .expect("response should decode");
        let routed = RoutedMessage::try_from(decoded).expect("decoded response should route");

        match routed {
            RoutedMessage::Result(result) => {
                assert_eq!(result.result.state, DeliveryState::Completed);
                assert_eq!(
                    result.result.payload,
                    Some(PayloadValue::String("42".to_string().into()))
                );
                assert_eq!(
                    result.result.correlation,
                    Some(Correlation::new(
                        "cmd-roundtrip".to_string().into(),
                        Some(Address::Resource("temp".to_string().into())),
                    ))
                );
                let transport = result.transport.expect("expected http transport capture");
                assert_eq!(transport.protocol, ProtocolId::HTTP);
                let view = HttpMeta::view(&transport).expect("http view over http capture");
                assert_eq!(view.status_code, Some(200));
                assert_eq!(view.content_type.as_deref(), Some("text/plain"));
                assert_eq!(
                    transport.headers,
                    Some(Headers::from(vec![
                        (Cow::Borrowed("Content-Type"), Cow::Borrowed("text/plain")),
                        (Cow::Borrowed("X-Test"), Cow::Borrowed("ok")),
                        (Cow::Borrowed("Content-Length"), Cow::Borrowed("2")),
                        (Cow::Borrowed("Connection"), Cow::Borrowed("close")),
                    ]))
                );
            }
            other => panic!("expected routed result, got {other:?}"),
        }
    }

    #[test]
    fn native_http_roundtrip_decodes_rejected_routed_result() {
        let command = Command {
            id: "cmd-roundtrip-reject".into(),
            source_device_id: None,
            target_device_id: "device-1".into(),
            intent: Intent::Read {
                resource: "temp".into(),
                options: RequestOptions::default(),
            },
            correlation: None,
        };
        let (addr, _rx, handle) = run_single_request_server(
            b"HTTP/1.1 503 Service Unavailable\r\nContent-Type: text/plain\r\nContent-Length: 4\r\nConnection: close\r\n\r\nnope",
        );
        let driver = make_http_driver_for_url(addr);
        let request = driver.native_request(command.clone()).unwrap();
        let response = runtime_stack::StackRuntime::default()
            .block_on(driver.execute(request))
            .expect("http request should return response");
        handle.join().expect("server should join");

        let decoded = HttpResponseDecoder::new(&driver.dvc, &command)
            .decode(&response)
            .expect("response should decode");
        let routed = RoutedMessage::try_from(decoded).expect("decoded response should route");

        match routed {
            RoutedMessage::Result(result) => {
                assert_eq!(result.result.state, DeliveryState::Rejected);
                assert_eq!(result.result.error.as_deref(), Some("http status 503"));
            }
            other => panic!("expected routed result, got {other:?}"),
        }
    }
}
