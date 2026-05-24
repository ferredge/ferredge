use std::{
    io::{Read, Write},
    net::TcpListener,
    process::{Command as ProcessCommand, Stdio},
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver},
    },
    thread,
    time::{Duration, Instant},
};

use ferredge_bridge::{
    BridgeAdapter, BridgeCapability, BridgeCodec, BridgeCommand, BridgeMessage, BridgeOp,
    BridgePayload, BridgeResult, BridgeRoute, BridgeTransportMeta, MessagingAction,
    MessagingCapability, MessagingOp, RequestResponseAction, RequestResponseOp,
};
use ferredge_core::prelude::*;
use ferredge_proto_http::{
    HttpBridgeAdapter, HttpBridgeCodec, HttpDriver, attributes::HttpResourceAttributes,
};
use ferredge_proto_modbus::{
    ModbusBridgeAdapter, ModbusBridgeCodec, ModbusDriver,
    attributes::{ModbusRegisterKind, ModbusResourceAttributes, ModbusValueCodec},
};
use ferredge_proto_mqtt::{MqttBridgeAdapter, MqttBridgeCodec, MqttDriver, MqttWirePacket};
use ferredge_test_support::{
    diagslave::DiagslaveGuard, mosquitto::MosquittoGuard, process::require_command,
    runtime::block_on,
};

const EVENT_WAIT_TIMEOUT_SECS: u64 = 5;
const POLL_INTERVAL_MS: u64 = 25;
const SUBSCRIBER_STARTUP_MS: u64 = 200;

struct RecordingSink {
    events: Arc<Mutex<Vec<RoutedEvent>>>,
}

impl EventSink for RecordingSink {
    type Event = RoutedEvent;
    type Error = ();

    fn handle(&mut self, event: Self::Event) -> Result<(), Self::Error> {
        self.events.lock().expect("recording sink lock").push(event);
        Ok(())
    }
}

#[derive(Debug)]
struct CapturedHttpRequest {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

struct LocalHttpServer {
    addr: String,
    requests: Receiver<CapturedHttpRequest>,
    handle: Option<thread::JoinHandle<()>>,
}

enum HttpServerBehavior {
    Response {
        status_line: &'static str,
        headers: Vec<(&'static str, &'static str)>,
        body: Vec<u8>,
    },
    CloseAfterRead,
}

impl LocalHttpServer {
    fn start(response_body: Vec<u8>) -> Self {
        Self::start_with_behavior(HttpServerBehavior::Response {
            status_line: "HTTP/1.1 200 OK",
            headers: vec![
                ("Content-Type", "text/plain"),
                ("X-Response-Version", "2026-05"),
            ],
            body: response_body,
        })
    }

    fn start_and_close() -> Self {
        Self::start_with_behavior(HttpServerBehavior::CloseAfterRead)
    }

    fn start_with_behavior(behavior: HttpServerBehavior) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("http test listener should bind");
        let addr = listener
            .local_addr()
            .expect("http test listener should have address");
        let (tx, rx) = mpsc::channel();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("http test server should accept");
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .expect("http test server should set read timeout");

            let mut data = Vec::new();
            let mut buf = [0u8; 1024];
            let mut body_len = None;
            loop {
                match stream.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        data.extend_from_slice(&buf[..n]);
                        if body_len.is_none() {
                            body_len = parse_content_length(&data);
                        }
                        if let Some(header_end) = find_header_end(&data) {
                            let expected = header_end + 4 + body_len.unwrap_or(0);
                            if data.len() >= expected {
                                break;
                            }
                        }
                    }
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            || error.kind() == std::io::ErrorKind::TimedOut =>
                    {
                        break;
                    }
                    Err(error) => panic!("http test server read failed: {error}"),
                }
            }

            let request = parse_http_request(&data);
            tx.send(request)
                .expect("http test server should send captured request");

            match behavior {
                HttpServerBehavior::Response {
                    status_line,
                    headers,
                    body,
                } => {
                    let mut response =
                        format!("{status_line}\r\nContent-Length: {}\r\n", body.len());
                    for (key, value) in headers {
                        response.push_str(&format!("{key}: {value}\r\n"));
                    }
                    response.push_str("Connection: close\r\n\r\n");
                    stream
                        .write_all(response.as_bytes())
                        .expect("http test server should write response headers");
                    stream
                        .write_all(&body)
                        .expect("http test server should write response body");
                }
                HttpServerBehavior::CloseAfterRead => {}
            }
        });

        Self {
            addr: addr.to_string(),
            requests: rx,
            handle: Some(handle),
        }
    }

    fn endpoint(&self) -> String {
        self.addr.clone()
    }

    fn finish(mut self) -> CapturedHttpRequest {
        let request = self
            .requests
            .recv_timeout(Duration::from_secs(2))
            .expect("http test server should capture request");
        if let Some(handle) = self.handle.take() {
            handle.join().expect("http test server should join");
        }
        request
    }
}

fn find_header_end(data: &[u8]) -> Option<usize> {
    data.windows(4).position(|window| window == b"\r\n\r\n")
}

fn parse_content_length(data: &[u8]) -> Option<usize> {
    let header_end = find_header_end(data)?;
    let headers = String::from_utf8_lossy(&data[..header_end]);
    headers.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        if name.eq_ignore_ascii_case("content-length") {
            Some(
                value
                    .trim()
                    .parse::<usize>()
                    .expect("http content-length should parse"),
            )
        } else {
            None
        }
    })
}

fn parse_http_request(data: &[u8]) -> CapturedHttpRequest {
    let header_end = find_header_end(data).expect("http request should contain headers");
    let headers = String::from_utf8_lossy(&data[..header_end]);
    let mut lines = headers.lines();
    let request_line = lines
        .next()
        .expect("http request should have a request line");
    let mut parts = request_line.split_whitespace();
    let method = parts.next().expect("http method should exist").to_string();
    let path = parts.next().expect("http path should exist").to_string();
    let headers = lines
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            Some((name.trim().to_string(), value.trim().to_string()))
        })
        .collect();
    let body = data[header_end + 4..].to_vec();
    CapturedHttpRequest {
        method,
        path,
        headers,
        body,
    }
}

fn parse_http_response_body(data: &[u8]) -> &[u8] {
    match find_header_end(data) {
        Some(header_end) => &data[header_end + 4..],
        None => data,
    }
}

fn wait_for_event_payload(events: &Arc<Mutex<Vec<RoutedEvent>>>, payload: &[u8]) -> RoutedEvent {
    let deadline = Instant::now() + Duration::from_secs(EVENT_WAIT_TIMEOUT_SECS);
    loop {
        if let Some(event) = events
            .lock()
            .expect("events lock")
            .iter()
            .find(|event| payload_matches(&event.payload, payload))
            .cloned()
        {
            return event;
        }
        assert!(Instant::now() < deadline, "expected inbound MQTT event");
        thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));
    }
}

fn wait_for_event_topic(events: &Arc<Mutex<Vec<RoutedEvent>>>, topic: &str) -> RoutedEvent {
    let deadline = Instant::now() + Duration::from_secs(EVENT_WAIT_TIMEOUT_SECS);
    loop {
        if let Some(event) = events
            .lock()
            .expect("events lock")
            .iter()
            .find(|event| event.address == Address::Channel(topic.to_string()))
            .cloned()
        {
            return event;
        }
        assert!(Instant::now() < deadline, "expected inbound MQTT event");
        thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));
    }
}

fn payload_matches(actual: &PayloadValue, expected: &[u8]) -> bool {
    *actual == PayloadValue::from(expected)
        || std::str::from_utf8(expected)
            .map(|value| *actual == PayloadValue::String(value.to_string()))
            .unwrap_or(false)
}

fn make_http_driver(endpoint: String) -> HttpDriver {
    let mut resources = Map::default();
    resources.insert(
        "setpoint".to_string(),
        DeviceResource {
            name: "setpoint".to_string(),
            resource_attributes: HttpResourceAttributes {
                slug: "/interop/setpoint".to_string(),
                method: "POST".to_string(),
                headers: Some(vec![("Content-Type".to_string(), "text/plain".to_string())]),
            },
            unit: None,
            permission: Some(DeviceResourceAccessPermission::WRITE),
        },
    );

    HttpDriver::new(Device {
        id: "http-interop-device".to_string(),
        name: "HTTP Interop Device".to_string(),
        status: DeviceStatus::Online,
        endpoint: DeviceEndpoint::http(HttpEndpointConfig { url: endpoint }),
        metadata: None,
        max_connections: Some(1),
        resources,
        message_endpoints: Vec::new(),
    })
}

fn make_mqtt_driver(
    broker: String,
    device_id: &str,
    client_id: &str,
    supported_versions: Vec<MqttProtocolVersion>,
) -> MqttDriver {
    MqttDriver::new(Device {
        id: device_id.to_string(),
        name: "MQTT Interop Device".to_string(),
        status: DeviceStatus::Online,
        endpoint: DeviceEndpoint::mqtt(MqttEndpointConfig {
            broker,
            client_id: client_id.to_string(),
            auth: None,
            tls: None,
            keepalive_secs: Some(5),
            clean_start: true,
            session_expiry_secs: None,
            topic_prefix: None,
            connect_properties: MqttConnectProperties::default(),
            will: None,
            reconnect: BrokerReconnectConfig {
                enabled: true,
                initial_delay_ms: 100,
                max_delay_ms: 1_000,
                strategy: BackoffStrategy::Exponential,
                multiplier: 2,
                max_attempts: None,
                replay_subscriptions: true,
                queue_requests_while_disconnected: true,
                max_queued_requests: 64,
            },
            supported_versions,
        }),
        metadata: None,
        max_connections: Some(4),
        resources: Map::default(),
        message_endpoints: Vec::new(),
    })
}

fn make_modbus_driver(port: u16) -> ModbusDriver {
    let mut resources = Map::default();
    resources.insert(
        "holding_u16".to_string(),
        DeviceResource {
            name: "holding_u16".to_string(),
            resource_attributes: ModbusResourceAttributes {
                address: 100,
                register_kind: ModbusRegisterKind::HoldingRegister,
                codec: ModbusValueCodec::U16,
                quantity: None,
                description: None,
            },
            unit: None,
            permission: Some(
                DeviceResourceAccessPermission::READ | DeviceResourceAccessPermission::WRITE,
            ),
        },
    );
    resources.insert(
        "coil_bit".to_string(),
        DeviceResource {
            name: "coil_bit".to_string(),
            resource_attributes: ModbusResourceAttributes {
                address: 12,
                register_kind: ModbusRegisterKind::Coil,
                codec: ModbusValueCodec::Bool,
                quantity: None,
                description: None,
            },
            unit: None,
            permission: Some(
                DeviceResourceAccessPermission::READ | DeviceResourceAccessPermission::WRITE,
            ),
        },
    );
    resources.insert(
        "input_u16".to_string(),
        DeviceResource {
            name: "input_u16".to_string(),
            resource_attributes: ModbusResourceAttributes {
                address: 200,
                register_kind: ModbusRegisterKind::InputRegister,
                codec: ModbusValueCodec::U16,
                quantity: None,
                description: None,
            },
            unit: None,
            permission: Some(DeviceResourceAccessPermission::READ),
        },
    );

    ModbusDriver::new(Device {
        id: "modbus-interop-device".to_string(),
        name: "Modbus Interop Device".to_string(),
        status: DeviceStatus::Online,
        endpoint: DeviceEndpoint::modbus_tcp(ModbusTcpEndpointConfig {
            addr: "127.0.0.1".to_string(),
            port,
            options: ModbusClientOptions::default(),
        }),
        metadata: None,
        max_connections: Some(1),
        resources,
        message_endpoints: Vec::new(),
    })
}

fn http_write_command(payload: &str) -> Command {
    Command {
        id: "http-write-1".to_string(),
        source_device_id: Some("interop-source".to_string()),
        target_device_id: "http-interop-device".to_string(),
        intent: Intent::Write {
            resource: "setpoint".to_string(),
            payload: PayloadValue::String(payload.to_string()),
            options: RequestOptions {
                headers: vec![
                    ("X-Request-Version".to_string(), "2026-05".to_string()),
                    ("X-Trace-Id".to_string(), "trace-http-17".to_string()),
                ],
                content_type: Some("text/plain".to_string()),
                method: None,
                path: None,
            },
        },
        correlation: Some(Correlation {
            request_id: "http-corr-1".to_string(),
            reply_to: Some(Address::Channel("ferredge/http/reply".to_string())),
        }),
    }
}

fn mqtt_publish_command(
    target_device_id: &str,
    topic: &str,
    payload: PayloadValue,
    options: BrokerMessageOptions,
) -> Command {
    Command {
        id: format!("mqtt-publish-{topic}"),
        source_device_id: Some("interop-source".to_string()),
        target_device_id: target_device_id.to_string(),
        intent: Intent::Send {
            channel: BrokerAddress {
                name: topic.to_string(),
                kind: Some(BrokerChannelKind::Topic),
            },
            payload,
            options,
        },
        correlation: None,
    }
}

fn mqtt_subscribe_command(target_device_id: &str, topic: &str) -> Command {
    Command {
        id: format!("mqtt-subscribe-{topic}"),
        source_device_id: None,
        target_device_id: target_device_id.to_string(),
        intent: Intent::Subscribe {
            channel: BrokerAddress {
                name: topic.to_string(),
                kind: Some(BrokerChannelKind::Topic),
            },
            options: BrokerSubscriptionOptions::default(),
        },
        correlation: None,
    }
}

fn modbus_write_command(value: u16) -> Command {
    modbus_write_resource_command(
        "modbus-write-1",
        "holding_u16",
        PayloadValue::U64(value.into()),
    )
}

fn modbus_write_resource_command(id: &str, resource: &str, payload: PayloadValue) -> Command {
    Command {
        id: id.to_string(),
        source_device_id: Some("interop-source".to_string()),
        target_device_id: "modbus-interop-device".to_string(),
        intent: Intent::Write {
            resource: resource.to_string(),
            payload,
            options: RequestOptions::default(),
        },
        correlation: Some(Correlation {
            request_id: "modbus-corr-1".to_string(),
            reply_to: None,
        }),
    }
}

fn modbus_read_command() -> Command {
    modbus_read_resource_command("modbus-read-1", "holding_u16")
}

fn modbus_read_resource_command(id: &str, resource: &str) -> Command {
    Command {
        id: id.to_string(),
        source_device_id: Some("interop-source".to_string()),
        target_device_id: "modbus-interop-device".to_string(),
        intent: Intent::Read {
            resource: resource.to_string(),
            options: RequestOptions::default(),
        },
        correlation: Some(Correlation {
            request_id: "modbus-corr-2".to_string(),
            reply_to: None,
        }),
    }
}

fn assert_v5_publish(packet: &MqttWirePacket) {
    assert!(
        matches!(packet, MqttWirePacket::V5Publish(_)),
        "expected MQTT v5 publish packet"
    );
}

fn assert_v3_publish(packet: &MqttWirePacket) {
    assert!(
        matches!(packet, MqttWirePacket::V3Publish(_)),
        "expected MQTT v3.1.1 publish packet"
    );
}

fn modbus_request_from_command(
    driver: &ModbusDriver,
    command: &Command,
) -> ferredge_proto_modbus::ModbusRequest {
    let message = ModbusBridgeAdapter::new(&driver.dvc)
        .command_to_bridge(command.clone())
        .expect("modbus command should plan");
    match &command.intent {
        Intent::Read { resource, .. } | Intent::Write { resource, .. } => {
            assert!(
                driver.dvc.resources.contains_key(resource),
                "modbus resource should exist"
            );
        }
        other => panic!("expected modbus read/write command, got {other:?}"),
    };
    ModbusBridgeCodec::new(&driver.dvc)
        .encode(&message)
        .expect("modbus command should encode")
}

fn mqtt_source_ref(device_id: &str) -> EndpointRef {
    EndpointRef {
        device_id: device_id.to_string(),
        protocol: DeviceProtocol::MQTT,
    }
}

fn http_source_ref() -> EndpointRef {
    EndpointRef {
        device_id: "http-interop-device".to_string(),
        protocol: DeviceProtocol::HTTP,
    }
}

fn modbus_source_ref() -> EndpointRef {
    EndpointRef {
        device_id: "modbus-interop-device".to_string(),
        protocol: DeviceProtocol::Modbus,
    }
}

fn routed_result(source: EndpointRef, command_id: &str, payload: PayloadValue) -> RoutedResult {
    RoutedResult {
        source,
        result: CommandResult {
            command_id: command_id.to_string(),
            device_id: "interop-device".to_string(),
            state: DeliveryState::Completed,
            payload: Some(payload),
            error: None,
            correlation: None,
        },
        transport: None,
    }
}

fn routed_rejected_result(source: EndpointRef, command_id: &str, error: &str) -> RoutedResult {
    RoutedResult {
        source,
        result: CommandResult {
            command_id: command_id.to_string(),
            device_id: "interop-device".to_string(),
            state: DeliveryState::Rejected,
            payload: None,
            error: Some(error.to_string()),
            correlation: None,
        },
        transport: None,
    }
}

#[test]
fn http_to_mqtt_v5_bridge_roundtrip() {
    let server = LocalHttpServer::start(b"41".to_vec());
    let http_driver = make_http_driver(server.endpoint());
    let http_command = http_write_command("17");

    let http_bridge = HttpBridgeAdapter
        .command_to_bridge(http_command)
        .expect("http command should plan");
    match &http_bridge {
        BridgeMessage::Command(command) => {
            assert!(matches!(
                command.route,
                BridgeRoute::RequestResponse { ref resource, .. } if resource == "setpoint"
            ));
            assert!(matches!(
                command.operation,
                BridgeOp::RequestResponse(RequestResponseOp {
                    action: RequestResponseAction::Write
                })
            ));
            let BridgeTransportMeta::Http(meta) =
                command.transport.as_ref().expect("http transport metadata expected")
            else {
                panic!("expected http transport metadata");
            };
            assert_eq!(meta.content_type.as_deref(), Some("text/plain"));
            let headers = command.headers.as_ref().expect("http headers expected");
            assert!(headers
                .iter_http_headers()
                .any(|header| header.key == "X-Request-Version" && header.value == "2026-05"));
            assert!(headers
                .iter_http_headers()
                .any(|header| header.key == "X-Trace-Id" && header.value == "trace-http-17"));
        }
        other => panic!("expected bridge command, got {other:?}"),
    }

    let http_request = HttpBridgeCodec::new(&http_driver.dvc)
        .encode(&http_bridge)
        .expect("http bridge should encode");
    assert_eq!(http_request.method, "POST");
    assert_eq!(http_request.path, "/interop/setpoint");
    assert_eq!(http_request.body, Some(b"17".to_vec()));
    assert!(http_request
        .headers
        .contains(&("Content-Type".to_string(), "text/plain".to_string())));
    assert!(http_request
        .headers
        .contains(&("X-Request-Version".to_string(), "2026-05".to_string())));
    assert!(http_request
        .headers
        .contains(&("X-Trace-Id".to_string(), "trace-http-17".to_string())));

    let http_response =
        block_on(http_driver.execute(http_request.clone())).expect("http driver should execute");
    let captured = server.finish();
    assert_eq!(captured.method, "POST");
    assert_eq!(captured.path, "/interop/setpoint");
    assert_eq!(captured.body, b"17");
    assert!(captured
        .headers
        .contains(&("Content-Type".to_string(), "text/plain".to_string())));
    assert!(captured
        .headers
        .contains(&("X-Request-Version".to_string(), "2026-05".to_string())));
    assert!(captured
        .headers
        .contains(&("X-Trace-Id".to_string(), "trace-http-17".to_string())));
    assert_eq!(parse_http_response_body(&http_response.body), b"41");

    let broker = MosquittoGuard::start();
    let publisher = make_mqtt_driver(
        broker.broker_url(),
        "mqtt-http-v5-publisher",
        "mqtt-http-v5-publisher",
        vec![MqttProtocolVersion::V5_0],
    );
    let subscriber = make_mqtt_driver(
        broker.broker_url(),
        "mqtt-http-v5-subscriber",
        "mqtt-http-v5-subscriber",
        vec![MqttProtocolVersion::V5_0],
    );
    let events = Arc::new(Mutex::new(Vec::new()));
    let topic = "ferredge/interop/http-mqtt-v5";

    block_on(subscriber.start()).expect("subscriber should connect");
    let subscribe_command = mqtt_subscribe_command(&subscriber.dvc.id, topic);
    let subscribe_message = MqttBridgeAdapter
        .command_to_bridge(subscribe_command)
        .expect("mqtt subscribe should plan");
    let subscribe_packet = MqttBridgeCodec::new(&subscriber.dvc)
        .encode(&subscribe_message)
        .expect("mqtt subscribe should encode");
    block_on(subscriber.subscribe(
        subscribe_packet,
        RecordingSink {
            events: Arc::clone(&events),
        },
    ))
    .expect("subscriber should subscribe");
    block_on(subscriber.start_listening(RecordingSink {
        events: Arc::clone(&events),
    }))
    .expect("subscriber listener should start");
    thread::sleep(Duration::from_millis(SUBSCRIBER_STARTUP_MS));

    let mqtt_command = mqtt_publish_command(
        &publisher.dvc.id,
        topic,
        PayloadValue::from(parse_http_response_body(&http_response.body)),
        BrokerMessageOptions {
            correlation_id: Some("corr-http-mqtt-v5".to_string()),
            reply_to: Some("ferredge/http/reply".to_string()),
            protocol: Some(BrokerMessageProtocolOptions::Mqtt(MqttMessageOptions {
                content_type: Some("text/plain".to_string()),
                user_properties: vec![("x-origin".to_string(), "http".to_string())],
                ..MqttMessageOptions::default()
            })),
            ..BrokerMessageOptions::default()
        },
    );
    let mqtt_message = MqttBridgeAdapter
        .command_to_bridge(mqtt_command)
        .expect("mqtt publish should plan");
    let mqtt_packet = MqttBridgeCodec::new(&publisher.dvc)
        .encode(&mqtt_message)
        .expect("mqtt publish should encode");
    assert_v5_publish(&mqtt_packet.packet);

    block_on(publisher.start()).expect("publisher should connect");
    block_on(publisher.publish(mqtt_packet)).expect("publisher should publish");

    let event = wait_for_event_payload(&events, b"41");
    assert_eq!(event.address, Address::Channel(topic.to_string()));
    assert_eq!(event.payload, PayloadValue::from(b"41".as_slice()));
    assert_eq!(
        event.correlation,
        Some(Correlation {
            request_id: "corr-http-mqtt-v5".to_string(),
            reply_to: Some(Address::Channel("ferredge/http/reply".to_string())),
        })
    );
    match &event.transport {
        Some(TransportMeta::Mqtt(meta)) => {
            assert_eq!(meta.content_type.as_deref(), Some("text/plain"));
            assert_eq!(meta.response_topic.as_deref(), Some("ferredge/http/reply"));
            assert_eq!(meta.correlation_data.as_deref(), Some("corr-http-mqtt-v5"));
            assert!(
                meta.user_properties
                    .contains(&("x-origin".to_string(), "http".to_string()))
            );
        }
        other => panic!("expected MQTT transport metadata, got {other:?}"),
    }

    let inbound_bridge = MqttBridgeAdapter
        .event_to_bridge(event)
        .expect("mqtt event should bridge");
    match inbound_bridge {
        BridgeMessage::Event(bridge_event) => {
            assert!(matches!(
                bridge_event.route,
                BridgeRoute::Messaging { topic: ref route_topic } if route_topic == topic
            ));
            assert_eq!(
                bridge_event
                    .correlation
                    .as_ref()
                    .map(|value| value.request_id.as_ref()),
                Some("corr-http-mqtt-v5")
            );
        }
        other => panic!("expected bridge event, got {other:?}"),
    }

    let http_result = RoutedResult {
        source: http_source_ref(),
        result: CommandResult {
            command_id: "http-write-1".to_string(),
            device_id: "http-interop-device".to_string(),
            state: DeliveryState::Completed,
            payload: Some(PayloadValue::from(parse_http_response_body(&http_response.body))),
            error: None,
            correlation: None,
        },
        transport: Some(TransportMeta::Http(HttpMeta {
            method: Some("POST".to_string()),
            path: Some("/interop/setpoint".to_string()),
            status_code: Some(http_response.status_code),
            headers: http_response.headers.clone(),
        })),
    };
    let http_result_bridge = HttpBridgeAdapter
        .result_to_bridge(http_result)
        .expect("http result should bridge");
    let (result_transport, result_headers) = match &http_result_bridge {
        BridgeMessage::Result(BridgeResult::Success {
            transport, headers, ..
        }) => {
            let BridgeTransportMeta::Http(meta) =
                transport.as_ref().expect("http result transport expected")
            else {
                panic!("expected http result transport metadata");
            };
            assert_eq!(meta.status_code, Some(200));
            assert_eq!(meta.content_type.as_deref(), Some("text/plain"));
            let headers = headers.as_ref().expect("http result headers expected");
            assert!(headers
                .iter_http_headers()
                .any(|header| header.key == "X-Response-Version" && header.value == "2026-05"));
            assert!(headers
                .iter_http_headers()
                .any(|header| header.key == "Content-Length" && header.value == "2"));
            (transport.clone(), headers.clone())
        }
        other => panic!("expected bridged http success result, got {other:?}"),
    };

    let projected_message = BridgeMessage::Command(BridgeCommand {
        id: "http-result-mqtt-project".to_string(),
        source_device_id: Some("http-interop-device".to_string()),
        target_device_id: publisher.dvc.id.clone(),
        capability: BridgeCapability::Messaging(MessagingCapability {
            binary_payloads: true,
        }),
        operation: BridgeOp::Messaging(MessagingOp {
            action: MessagingAction::Publish,
        }),
        payload: Some(BridgePayload::Binary(b"41".to_vec())),
        route: BridgeRoute::Messaging {
            topic: topic.into(),
        },
        transport: result_transport,
        headers: Some(result_headers),
        correlation: None,
    });
    let projected_packet = MqttBridgeCodec::new(&publisher.dvc)
        .encode(&projected_message)
        .expect("http result bridge should project into mqtt packet");
    match projected_packet.packet {
        MqttWirePacket::V5Publish(packet) => {
            let props_debug = format!("{:?}", packet.props());
            assert!(props_debug.contains("text/plain"));
            assert!(props_debug.contains("ferredge-http-status-code"));
            assert!(props_debug.contains("200"));
            assert!(props_debug.contains("ferredge-http-path"));
            assert!(props_debug.contains("/interop/setpoint"));
            assert!(props_debug.contains("X-Response-Version"));
            assert!(props_debug.contains("2026-05"));
            assert!(props_debug.contains("Content-Length"));
            assert!(props_debug.contains("\"2\"") || props_debug.contains(" 2"));
        }
        other => panic!("expected projected v5 publish packet, got {other:?}"),
    }

    block_on(publisher.stop()).expect("publisher should stop");
    block_on(subscriber.stop()).expect("subscriber should stop");
}

#[test]
fn http_to_mqtt_v3_bridge_roundtrip() {
    let server = LocalHttpServer::start(b"19".to_vec());
    let http_driver = make_http_driver(server.endpoint());
    let http_command = http_write_command("11");

    let http_bridge = HttpBridgeAdapter
        .command_to_bridge(http_command)
        .expect("http command should plan");
    let http_request = HttpBridgeCodec::new(&http_driver.dvc)
        .encode(&http_bridge)
        .expect("http bridge should encode");
    let http_response =
        block_on(http_driver.execute(http_request)).expect("http driver should execute");
    let captured = server.finish();
    assert_eq!(captured.body, b"11");

    let broker = MosquittoGuard::start();
    let publisher = make_mqtt_driver(
        broker.broker_url(),
        "mqtt-http-v3-publisher",
        "mqtt-http-v3-publisher",
        vec![MqttProtocolVersion::V3_1_1],
    );
    let topic = "ferredge/interop/http-mqtt-v3";

    let mqtt_command = mqtt_publish_command(
        &publisher.dvc.id,
        topic,
        PayloadValue::from(parse_http_response_body(&http_response.body)),
        BrokerMessageOptions::default(),
    );
    let mqtt_message = MqttBridgeAdapter
        .command_to_bridge(mqtt_command)
        .expect("mqtt publish should plan");
    let mqtt_packet = MqttBridgeCodec::new(&publisher.dvc)
        .encode(&mqtt_message)
        .expect("mqtt publish should encode");
    assert_v3_publish(&mqtt_packet.packet);

    require_command("mosquitto_pub");
    require_command("mosquitto_sub");
    let subscriber = ProcessCommand::new("mosquitto_sub")
        .args([
            "-h",
            broker.host(),
            "-p",
            broker.port_string().as_str(),
            "-V",
            "mqttv311",
            "-t",
            topic,
            "-C",
            "1",
            "-W",
            "5",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("mosquitto_sub should spawn");
    thread::sleep(Duration::from_millis(SUBSCRIBER_STARTUP_MS));
    let publish_status = ProcessCommand::new("mosquitto_pub")
        .args([
            "-h",
            broker.host(),
            "-p",
            broker.port_string().as_str(),
            "-V",
            "mqttv311",
            "-t",
            topic,
            "-m",
            "19",
        ])
        .status()
        .expect("mosquitto_pub should run");
    assert!(publish_status.success());
    let output = subscriber
        .wait_with_output()
        .expect("mosquitto_sub should capture v3 payload");
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "19");

    let inbound_event = RoutedEvent {
        source: mqtt_source_ref("mqtt-http-v3-publisher"),
        address: Address::Channel(topic.to_string()),
        payload: PayloadValue::String("19".to_string()),
        correlation: None,
        transport: Some(TransportMeta::Mqtt(MqttMeta {
            topic: topic.to_string(),
            qos: 0,
            retain: false,
            duplicate: false,
            packet_id: None,
            content_type: None,
            payload_format: None,
            message_expiry_interval_secs: None,
            response_topic: None,
            correlation_data: None,
            correlation_data_bytes: None,
            topic_alias: None,
            subscription_identifiers: Vec::new(),
            user_properties: Vec::new(),
            reason_codes: Vec::new(),
            reason_string: None,
        })),
    };
    let inbound_bridge = MqttBridgeAdapter
        .event_to_bridge(inbound_event)
        .expect("mqtt event should bridge");
    assert!(matches!(inbound_bridge, BridgeMessage::Event(_)));
}

#[test]
fn mqtt_v5_to_modbus_bridge_roundtrip() {
    let broker = MosquittoGuard::start();
    let publisher = make_mqtt_driver(
        broker.broker_url(),
        "mqtt-modbus-v5-publisher",
        "mqtt-modbus-v5-publisher",
        vec![MqttProtocolVersion::V5_0],
    );
    let subscriber = make_mqtt_driver(
        broker.broker_url(),
        "mqtt-modbus-v5-subscriber",
        "mqtt-modbus-v5-subscriber",
        vec![MqttProtocolVersion::V5_0],
    );
    let events = Arc::new(Mutex::new(Vec::new()));
    let topic = "ferredge/interop/mqtt-modbus-v5";

    block_on(subscriber.start()).expect("subscriber should connect");
    let subscribe_command = mqtt_subscribe_command(&subscriber.dvc.id, topic);
    let subscribe_message = MqttBridgeAdapter
        .command_to_bridge(subscribe_command)
        .expect("mqtt subscribe should plan");
    let subscribe_packet = MqttBridgeCodec::new(&subscriber.dvc)
        .encode(&subscribe_message)
        .expect("mqtt subscribe should encode");
    block_on(subscriber.subscribe(
        subscribe_packet,
        RecordingSink {
            events: Arc::clone(&events),
        },
    ))
    .expect("subscriber should subscribe");
    block_on(subscriber.start_listening(RecordingSink {
        events: Arc::clone(&events),
    }))
    .expect("subscriber listener should start");
    thread::sleep(Duration::from_millis(SUBSCRIBER_STARTUP_MS));

    let mqtt_command = mqtt_publish_command(
        &publisher.dvc.id,
        topic,
        PayloadValue::String("77".to_string()),
        BrokerMessageOptions {
            protocol: Some(BrokerMessageProtocolOptions::Mqtt(MqttMessageOptions {
                content_type: Some("text/plain".to_string()),
                user_properties: vec![("x-origin".to_string(), "mqtt-v5".to_string())],
                ..MqttMessageOptions::default()
            })),
            ..BrokerMessageOptions::default()
        },
    );
    let mqtt_message = MqttBridgeAdapter
        .command_to_bridge(mqtt_command)
        .expect("mqtt publish should plan");
    let mqtt_packet = MqttBridgeCodec::new(&publisher.dvc)
        .encode(&mqtt_message)
        .expect("mqtt publish should encode");
    assert_v5_publish(&mqtt_packet.packet);

    block_on(publisher.start()).expect("publisher should connect");
    block_on(publisher.publish(mqtt_packet)).expect("publisher should publish");

    let event = wait_for_event_payload(&events, b"77");
    match &event.transport {
        Some(TransportMeta::Mqtt(meta)) => {
            assert_eq!(meta.content_type.as_deref(), Some("text/plain"));
            assert!(
                meta.user_properties
                    .contains(&("x-origin".to_string(), "mqtt-v5".to_string()))
            );
        }
        other => panic!("expected MQTT transport metadata, got {other:?}"),
    }
    let mqtt_bridge = MqttBridgeAdapter
        .event_to_bridge(event)
        .expect("mqtt event should bridge");
    assert!(matches!(mqtt_bridge, BridgeMessage::Event(_)));

    let slave = DiagslaveGuard::start("tcp");
    let modbus_driver = make_modbus_driver(slave.port());
    let modbus_command = modbus_write_command(77);
    let modbus_bridge = ModbusBridgeAdapter::new(&modbus_driver.dvc)
        .command_to_bridge(modbus_command.clone())
        .expect("modbus command should plan");
    match &modbus_bridge {
        BridgeMessage::Command(command) => {
            let BridgeRoute::AddressedAccess {
                ref resource,
                ref access,
                node_id,
            } = command.route
            else {
                panic!("expected register route");
            };
            assert_eq!(resource, "holding_u16");
            assert_eq!(access.address, 100);
            assert_eq!(access.domain.as_ref(), "holding-register");
            assert_eq!(node_id, Some(1));
        }
        other => panic!("expected bridge command, got {other:?}"),
    }
    let modbus_request = modbus_request_from_command(&modbus_driver, &modbus_command);
    assert!(modbus_request.is_write);

    block_on(modbus_driver.execute_command(modbus_command)).expect("modbus write should succeed");
    let read_response = block_on(modbus_driver.execute_command(modbus_read_command()))
        .expect("modbus read should succeed");
    assert_eq!(read_response.payload, PayloadValue::U64(77));

    let bridged_result = routed_result(
        modbus_source_ref(),
        "modbus-read-1",
        read_response.payload.clone(),
    );
    let result_bridge = ModbusBridgeAdapter::new(&modbus_driver.dvc)
        .result_to_bridge(bridged_result)
        .expect("modbus result should bridge");
    assert!(matches!(
        result_bridge,
        BridgeMessage::Result(BridgeResult::Success { .. })
    ));

    block_on(publisher.stop()).expect("publisher should stop");
    block_on(subscriber.stop()).expect("subscriber should stop");
}

#[test]
fn mqtt_v3_to_modbus_bridge_roundtrip() {
    let broker = MosquittoGuard::start();
    let publisher = make_mqtt_driver(
        broker.broker_url(),
        "mqtt-modbus-v3-publisher",
        "mqtt-modbus-v3-publisher",
        vec![MqttProtocolVersion::V3_1_1],
    );
    let topic = "ferredge/interop/mqtt-modbus-v3";

    let mqtt_command = mqtt_publish_command(
        &publisher.dvc.id,
        topic,
        PayloadValue::String("21".to_string()),
        BrokerMessageOptions::default(),
    );
    let mqtt_message = MqttBridgeAdapter
        .command_to_bridge(mqtt_command)
        .expect("mqtt publish should plan");
    let mqtt_packet = MqttBridgeCodec::new(&publisher.dvc)
        .encode(&mqtt_message)
        .expect("mqtt publish should encode");
    assert_v3_publish(&mqtt_packet.packet);

    require_command("mosquitto_pub");
    require_command("mosquitto_sub");
    let subscriber = ProcessCommand::new("mosquitto_sub")
        .args([
            "-h",
            broker.host(),
            "-p",
            broker.port_string().as_str(),
            "-V",
            "mqttv311",
            "-t",
            topic,
            "-C",
            "1",
            "-W",
            "5",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("mosquitto_sub should spawn");
    thread::sleep(Duration::from_millis(SUBSCRIBER_STARTUP_MS));
    let publish_status = ProcessCommand::new("mosquitto_pub")
        .args([
            "-h",
            broker.host(),
            "-p",
            broker.port_string().as_str(),
            "-V",
            "mqttv311",
            "-t",
            topic,
            "-m",
            "21",
        ])
        .status()
        .expect("mosquitto_pub should run");
    assert!(publish_status.success());
    let output = subscriber
        .wait_with_output()
        .expect("mosquitto_sub should capture v3 payload");
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "21");

    let slave = DiagslaveGuard::start("tcp");
    let modbus_driver = make_modbus_driver(slave.port());
    let modbus_command = modbus_write_command(21);
    let modbus_request = modbus_request_from_command(&modbus_driver, &modbus_command);
    assert!(modbus_request.is_write);

    block_on(modbus_driver.execute_command(modbus_command)).expect("modbus write should succeed");
    let read_response = block_on(modbus_driver.execute_command(modbus_read_command()))
        .expect("modbus read should succeed");
    assert_eq!(read_response.payload, PayloadValue::U64(21));

    let bridged_result = routed_result(
        modbus_source_ref(),
        "modbus-read-1",
        read_response.payload.clone(),
    );
    let result_bridge = ModbusBridgeAdapter::new(&modbus_driver.dvc)
        .result_to_bridge(bridged_result)
        .expect("modbus result should bridge");
    assert!(matches!(
        result_bridge,
        BridgeMessage::Result(BridgeResult::Success { .. })
    ));
}

#[test]
fn http_to_modbus_bridge_roundtrip() {
    let server = LocalHttpServer::start(b"55".to_vec());
    let http_driver = make_http_driver(server.endpoint());
    let http_command = http_write_command("23");

    let http_bridge = HttpBridgeAdapter
        .command_to_bridge(http_command)
        .expect("http command should plan");
    let http_request = HttpBridgeCodec::new(&http_driver.dvc)
        .encode(&http_bridge)
        .expect("http bridge should encode");
    let response = block_on(http_driver.execute(http_request)).expect("http driver should execute");
    let captured = server.finish();
    assert_eq!(captured.body, b"23");

    let http_result = routed_result(
        http_source_ref(),
        "http-write-1",
        PayloadValue::from(parse_http_response_body(&response.body)),
    );
    let http_result_bridge = HttpBridgeAdapter
        .result_to_bridge(http_result)
        .expect("http result should bridge");
    match &http_result_bridge {
        BridgeMessage::Result(BridgeResult::Success {
            command_id,
            payload,
            ..
        }) => {
            assert_eq!(command_id, "http-write-1");
            assert_eq!(payload, &Some(BridgePayload::Binary(b"55".to_vec())));
        }
        other => panic!("expected bridge result, got {other:?}"),
    }

    let slave = DiagslaveGuard::start("tcp");
    let modbus_driver = make_modbus_driver(slave.port());
    let modbus_command = modbus_write_command(55);
    let modbus_bridge = ModbusBridgeAdapter::new(&modbus_driver.dvc)
        .command_to_bridge(modbus_command.clone())
        .expect("modbus command should plan");
    assert!(matches!(modbus_bridge, BridgeMessage::Command(_)));
    let modbus_request = modbus_request_from_command(&modbus_driver, &modbus_command);
    assert!(modbus_request.is_write);

    block_on(modbus_driver.execute_command(modbus_command)).expect("modbus write should succeed");
    let read_response = block_on(modbus_driver.execute_command(modbus_read_command()))
        .expect("modbus read should succeed");
    assert_eq!(read_response.payload, PayloadValue::U64(55));
}

#[test]
fn mqtt_v5_to_modbus_coil_bridge_roundtrip() {
    let broker = MosquittoGuard::start();
    let publisher = make_mqtt_driver(
        broker.broker_url(),
        "mqtt-modbus-coil-publisher",
        "mqtt-modbus-coil-publisher",
        vec![MqttProtocolVersion::V5_0],
    );
    let subscriber = make_mqtt_driver(
        broker.broker_url(),
        "mqtt-modbus-coil-subscriber",
        "mqtt-modbus-coil-subscriber",
        vec![MqttProtocolVersion::V5_0],
    );
    let events = Arc::new(Mutex::new(Vec::new()));
    let topic = "ferredge/interop/mqtt-modbus-coil";

    block_on(subscriber.start()).expect("subscriber should connect");
    let subscribe_command = mqtt_subscribe_command(&subscriber.dvc.id, topic);
    let subscribe_message = MqttBridgeAdapter
        .command_to_bridge(subscribe_command)
        .expect("mqtt subscribe should plan");
    let subscribe_packet = MqttBridgeCodec::new(&subscriber.dvc)
        .encode(&subscribe_message)
        .expect("mqtt subscribe should encode");
    block_on(subscriber.subscribe(
        subscribe_packet,
        RecordingSink {
            events: Arc::clone(&events),
        },
    ))
    .expect("subscriber should subscribe");
    block_on(subscriber.start_listening(RecordingSink {
        events: Arc::clone(&events),
    }))
    .expect("subscriber listener should start");
    thread::sleep(Duration::from_millis(SUBSCRIBER_STARTUP_MS));

    let mqtt_command = mqtt_publish_command(
        &publisher.dvc.id,
        topic,
        PayloadValue::Bool(true),
        BrokerMessageOptions::default(),
    );
    let mqtt_message = MqttBridgeAdapter
        .command_to_bridge(mqtt_command)
        .expect("mqtt publish should plan");
    let mqtt_packet = MqttBridgeCodec::new(&publisher.dvc)
        .encode(&mqtt_message)
        .expect("mqtt publish should encode");
    block_on(publisher.start()).expect("publisher should connect");
    block_on(publisher.publish(mqtt_packet)).expect("publisher should publish");

    let event = wait_for_event_topic(&events, topic);
    let mqtt_bridge = MqttBridgeAdapter
        .event_to_bridge(event.clone())
        .expect("mqtt event should bridge");
    assert!(matches!(mqtt_bridge, BridgeMessage::Event(_)));
    assert!(matches!(event.payload, PayloadValue::Bytes(_)));

    let slave = DiagslaveGuard::start("tcp");
    let modbus_driver = make_modbus_driver(slave.port());
    let modbus_command =
        modbus_write_resource_command("modbus-coil-write-1", "coil_bit", PayloadValue::Bool(true));
    let modbus_request = modbus_request_from_command(&modbus_driver, &modbus_command);
    assert!(modbus_request.is_write);

    block_on(modbus_driver.execute_command(modbus_command)).expect("coil write should succeed");
    let read_response = block_on(modbus_driver.execute_command(modbus_read_resource_command(
        "modbus-coil-read-1",
        "coil_bit",
    )))
    .expect("coil read should succeed");
    assert_eq!(read_response.payload, PayloadValue::Bool(true));

    block_on(publisher.stop()).expect("publisher should stop");
    block_on(subscriber.stop()).expect("subscriber should stop");
}

#[test]
fn bridge_to_modbus_input_register_write_fails_during_conversion() {
    let slave = DiagslaveGuard::start("tcp");
    let modbus_driver = make_modbus_driver(slave.port());
    let modbus_command =
        modbus_write_resource_command("modbus-input-write-1", "input_u16", PayloadValue::U64(7));

    let error = ModbusBridgeAdapter::new(&modbus_driver.dvc)
        .command_to_bridge(modbus_command)
        .and_then(|message| ModbusBridgeCodec::new(&modbus_driver.dvc).encode(&message))
        .unwrap_err();

    assert_eq!(
        error,
        ferredge_proto_modbus::ModbusCommandConversionError::UnsupportedWrite(
            "input_u16".to_string()
        )
    );
}

#[test]
fn http_bridge_surfaces_transport_failure_before_result_bridge() {
    let server = LocalHttpServer::start_and_close();
    let http_driver = make_http_driver(server.endpoint());
    let http_command = http_write_command("23");

    let http_bridge = HttpBridgeAdapter
        .command_to_bridge(http_command)
        .expect("http command should plan");
    let http_request = HttpBridgeCodec::new(&http_driver.dvc)
        .encode(&http_bridge)
        .expect("http bridge should encode");
    let error = block_on(http_driver.execute(http_request)).unwrap_err();
    let captured = server.finish();

    assert_eq!(captured.body, b"23");
    assert!(error.contains("invalid HTTP response"));
}

#[test]
fn rejected_routed_result_bridges_as_failure_outcome() {
    let rejected_result =
        routed_rejected_result(http_source_ref(), "http-write-1", "upstream rejected");
    let result_bridge = HttpBridgeAdapter
        .result_to_bridge(rejected_result)
        .expect("http failure result should bridge");

    match result_bridge {
        BridgeMessage::Result(BridgeResult::Failure {
            command_id,
            error,
            fault,
            ..
        }) => {
            assert_eq!(command_id, "http-write-1");
            assert_eq!(error.as_deref(), Some("upstream rejected"));
            assert_eq!(
                fault.category,
                ferredge_bridge::BridgeFaultCategory::Rejected
            );
        }
        other => panic!("expected failure bridge result, got {other:?}"),
    }
}
