use std::{
    collections::HashMap,
    future::Future,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{Arc, Mutex, OnceLock, mpsc},
    thread,
    time::Duration,
};

use ferredge_bridge::{
    BridgeCommand, BridgeHeaders, BridgeMessage, BridgeOp, BridgePayload, BridgeRoute,
    BridgeTransportMeta, HttpBridgeMeta, MessagingAction, MessagingCapability, MessagingOp,
    ProtocolEncoder, planner,
};
use ferredge_core::prelude::{
    Address, AsyncRuntime, BrokerAddress, BrokerChannelKind, BrokerMessageOptions,
    BrokerMessageProtocolOptions, BrokerReconnectConfig, BrokerSubscriptionOptions,
    BrokerSubscriptionProtocolOptions, ChannelReceiver, Command, Correlation, DeliveryGuarantee,
    Device, DeviceEndpoint, DeviceStatus, EventSink, EventSource, Intent, Lifecycle, Map,
    MqttConnectProperties, MqttEndpointConfig, MqttMessageOptions, MqttPayloadFormat,
    MqttProtocolVersion, MqttRetainHandling, MqttSubscriptionOptions, MqttWillConfig, PayloadValue,
    PubSub, RequestOptions, RoutedEvent, RoutedMessage, TransportMeta,
};
use mqtt_protocol_core::mqtt;
use mqtt_protocol_core::mqtt::packet::GenericPacketTrait;

use crate::{
    MqttAuthChallenge, MqttAuthFlowReason, MqttAuthResponse, MqttAuthStage, MqttBridgeCodec,
    MqttDriver, MqttListenerStatus,
    runtime::{
        build_connect_packet, normalize_broker_addr, pending_reply_route_from_packet,
        routed_message_from_packet,
    },
    runtime_stack::StackRuntime,
    types::{MqttPacketRequest, MqttWirePacket},
};

type RuntimeReceiver<T> = <StackRuntime as AsyncRuntime>::Receiver<T>;

const TEST_STATUS_TIMEOUT_SECS: u64 = 5;
const TEST_BROKER_READ_TIMEOUT_MS: u64 = 100;
const TEST_BROKER_PUBLISH_DELAY_MS: u64 = 150;
const TEST_EVENT_WAIT_POLL_MS: u64 = 25;
const TEST_KEEPALIVE_ASSERT_DELAY_MS: u64 = 1_500;

fn make_driver(broker: String, supported_versions: Vec<MqttProtocolVersion>) -> MqttDriver {
    MqttDriver::new(Device {
        id: "mqtt-device-1".to_string(),
        name: "MQTT Device".to_string(),
        status: DeviceStatus::Online,
        endpoint: DeviceEndpoint::mqtt(MqttEndpointConfig {
            broker,
            client_id: "client-1".to_string(),
            auth: None,
            tls: None,
            keepalive_secs: Some(30),
            clean_start: true,
            session_expiry_secs: None,
            topic_prefix: Some("ferredge".to_string()),
            connect_properties: MqttConnectProperties::default(),
            will: None,
            reconnect: BrokerReconnectConfig::default(),
            supported_versions,
        }),
        metadata: None,
        max_connections: Some(16),
        resources: Map::default(),
        message_endpoints: Vec::new(),
    })
}

fn make_default_driver() -> MqttDriver {
    make_driver("mqtt://broker".to_string(), vec![MqttProtocolVersion::V5_0])
}

fn packet_request(driver: &MqttDriver, command: &Command) -> MqttPacketRequest {
    driver
        .bridge_packet_request(command.clone())
        .expect("mqtt packet should build")
}

fn encode_bridge_message(driver: &MqttDriver, message: &BridgeMessage<'_>) -> MqttPacketRequest {
    MqttBridgeCodec::new(&driver.dvc)
        .encode(message.clone().into_owned())
        .expect("bridge message should encode")
}

fn block_on<F: Future>(future: F) -> F::Output {
    static RUNTIME: OnceLock<StackRuntime> = OnceLock::new();
    RUNTIME.get_or_init(StackRuntime::default).block_on(future)
}

fn wait_for_status(
    rx: &mut RuntimeReceiver<MqttListenerStatus>,
    matcher: impl Fn(&MqttListenerStatus) -> bool,
) -> MqttListenerStatus {
    let deadline = std::time::Instant::now() + Duration::from_secs(TEST_STATUS_TIMEOUT_SECS);
    loop {
        assert!(
            std::time::Instant::now() < deadline,
            "listener status should arrive before timeout"
        );
        let status = block_on(rx.recv()).expect("listener status should arrive before timeout");
        if matcher(&status) {
            return status;
        }
    }
}

fn spawn_test_broker_v5(
    publish_after_connack: Option<mqtt::packet::v5_0::Publish>,
) -> Option<(String, mpsc::Sender<()>, thread::JoinHandle<()>)> {
    let publishes = publish_after_connack.into_iter().collect::<Vec<_>>();
    spawn_test_broker_v5_with_publishes(
        publishes,
        Duration::from_millis(TEST_BROKER_PUBLISH_DELAY_MS),
    )
}

fn spawn_test_broker_v5_with_publishes(
    publishes_after_connack: Vec<mqtt::packet::v5_0::Publish>,
    publish_delay: Duration,
) -> Option<(String, mpsc::Sender<()>, thread::JoinHandle<()>)> {
    let listener = TcpListener::bind("127.0.0.1:0").expect("test broker should bind");
    let addr = listener
        .local_addr()
        .expect("test broker should have local addr");
    let (shutdown_tx, shutdown_rx) = mpsc::channel();

    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("broker should accept client");
        stream
            .set_read_timeout(Some(Duration::from_millis(TEST_BROKER_READ_TIMEOUT_MS)))
            .expect("broker should set read timeout");

        let mut connect_buffer = [0u8; 4096];
        let _ = stream.read(&mut connect_buffer);

        let connack = mqtt::packet::v5_0::Connack::builder()
            .session_present(false)
            .reason_code(mqtt::result_code::ConnectReasonCode::Success)
            .props(Vec::new())
            .build()
            .expect("connack should build");
        stream
            .write_all(&connack.to_continuous_buffer())
            .expect("broker should send connack");

        for publish in publishes_after_connack {
            thread::sleep(publish_delay);
            stream
                .write_all(&publish.to_continuous_buffer())
                .expect("broker should send publish");
        }

        loop {
            if shutdown_rx.try_recv().is_ok() {
                break;
            }

            let mut buffer = [0u8; 4096];
            match stream.read(&mut buffer) {
                Ok(0) => break,
                Ok(_) => {}
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(_) => break,
            }
        }
    });

    Some((format!("mqtt://{addr}"), shutdown_tx, handle))
}

fn spawn_reconnecting_test_broker_v5(
    publish_after_reconnect: mqtt::packet::v5_0::Publish,
) -> Option<(String, mpsc::Sender<()>, thread::JoinHandle<()>)> {
    let listener = TcpListener::bind("127.0.0.1:0").expect("test broker should bind");
    let addr = listener
        .local_addr()
        .expect("test broker should have local addr");
    let (shutdown_tx, shutdown_rx) = mpsc::channel();

    let handle = thread::spawn(move || {
        for connection_idx in 0..2 {
            let (mut stream, _) = listener.accept().expect("broker should accept client");
            stream
                .set_read_timeout(Some(Duration::from_millis(TEST_BROKER_READ_TIMEOUT_MS)))
                .expect("broker should set read timeout");

            let mut connect_buffer = [0u8; 4096];
            let _ = stream.read(&mut connect_buffer);

            let connack = mqtt::packet::v5_0::Connack::builder()
                .session_present(false)
                .reason_code(mqtt::result_code::ConnectReasonCode::Success)
                .props(Vec::new())
                .build()
                .expect("connack should build");
            stream
                .write_all(&connack.to_continuous_buffer())
                .expect("broker should send connack");

            if connection_idx == 0 {
                continue;
            }

            thread::sleep(Duration::from_millis(TEST_BROKER_PUBLISH_DELAY_MS));
            stream
                .write_all(&publish_after_reconnect.to_continuous_buffer())
                .expect("broker should send publish after reconnect");

            loop {
                if shutdown_rx.try_recv().is_ok() {
                    break;
                }
                let mut buffer = [0u8; 4096];
                match stream.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(_) => {}
                    Err(error)
                        if matches!(
                            error.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                        ) => {}
                    Err(_) => break,
                }
            }
        }
    });

    Some((format!("mqtt://{addr}"), shutdown_tx, handle))
}

fn spawn_keepalive_test_broker_v5() -> Option<(String, mpsc::Sender<()>, thread::JoinHandle<()>)> {
    let listener = TcpListener::bind("127.0.0.1:0").expect("test broker should bind");
    let addr = listener
        .local_addr()
        .expect("test broker should have local addr");
    let (shutdown_tx, shutdown_rx) = mpsc::channel();

    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("broker should accept client");
        stream
            .set_read_timeout(Some(Duration::from_millis(TEST_BROKER_READ_TIMEOUT_MS)))
            .expect("broker should set read timeout");

        let mut connect_buffer = [0u8; 4096];
        let _ = stream.read(&mut connect_buffer);

        let connack = mqtt::packet::v5_0::Connack::builder()
            .session_present(false)
            .reason_code(mqtt::result_code::ConnectReasonCode::Success)
            .props(Vec::new())
            .build()
            .expect("connack should build");
        stream
            .write_all(&connack.to_continuous_buffer())
            .expect("broker should send connack");

        loop {
            if shutdown_rx.try_recv().is_ok() {
                break;
            }
            let mut buffer = [0u8; 4096];
            match stream.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => {
                    if buffer[..n].starts_with(&[0xC0, 0x00]) {
                        let pingresp = mqtt::packet::v5_0::Pingresp::builder()
                            .build()
                            .expect("pingresp should build");
                        stream
                            .write_all(&pingresp.to_continuous_buffer())
                            .expect("broker should send pingresp");
                    }
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(_) => break,
            }
        }
    });

    Some((format!("mqtt://{addr}"), shutdown_tx, handle))
}

fn recv_server_packet(
    stream: &mut TcpStream,
    connection: &mut mqtt::Connection<mqtt::role::Server>,
) -> mqtt::packet::Packet {
    let deadline = std::time::Instant::now() + Duration::from_secs(TEST_STATUS_TIMEOUT_SECS);
    loop {
        assert!(
            std::time::Instant::now() < deadline,
            "broker should receive MQTT packet before timeout"
        );
        let mut buffer = [0u8; 4096];
        match stream.read(&mut buffer) {
            Ok(0) => panic!("client closed before expected MQTT packet"),
            Ok(n) => {
                let events = connection.recv(&mut mqtt::common::Cursor::new(&buffer[..n]));
                for event in events {
                    if let mqtt::connection::Event::NotifyPacketReceived(packet) = event {
                        return packet;
                    }
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(error) => panic!("broker read failed: {error}"),
        }
    }
}

fn send_server_packet(
    stream: &mut TcpStream,
    connection: &mut mqtt::Connection<mqtt::role::Server>,
    packet: mqtt::packet::Packet,
) {
    let events = connection.checked_send(packet);
    for event in events {
        if let mqtt::connection::Event::RequestSendPacket { packet, .. } = event {
            stream
                .write_all(&packet.to_continuous_buffer())
                .expect("broker should send MQTT packet");
        }
    }
}

struct NoopSink;

impl EventSink for NoopSink {
    type Event = RoutedEvent<'static>;
    type Error = ();

    fn handle(&mut self, _event: Self::Event) -> Result<(), Self::Error> {
        Ok(())
    }
}

struct FailOnEventSink;

impl EventSink for FailOnEventSink {
    type Event = RoutedEvent<'static>;
    type Error = ();

    fn handle(&mut self, _event: Self::Event) -> Result<(), Self::Error> {
        Err(())
    }
}

struct RecordingSink {
    events: Arc<Mutex<Vec<RoutedEvent<'static>>>>,
}

impl EventSink for RecordingSink {
    type Event = RoutedEvent<'static>;
    type Error = ();

    fn handle(&mut self, event: Self::Event) -> Result<(), Self::Error> {
        self.events.lock().expect("recording sink lock").push(event);
        Ok(())
    }
}

#[test]
fn mqtt_send_prefers_v5_packet_when_available() {
    let driver = make_driver(
        "mqtt://broker".to_string(),
        vec![MqttProtocolVersion::V3_1_1, MqttProtocolVersion::V5_0],
    );
    let command = ferredge_core::prelude::Command {
        id: "cmd-1".to_string(),
        source_device_id: None,
        target_device_id: "mqtt-device-1".to_string(),
        intent: ferredge_core::prelude::Intent::Send {
            channel: BrokerAddress {
                name: "sensors/temp".to_string(),
                kind: Some(BrokerChannelKind::Topic),
            },
            payload: PayloadValue::Bytes(b"42".to_vec().into()),
            options: BrokerMessageOptions {
                delivery: Some(ferredge_core::prelude::DeliveryGuarantee::AtLeastOnce),
                ..BrokerMessageOptions::default()
            },
        },
        correlation: None,
    };

    let packet = packet_request(&driver, &command);

    assert_eq!(packet.command_id, "cmd-1");
    assert!(matches!(packet.packet, MqttWirePacket::V5Publish(_)));
}

#[test]
fn mqtt_send_falls_back_to_v3_when_v5_not_available() {
    let driver = make_driver(
        "mqtt://broker".to_string(),
        vec![MqttProtocolVersion::V3_1_1],
    );
    let command = ferredge_core::prelude::Command {
        id: "cmd-2".to_string(),
        source_device_id: None,
        target_device_id: "mqtt-device-1".to_string(),
        intent: ferredge_core::prelude::Intent::Send {
            channel: BrokerAddress {
                name: "sensors/temp".to_string(),
                kind: Some(BrokerChannelKind::Topic),
            },
            payload: PayloadValue::Bytes(b"42".to_vec().into()),
            options: BrokerMessageOptions::default(),
        },
        correlation: None,
    };

    let packet = packet_request(&driver, &command);

    assert_eq!(packet.command_id, "cmd-2");
    assert!(matches!(packet.packet, MqttWirePacket::V3Publish(_)));
}

#[test]
fn mqtt_subscribe_builds_version_specific_packet() {
    let driver = make_default_driver();
    let command = ferredge_core::prelude::Command {
        id: "cmd-3".to_string(),
        source_device_id: None,
        target_device_id: "mqtt-device-1".to_string(),
        intent: ferredge_core::prelude::Intent::Subscribe {
            channel: BrokerAddress {
                name: "alerts/#".to_string(),
                kind: Some(BrokerChannelKind::Topic),
            },
            options: BrokerSubscriptionOptions {
                delivery: Some(ferredge_core::prelude::DeliveryGuarantee::AtLeastOnce),
                durable_name: Some("durable-a".to_string()),
                shared_group: Some("shared-a".to_string()),
                protocol: Some(BrokerSubscriptionProtocolOptions::Mqtt(
                    MqttSubscriptionOptions {
                        no_local: true,
                        retain_as_published: true,
                        retain_handling: Some(MqttRetainHandling::DoNotSendRetained),
                        subscription_identifier: Some(7),
                        ..MqttSubscriptionOptions::default()
                    },
                )),
                ..BrokerSubscriptionOptions::default()
            },
        },
        correlation: None,
    };

    let packet = packet_request(&driver, &command);

    assert_eq!(packet.command_id, "cmd-3");
    match packet.packet {
        MqttWirePacket::V5Subscribe(packet) => {
            let mut saw_subscription_identifier = false;
            assert_eq!(
                packet.entries()[0].topic_filter(),
                "$share/shared-a/alerts/#"
            );
            assert!(packet.entries()[0].sub_opts().nl());
            assert!(packet.entries()[0].sub_opts().rap());
            assert_eq!(
                packet.entries()[0].sub_opts().rh(),
                mqtt::packet::RetainHandling::DoNotSendRetained
            );
            for prop in packet.props() {
                match prop {
                    mqtt::packet::Property::SubscriptionIdentifier(prop) => {
                        saw_subscription_identifier = prop.val() == 7;
                    }
                    _ => {}
                }
            }
            assert!(saw_subscription_identifier);
        }
        other => panic!("expected v5 subscribe packet, got {other:?}"),
    }
}

#[test]
fn mqtt_v3_rejects_shared_subscriptions() {
    let driver = make_driver(
        "mqtt://broker".to_string(),
        vec![MqttProtocolVersion::V3_1_1],
    );
    let command = ferredge_core::prelude::Command {
        id: "cmd-v3-shared-sub".to_string(),
        source_device_id: None,
        target_device_id: "mqtt-device-1".to_string(),
        intent: ferredge_core::prelude::Intent::Subscribe {
            channel: BrokerAddress {
                name: "alerts/#".to_string(),
                kind: Some(BrokerChannelKind::Topic),
            },
            options: BrokerSubscriptionOptions {
                shared_group: Some("shared-a".to_string()),
                ..BrokerSubscriptionOptions::default()
            },
        },
        correlation: None,
    };

    let error = driver
        .bridge_packet_request(command)
        .expect_err("v3 shared subscriptions should be rejected");
    assert!(matches!(
        error,
        crate::types::MqttCommandConversionError::MqttV5SubscriptionOptionsOnV3
    ));
}

#[test]
fn mqtt_v5_publish_maps_retain_alias_and_expiry_properties() {
    let driver = make_default_driver();
    let command = ferredge_core::prelude::Command {
        id: "cmd-retain-1".to_string(),
        source_device_id: None,
        target_device_id: "mqtt-device-1".to_string(),
        intent: ferredge_core::prelude::Intent::Send {
            channel: BrokerAddress {
                name: "state/device".to_string(),
                kind: Some(BrokerChannelKind::Topic),
            },
            payload: PayloadValue::Bytes(b"online".to_vec().into()),
            options: BrokerMessageOptions {
                delivery: Some(ferredge_core::prelude::DeliveryGuarantee::AtLeastOnce),
                protocol: Some(BrokerMessageProtocolOptions::Mqtt(MqttMessageOptions {
                    retain: true,
                    payload_format: Some(MqttPayloadFormat::Utf8),
                    message_expiry_interval_secs: Some(30),
                    topic_alias: Some(4),
                    ..MqttMessageOptions::default()
                })),
                ..BrokerMessageOptions::default()
            },
        },
        correlation: None,
    };

    let packet = packet_request(&driver, &command);

    match packet.packet {
        MqttWirePacket::V5Publish(packet) => {
            assert!(packet.retain());
            assert!(packet.props().iter().any(|prop| matches!(
                prop,
                mqtt::packet::Property::PayloadFormatIndicator(value) if value.val() == 1
            )));
            assert!(packet.props().iter().any(|prop| matches!(
                prop,
                mqtt::packet::Property::MessageExpiryInterval(value) if value.val() == 30
            )));
            assert!(packet.props().iter().any(|prop| matches!(
                prop,
                mqtt::packet::Property::TopicAlias(value) if value.val() == 4
            )));
        }
        other => panic!("expected v5 publish packet, got {other:?}"),
    }
}

#[test]
fn normalize_broker_addr_adds_default_port_for_bracketed_ipv6() {
    assert_eq!(normalize_broker_addr("mqtt://[::1]"), "[::1]:1883");
    assert_eq!(normalize_broker_addr("[::1]"), "[::1]:1883");
    assert_eq!(normalize_broker_addr("mqtt://[::1]:1884"), "[::1]:1884");
    assert_eq!(
        normalize_broker_addr("mqtt://2001:db8::10"),
        "[2001:db8::10]:1883"
    );
    assert_eq!(normalize_broker_addr("2001:db8::10"), "[2001:db8::10]:1883");
}

#[test]
fn mqtt_unsubscribe_v5_includes_command_id_user_property() {
    let driver = make_default_driver();
    let command = ferredge_core::prelude::Command {
        id: "cmd-unsub-1".to_string(),
        source_device_id: None,
        target_device_id: "mqtt-device-1".to_string(),
        intent: ferredge_core::prelude::Intent::Unsubscribe {
            channel: BrokerAddress {
                name: "alerts/#".to_string(),
                kind: Some(BrokerChannelKind::Topic),
            },
        },
        correlation: None,
    };

    let packet = packet_request(&driver, &command);

    match packet.packet {
        MqttWirePacket::V5Unsubscribe(packet) => {
            let mut saw_command_id = false;
            for prop in packet.props() {
                if let mqtt::packet::Property::UserProperty(prop) = prop
                    && prop.key() == "ferredge-command-id"
                    && prop.val() == "cmd-unsub-1"
                {
                    saw_command_id = true;
                }
            }
            assert!(saw_command_id);
        }
        other => panic!("expected v5 unsubscribe packet, got {other:?}"),
    }
}

#[test]
fn mqtt_v5_publish_maps_reply_and_correlation_properties() {
    let driver = make_default_driver();
    let command = ferredge_core::prelude::Command {
        id: "cmd-4".to_string(),
        source_device_id: None,
        target_device_id: "mqtt-device-1".to_string(),
        intent: ferredge_core::prelude::Intent::Send {
            channel: BrokerAddress {
                name: "rpc/request".to_string(),
                kind: Some(BrokerChannelKind::Topic),
            },
            payload: PayloadValue::Bytes(b"{}".to_vec().into()),
            options: BrokerMessageOptions {
                delivery: Some(ferredge_core::prelude::DeliveryGuarantee::AtLeastOnce),
                reply_to: Some("rpc/reply".to_string()),
                correlation_id: Some("corr-123".to_string()),
                protocol: Some(BrokerMessageProtocolOptions::Mqtt(MqttMessageOptions {
                    content_type: Some("application/json".to_string()),
                    ..MqttMessageOptions::default()
                })),
                ..BrokerMessageOptions::default()
            },
        },
        correlation: None,
    };

    let packet = packet_request(&driver, &command);

    match packet.packet {
        MqttWirePacket::V5Publish(packet) => {
            let mut saw_reply = false;
            let mut saw_corr = false;
            let mut saw_content_type = false;
            for prop in packet.props() {
                match prop {
                    mqtt::packet::Property::ResponseTopic(prop) => {
                        saw_reply = prop.val() == "rpc/reply";
                    }
                    mqtt::packet::Property::CorrelationData(prop) => {
                        saw_corr = prop.val() == b"corr-123";
                    }
                    mqtt::packet::Property::ContentType(prop) => {
                        saw_content_type = prop.val() == "application/json";
                    }
                    _ => {}
                }
            }
            assert!(saw_reply);
            assert!(saw_corr);
            assert!(saw_content_type);
        }
        other => panic!("expected v5 publish packet, got {other:?}"),
    }
}

#[test]
fn mqtt_v5_publish_projects_http_metadata_into_user_properties() {
    let driver = make_default_driver();
    let message = BridgeMessage::Command(BridgeCommand {
        id: "bridge-http-1".to_string().into(),
        source_device_id: Some("http-device-1".to_string()),
        target_device_id: "mqtt-device-1".to_string(),
        capability: ferredge_bridge::BridgeCapability::Messaging(MessagingCapability {
            binary_payloads: true,
        }),
        operation: BridgeOp::Messaging(MessagingOp {
            action: MessagingAction::Publish,
        }),
        payload: Some(BridgePayload::Binary(b"ok".to_vec().into())),
        route: BridgeRoute::Messaging {
            topic: "interop/http".into(),
        },
        transport: Some(BridgeTransportMeta::Http(HttpBridgeMeta {
            method: Some("POST".into()),
            path: Some("/api/result".into()),
            status_code: Some(207),
            content_type: Some("text/plain".into()),
        })),
        headers: Some(BridgeHeaders::http(vec![(
            "X-Request-Version".to_string(),
            "2026-05".to_string(),
        )])),
        correlation: None,
    });

    let packet = encode_bridge_message(&driver, &message);

    match packet.packet {
        MqttWirePacket::V5Publish(packet) => {
            let mut saw_content_type = false;
            let mut saw_status = false;
            let mut saw_method = false;
            let mut saw_path = false;
            let mut saw_header = false;
            for prop in packet.props() {
                match prop {
                    mqtt::packet::Property::ContentType(prop) => {
                        saw_content_type = prop.val() == "text/plain";
                    }
                    mqtt::packet::Property::UserProperty(prop) => match prop.key() {
                        "ferredge-http-status-code" => saw_status = prop.val() == "207",
                        "ferredge-http-method" => saw_method = prop.val() == "POST",
                        "ferredge-http-path" => saw_path = prop.val() == "/api/result",
                        "X-Request-Version" => saw_header = prop.val() == "2026-05",
                        _ => {}
                    },
                    _ => {}
                }
            }
            assert!(saw_content_type);
            assert!(saw_status);
            assert!(saw_method);
            assert!(saw_path);
            assert!(saw_header);
        }
        other => panic!("expected v5 publish packet, got {other:?}"),
    }
}

#[test]
fn mqtt_v5_publish_uses_command_id_as_correlation_when_reply_topic_present() {
    let driver = make_default_driver();
    let command = ferredge_core::prelude::Command {
        id: "cmd-implicit-corr".to_string(),
        source_device_id: None,
        target_device_id: "mqtt-device-1".to_string(),
        intent: ferredge_core::prelude::Intent::Send {
            channel: BrokerAddress {
                name: "rpc/request".to_string(),
                kind: Some(BrokerChannelKind::Topic),
            },
            payload: PayloadValue::Bytes(b"{}".to_vec().into()),
            options: BrokerMessageOptions {
                delivery: Some(ferredge_core::prelude::DeliveryGuarantee::AtLeastOnce),
                reply_to: Some("rpc/reply".to_string()),
                ..BrokerMessageOptions::default()
            },
        },
        correlation: None,
    };

    let packet = packet_request(&driver, &command);

    match packet.packet {
        MqttWirePacket::V5Publish(packet) => {
            let mut saw_corr = false;
            for prop in packet.props() {
                if let mqtt::packet::Property::CorrelationData(prop) = prop {
                    saw_corr = prop.val() == b"cmd-implicit-corr";
                }
            }
            assert!(saw_corr);
        }
        other => panic!("expected v5 publish packet, got {other:?}"),
    }
}

#[test]
fn mqtt_v5_publish_registers_pending_reply_route() {
    let driver = make_default_driver();
    let command = ferredge_core::prelude::Command {
        id: "cmd-route-1".to_string(),
        source_device_id: None,
        target_device_id: "mqtt-device-1".to_string(),
        intent: ferredge_core::prelude::Intent::Send {
            channel: BrokerAddress {
                name: "rpc/request".to_string(),
                kind: Some(BrokerChannelKind::Topic),
            },
            payload: PayloadValue::Bytes(b"{}".to_vec().into()),
            options: BrokerMessageOptions {
                reply_to: Some("rpc/reply".to_string()),
                correlation_id: Some("corr-route-1".to_string()),
                ..BrokerMessageOptions::default()
            },
        },
        correlation: None,
    };

    let route = pending_reply_route_from_packet(&packet_request(&driver, &command))
        .expect("route should be registered");
    assert_eq!(route.0, "corr-route-1");
    assert_eq!(route.1.command_id, "cmd-route-1");
    assert_eq!(
        route.1.reply_to,
        Some(Address::Channel("rpc/reply".to_string().into()))
    );
}

#[test]
fn mqtt_v5_publish_registers_pending_reply_route_from_command_id_fallback() {
    let driver = make_default_driver();
    let command = ferredge_core::prelude::Command {
        id: "cmd-route-fallback".to_string(),
        source_device_id: None,
        target_device_id: "mqtt-device-1".to_string(),
        intent: ferredge_core::prelude::Intent::Send {
            channel: BrokerAddress {
                name: "rpc/request".to_string(),
                kind: Some(BrokerChannelKind::Topic),
            },
            payload: PayloadValue::Bytes(b"{}".to_vec().into()),
            options: BrokerMessageOptions {
                reply_to: Some("rpc/reply".to_string()),
                ..BrokerMessageOptions::default()
            },
        },
        correlation: None,
    };

    let route = pending_reply_route_from_packet(&packet_request(&driver, &command))
        .expect("fallback route should be registered");
    assert_eq!(route.0, "cmd-route-fallback");
    assert_eq!(route.1.command_id, "cmd-route-fallback");
    assert_eq!(
        route.1.reply_to,
        Some(Address::Channel("rpc/reply".to_string().into()))
    );
}

#[test]
fn inbound_publish_packet_converts_to_routed_event() {
    let publish = mqtt::packet::v5_0::Publish::builder()
        .topic_name("sensors/temp")
        .unwrap()
        .payload("42")
        .props({
            let mut props = mqtt::packet::Properties::new();
            props.push(mqtt::packet::Property::ContentType(
                mqtt::packet::ContentType::new("application/json").unwrap(),
            ));
            props.push(mqtt::packet::Property::ResponseTopic(
                mqtt::packet::ResponseTopic::new("rpc/reply").unwrap(),
            ));
            props.push(mqtt::packet::Property::CorrelationData(
                mqtt::packet::CorrelationData::new(b"corr-42".to_vec()).unwrap(),
            ));
            props.push(mqtt::packet::Property::SubscriptionIdentifier(
                mqtt::packet::SubscriptionIdentifier::new(7).unwrap(),
            ));
            props.push(mqtt::packet::Property::UserProperty(
                mqtt::packet::UserProperty::new("source", "broker-a").unwrap(),
            ));
            props
        })
        .build()
        .unwrap();

    let message = routed_message_from_packet(
        &mut HashMap::new(),
        &mut HashMap::new(),
        "mqtt-device-1",
        mqtt::packet::Packet::V5_0Publish(publish),
    )
    .expect("publish packet should convert");

    match message {
        RoutedMessage::Event(event) => {
            assert_eq!(
                event.address,
                Address::Channel("sensors/temp".to_string().into())
            );
            assert_eq!(event.payload, PayloadValue::Bytes(b"42".to_vec().into()));
            assert_eq!(
                event.correlation,
                Some(Correlation {
                    request_id: "corr-42".to_string().into(),
                    reply_to: Some(Address::Channel("rpc/reply".to_string().into())),
                })
            );
            match event.transport {
                Some(TransportMeta::Mqtt(meta)) => {
                    assert_eq!(meta.content_type.as_deref(), Some("application/json"));
                    assert_eq!(meta.response_topic.as_deref(), Some("rpc/reply"));
                    assert_eq!(meta.correlation_data.as_deref(), Some("corr-42"));
                    assert_eq!(
                        meta.correlation_data_bytes.as_deref(),
                        Some(&b"corr-42"[..])
                    );
                    assert_eq!(meta.subscription_identifiers, vec![7]);
                    assert_eq!(
                        meta.user_properties,
                        vec![("source".into(), "broker-a".into())]
                    );
                }
                other => panic!("expected MQTT transport metadata, got {other:?}"),
            }
        }
        other => panic!("expected routed event, got {other:?}"),
    }
}

#[test]
fn inbound_reply_publish_converts_to_routed_result_when_correlation_matches() {
    let publish = mqtt::packet::v5_0::Publish::builder()
        .topic_name("rpc/reply")
        .unwrap()
        .payload("done")
        .props({
            let mut props = mqtt::packet::Properties::new();
            props.push(mqtt::packet::Property::CorrelationData(
                mqtt::packet::CorrelationData::new(b"corr-77".to_vec()).unwrap(),
            ));
            props
        })
        .build()
        .unwrap();

    let message = routed_message_from_packet(
        &mut HashMap::new(),
        &mut HashMap::from([(
            "corr-77".to_string(),
            crate::runtime::PendingReplyRoute {
                command_id: "cmd-77".to_string(),
                reply_to: Some(Address::Channel("rpc/reply".to_string().into())),
            },
        )]),
        "mqtt-device-1",
        mqtt::packet::Packet::V5_0Publish(publish),
    )
    .expect("reply publish should convert");

    match message {
        RoutedMessage::Result(result) => {
            assert_eq!(result.result.command_id, "cmd-77");
            assert_eq!(
                result.result.state,
                ferredge_core::prelude::DeliveryState::Completed
            );
            assert_eq!(
                result.result.payload,
                Some(PayloadValue::Bytes(b"done".to_vec().into()))
            );
            assert_eq!(
                result.result.correlation,
                Some(Correlation {
                    request_id: "cmd-77".to_string().into(),
                    reply_to: Some(Address::Channel("rpc/reply".to_string().into())),
                })
            );
        }
        other => panic!("expected routed result, got {other:?}"),
    }
}

#[test]
fn inbound_reply_publish_consumes_pending_reply_route_once() {
    let publish = mqtt::packet::v5_0::Publish::builder()
        .topic_name("rpc/reply")
        .unwrap()
        .payload("done")
        .props({
            let mut props = mqtt::packet::Properties::new();
            props.push(mqtt::packet::Property::CorrelationData(
                mqtt::packet::CorrelationData::new(b"corr-once".to_vec()).unwrap(),
            ));
            props
        })
        .build()
        .unwrap();

    let mut pending_reply_routes = HashMap::from([(
        "corr-once".to_string(),
        crate::runtime::PendingReplyRoute {
            command_id: "cmd-once".to_string(),
            reply_to: Some(Address::Channel("rpc/reply".to_string().into())),
        },
    )]);
    let first = routed_message_from_packet(
        &mut HashMap::new(),
        &mut pending_reply_routes,
        "mqtt-device-1",
        mqtt::packet::Packet::V5_0Publish(publish.clone()),
    )
    .expect("first reply should convert");
    assert!(matches!(first, RoutedMessage::Result(_)));
    assert!(pending_reply_routes.is_empty());

    let second = routed_message_from_packet(
        &mut HashMap::new(),
        &mut pending_reply_routes,
        "mqtt-device-1",
        mqtt::packet::Packet::V5_0Publish(publish),
    )
    .expect("second publish should convert");
    assert!(matches!(second, RoutedMessage::Event(_)));
}

#[test]
fn v5_puback_failure_converts_to_rejected_result_with_reason_codes() {
    let puback = mqtt::packet::v5_0::Puback::builder()
        .packet_id(42u16)
        .reason_code(mqtt::result_code::PubackReasonCode::QuotaExceeded)
        .build()
        .unwrap();

    let message = routed_message_from_packet(
        &mut HashMap::from([(42u16, "cmd-42".to_string())]),
        &mut HashMap::new(),
        "mqtt-device-1",
        mqtt::packet::Packet::V5_0Puback(puback),
    )
    .expect("puback packet should convert");

    match message {
        RoutedMessage::Result(result) => {
            assert_eq!(result.result.command_id, "cmd-42");
            assert_eq!(
                result.result.state,
                ferredge_core::prelude::DeliveryState::Rejected
            );
            assert_eq!(result.result.error.as_deref(), Some("QuotaExceeded"));
            match result.transport {
                Some(TransportMeta::Mqtt(meta)) => {
                    assert_eq!(meta.packet_id, Some(42));
                    assert_eq!(meta.reason_codes, vec!["QuotaExceeded".to_string()]);
                }
                other => panic!("expected MQTT transport metadata, got {other:?}"),
            }
        }
        other => panic!("expected routed result, got {other:?}"),
    }
}

#[test]
fn v5_puback_success_converts_to_completed_result() {
    let puback = mqtt::packet::v5_0::Puback::builder()
        .packet_id(41u16)
        .reason_code(mqtt::result_code::PubackReasonCode::Success)
        .build()
        .unwrap();

    let message = routed_message_from_packet(
        &mut HashMap::from([(41u16, "cmd-41".to_string())]),
        &mut HashMap::new(),
        "mqtt-device-1",
        mqtt::packet::Packet::V5_0Puback(puback),
    )
    .expect("puback packet should convert");

    match message {
        RoutedMessage::Result(result) => {
            assert_eq!(result.result.command_id, "cmd-41");
            assert_eq!(
                result.result.state,
                ferredge_core::prelude::DeliveryState::Completed
            );
            assert_eq!(result.result.error, None);
        }
        other => panic!("expected routed result, got {other:?}"),
    }
}

#[test]
fn v5_suback_partial_failure_converts_to_rejected_result_with_reason_codes() {
    let suback = mqtt::packet::v5_0::Suback::builder()
        .packet_id(7u16)
        .reason_codes(vec![
            mqtt::result_code::SubackReasonCode::GrantedQos1,
            mqtt::result_code::SubackReasonCode::NotAuthorized,
        ])
        .build()
        .unwrap();

    let message = routed_message_from_packet(
        &mut HashMap::from([(7u16, "cmd-sub-7".to_string())]),
        &mut HashMap::new(),
        "mqtt-device-1",
        mqtt::packet::Packet::V5_0Suback(suback),
    )
    .expect("suback packet should convert");

    match message {
        RoutedMessage::Result(result) => {
            assert_eq!(result.result.command_id, "cmd-sub-7");
            assert_eq!(
                result.result.state,
                ferredge_core::prelude::DeliveryState::Rejected
            );
            assert_eq!(result.result.error.as_deref(), Some("NotAuthorized"));
            match result.transport {
                Some(TransportMeta::Mqtt(meta)) => {
                    assert_eq!(
                        meta.reason_codes,
                        vec!["GrantedQos1".to_string(), "NotAuthorized".to_string()]
                    );
                }
                other => panic!("expected MQTT transport metadata, got {other:?}"),
            }
        }
        other => panic!("expected routed result, got {other:?}"),
    }
}

#[test]
fn v5_suback_success_converts_to_completed_result() {
    let suback = mqtt::packet::v5_0::Suback::builder()
        .packet_id(6u16)
        .reason_codes(vec![mqtt::result_code::SubackReasonCode::GrantedQos1])
        .build()
        .unwrap();

    let message = routed_message_from_packet(
        &mut HashMap::from([(6u16, "cmd-sub-6".to_string())]),
        &mut HashMap::new(),
        "mqtt-device-1",
        mqtt::packet::Packet::V5_0Suback(suback),
    )
    .expect("suback packet should convert");

    match message {
        RoutedMessage::Result(result) => {
            assert_eq!(result.result.command_id, "cmd-sub-6");
            assert_eq!(
                result.result.state,
                ferredge_core::prelude::DeliveryState::Completed
            );
            assert_eq!(result.result.error, None);
        }
        other => panic!("expected routed result, got {other:?}"),
    }
}

#[test]
fn v5_unsuback_failure_converts_to_rejected_result_with_reason_codes() {
    let unsuback = mqtt::packet::v5_0::Unsuback::builder()
        .packet_id(8u16)
        .reason_codes(vec![mqtt::result_code::UnsubackReasonCode::NotAuthorized])
        .build()
        .unwrap();

    let message = routed_message_from_packet(
        &mut HashMap::from([(8u16, "cmd-unsub-8".to_string())]),
        &mut HashMap::new(),
        "mqtt-device-1",
        mqtt::packet::Packet::V5_0Unsuback(unsuback),
    )
    .expect("unsuback packet should convert");

    match message {
        RoutedMessage::Result(result) => {
            assert_eq!(result.result.command_id, "cmd-unsub-8");
            assert_eq!(
                result.result.state,
                ferredge_core::prelude::DeliveryState::Rejected
            );
            assert_eq!(result.result.error.as_deref(), Some("NotAuthorized"));
            match result.transport {
                Some(TransportMeta::Mqtt(meta)) => {
                    assert_eq!(meta.reason_codes, vec!["NotAuthorized".to_string()]);
                }
                other => panic!("expected MQTT transport metadata, got {other:?}"),
            }
        }
        other => panic!("expected routed result, got {other:?}"),
    }
}

#[test]
fn v5_disconnect_failure_converts_to_rejected_transport_result() {
    let disconnect = mqtt::packet::v5_0::Disconnect::builder()
        .reason_code(mqtt::result_code::DisconnectReasonCode::ServerBusy)
        .build()
        .unwrap();

    let message = routed_message_from_packet(
        &mut HashMap::new(),
        &mut HashMap::new(),
        "mqtt-device-1",
        mqtt::packet::Packet::V5_0Disconnect(disconnect),
    )
    .expect("disconnect packet should convert");

    match message {
        RoutedMessage::Result(result) => {
            assert_eq!(result.result.command_id, "__mqtt_disconnect__");
            assert_eq!(
                result.result.state,
                ferredge_core::prelude::DeliveryState::Rejected
            );
            assert_eq!(result.result.error.as_deref(), Some("ServerBusy"));
            match result.transport {
                Some(TransportMeta::Mqtt(meta)) => {
                    assert_eq!(meta.reason_codes, vec!["ServerBusy".to_string()]);
                }
                other => panic!("expected MQTT transport metadata, got {other:?}"),
            }
        }
        other => panic!("expected routed result, got {other:?}"),
    }
}

#[test]
fn v5_disconnect_normal_converts_to_completed_transport_result() {
    let disconnect = mqtt::packet::v5_0::Disconnect::builder()
        .reason_code(mqtt::result_code::DisconnectReasonCode::NormalDisconnection)
        .build()
        .unwrap();

    let message = routed_message_from_packet(
        &mut HashMap::new(),
        &mut HashMap::new(),
        "mqtt-device-1",
        mqtt::packet::Packet::V5_0Disconnect(disconnect),
    )
    .expect("disconnect packet should convert");

    match message {
        RoutedMessage::Result(result) => {
            assert_eq!(result.result.command_id, "__mqtt_disconnect__");
            assert_eq!(
                result.result.state,
                ferredge_core::prelude::DeliveryState::Completed
            );
            assert_eq!(result.result.error, None);
        }
        other => panic!("expected routed result, got {other:?}"),
    }
}

#[test]
fn mqtt_listener_status_starts_stopped() {
    let driver = make_default_driver();

    assert_eq!(
        driver
            .listener_status()
            .expect("listener status should be readable"),
        MqttListenerStatus::Stopped
    );
    assert_eq!(
        driver
            .last_listener_error()
            .expect("listener error should be readable"),
        None
    );
}

#[test]
fn mqtt_listener_error_can_be_cleared_without_runtime() {
    let driver = make_default_driver();

    driver
        .clear_listener_error()
        .expect("clearing empty listener error should succeed");
    assert_eq!(
        driver
            .listener_status()
            .expect("listener status should be readable"),
        MqttListenerStatus::Stopped
    );
}

#[test]
fn mqtt_listener_status_subscription_receives_initial_state() {
    let driver = make_default_driver();

    let mut rx = driver
        .subscribe_listener_status()
        .expect("listener status subscription should succeed");

    assert_eq!(
        block_on(rx.recv()).expect("initial listener status should be sent"),
        MqttListenerStatus::Stopped
    );
}

#[test]
fn mqtt_listener_status_subscription_receives_clear_transition() {
    let driver = make_default_driver();
    let mut rx = driver
        .subscribe_listener_status()
        .expect("listener status subscription should succeed");
    let _ = block_on(rx.recv()).expect("initial listener status should be sent");

    driver
        .clear_listener_error()
        .expect("clearing empty listener error should succeed");

    assert_eq!(
        block_on(rx.recv()).expect("listener status transition should be sent"),
        MqttListenerStatus::Stopped
    );
}

#[test]
fn mqtt_listener_can_start_stop_and_restart() {
    let Some((broker, shutdown_tx, broker_handle)) = spawn_test_broker_v5(None) else {
        return;
    };
    let driver = make_driver(broker, vec![MqttProtocolVersion::V5_0]);
    let mut rx = driver
        .subscribe_listener_status()
        .expect("listener status subscription should succeed");
    let _ = block_on(rx.recv()).expect("initial listener status should be sent");

    block_on(driver.start_listening(NoopSink)).expect("listener should start");
    assert_eq!(
        wait_for_status(&mut rx, |status| matches!(
            status,
            MqttListenerStatus::Running
        )),
        MqttListenerStatus::Running
    );

    block_on(driver.stop_listening()).expect("listener should stop");
    assert_eq!(
        wait_for_status(&mut rx, |status| matches!(
            status,
            MqttListenerStatus::Stopped
        )),
        MqttListenerStatus::Stopped
    );

    block_on(driver.start_listening(NoopSink)).expect("listener should restart");
    assert_eq!(
        wait_for_status(&mut rx, |status| matches!(
            status,
            MqttListenerStatus::Running
        )),
        MqttListenerStatus::Running
    );

    block_on(driver.stop()).expect("driver should stop cleanly");
    assert!(matches!(
        wait_for_status(&mut rx, |status| matches!(
            status,
            MqttListenerStatus::Stopped
        )),
        MqttListenerStatus::Stopped
    ));

    let _ = shutdown_tx.send(());
    broker_handle.join().expect("broker thread should join");
}

#[test]
fn mqtt_listener_reports_failed_when_sink_rejects_event() {
    let publish = mqtt::packet::v5_0::Publish::builder()
        .topic_name("alerts/test")
        .expect("publish topic should be valid")
        .payload("boom")
        .build()
        .expect("publish packet should build");
    let Some((broker, shutdown_tx, broker_handle)) = spawn_test_broker_v5(Some(publish)) else {
        return;
    };
    let driver = make_driver(broker, vec![MqttProtocolVersion::V5_0]);
    let mut rx = driver
        .subscribe_listener_status()
        .expect("listener status subscription should succeed");
    let _ = block_on(rx.recv()).expect("initial listener status should be sent");

    block_on(driver.start_listening(FailOnEventSink)).expect("listener should start");
    let failed = wait_for_status(&mut rx, |status| {
        matches!(status, MqttListenerStatus::Failed(_))
    });
    assert_eq!(
        failed,
        MqttListenerStatus::Failed("failed to forward MQTT event to sink".to_string())
    );
    assert_eq!(
        driver
            .last_listener_error()
            .expect("listener error should be readable"),
        Some("failed to forward MQTT event to sink".to_string())
    );

    block_on(driver.stop()).expect("driver should stop after failure");

    let _ = shutdown_tx.send(());
    broker_handle.join().expect("broker thread should join");
}

#[test]
fn bridge_can_translate_http_command_into_mqtt_publish() {
    let mqtt = make_default_driver();
    let command = Command {
        id: "http-read-1".to_string(),
        source_device_id: Some("http-device-1".to_string()),
        target_device_id: "http-device-1".to_string(),
        intent: Intent::Read {
            resource: "temp".to_string(),
            options: RequestOptions::default(),
        },
        correlation: None,
    };

    let mapped_command = match &command.intent {
        Intent::Read { resource, .. } => Command {
            id: format!("{}-mqtt", command.id),
            source_device_id: Some(command.target_device_id.clone()),
            target_device_id: "mqtt-device-1".to_string(),
            intent: Intent::Send {
                channel: BrokerAddress {
                    name: format!("requests/{resource}"),
                    kind: Some(BrokerChannelKind::Topic),
                },
                payload: PayloadValue::Bytes(Vec::new().into()),
                options: BrokerMessageOptions::default(),
            },
            correlation: command.correlation.clone(),
        },
        _ => panic!("expected read intent"),
    };
    let _bridge_message = planner::command_to_messaging(mapped_command.clone()).unwrap();
    let packet = mqtt
        .bridge_packet_request(mapped_command)
        .expect("bridged command should convert to mqtt packet");

    match packet.packet {
        MqttWirePacket::V5Publish(packet) => {
            assert_eq!(packet.topic_name(), "requests/temp");
        }
        other => panic!("expected v5 publish packet, got {other:?}"),
    }
}

#[test]
fn mqtt_connect_packet_maps_v5_session_expiry() {
    let packet = build_connect_packet(&MqttEndpointConfig {
        broker: "mqtt://broker".to_string(),
        client_id: "client-1".to_string(),
        auth: None,
        tls: None,
        keepalive_secs: Some(30),
        clean_start: true,
        session_expiry_secs: Some(3600),
        topic_prefix: None,
        connect_properties: MqttConnectProperties::default(),
        will: None,
        reconnect: BrokerReconnectConfig::default(),
        supported_versions: vec![MqttProtocolVersion::V5_0],
    })
    .expect("connect packet should build");

    let mqtt::packet::Packet::V5_0Connect(packet) = packet else {
        panic!("expected v5 connect packet");
    };
    let saw_session_expiry = packet.props().iter().any(|prop| {
        matches!(
            prop,
            mqtt::packet::Property::SessionExpiryInterval(value) if value.val() == 3600
        )
    });
    assert!(saw_session_expiry);
}

#[test]
fn mqtt_connect_packet_maps_v5_will_properties() {
    let packet = build_connect_packet(&MqttEndpointConfig {
        broker: "mqtt://broker".to_string(),
        client_id: "client-with-will".to_string(),
        auth: None,
        tls: None,
        keepalive_secs: Some(30),
        clean_start: true,
        session_expiry_secs: None,
        topic_prefix: None,
        connect_properties: MqttConnectProperties::default(),
        will: Some(MqttWillConfig {
            topic: "status/offline".to_string(),
            payload: b"offline".to_vec(),
            delivery: Some(DeliveryGuarantee::AtLeastOnce),
            retain: true,
            delay_interval_secs: Some(5),
            payload_format: Some(MqttPayloadFormat::Utf8),
            message_expiry_interval_secs: Some(60),
            content_type: Some("text/plain".to_string()),
            response_topic: Some("status/reply".to_string()),
            correlation_data: Some(b"will-corr".to_vec()),
            user_properties: vec![("source".to_string(), "ferredge".to_string())],
        }),
        reconnect: BrokerReconnectConfig::default(),
        supported_versions: vec![MqttProtocolVersion::V5_0],
    })
    .expect("connect packet should build");

    let mqtt::packet::Packet::V5_0Connect(packet) = packet else {
        panic!("expected v5 connect packet");
    };
    assert!(packet.will_flag());
    assert_eq!(packet.will_topic(), Some("status/offline"));
    assert_eq!(packet.will_payload(), Some(&b"offline"[..]));
    assert!(packet.will_retain());
    assert!(packet.will_props().iter().any(|prop| matches!(
        prop,
        mqtt::packet::Property::WillDelayInterval(value) if value.val() == 5
    )));
    assert!(packet.will_props().iter().any(|prop| matches!(
        prop,
        mqtt::packet::Property::MessageExpiryInterval(value) if value.val() == 60
    )));
}

#[test]
fn mqtt_listener_reconnects_after_broker_disconnect() {
    let publish = mqtt::packet::v5_0::Publish::builder()
        .topic_name("alerts/reconnected")
        .expect("publish topic should be valid")
        .payload("alive")
        .build()
        .expect("publish packet should build");
    let Some((broker, shutdown_tx, broker_handle)) = spawn_reconnecting_test_broker_v5(publish)
    else {
        return;
    };
    let driver = make_driver(broker, vec![MqttProtocolVersion::V5_0]);
    let events = Arc::new(Mutex::new(Vec::<RoutedEvent<'static>>::new()));

    block_on(driver.start_listening(RecordingSink {
        events: Arc::clone(&events),
    }))
    .expect("listener should start");

    let deadline = std::time::Instant::now() + Duration::from_secs(TEST_STATUS_TIMEOUT_SECS);
    loop {
        if !events.lock().expect("events lock").is_empty() {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "expected event after reconnect"
        );
        thread::sleep(Duration::from_millis(TEST_EVENT_WAIT_POLL_MS));
    }

    assert_eq!(
        events.lock().expect("events lock")[0].address,
        Address::Channel("alerts/reconnected".to_string().into())
    );
    assert_eq!(
        driver.listener_status().expect("listener status"),
        MqttListenerStatus::Running
    );

    block_on(driver.stop()).expect("driver should stop after reconnect test");
    let _ = shutdown_tx.send(());
    broker_handle.join().expect("broker thread should join");
}

#[test]
fn mqtt_listener_keeps_running_with_ping_response() {
    let Some((broker, shutdown_tx, broker_handle)) = spawn_keepalive_test_broker_v5() else {
        return;
    };
    let driver = MqttDriver::new(Device {
        id: "mqtt-device-keepalive".to_string(),
        name: "MQTT Keepalive Device".to_string(),
        status: DeviceStatus::Online,
        endpoint: DeviceEndpoint::mqtt(MqttEndpointConfig {
            broker,
            client_id: "client-keepalive".to_string(),
            auth: None,
            tls: None,
            keepalive_secs: Some(1),
            clean_start: true,
            session_expiry_secs: None,
            topic_prefix: None,
            connect_properties: MqttConnectProperties::default(),
            will: None,
            reconnect: BrokerReconnectConfig::default(),
            supported_versions: vec![MqttProtocolVersion::V5_0],
        }),
        metadata: None,
        max_connections: Some(4),
        resources: Map::default(),
        message_endpoints: Vec::new(),
    });

    block_on(driver.start_listening(NoopSink)).expect("listener should start");
    thread::sleep(Duration::from_millis(TEST_KEEPALIVE_ASSERT_DELAY_MS));
    assert_eq!(
        driver.listener_status().expect("listener status"),
        MqttListenerStatus::Running
    );

    block_on(driver.stop()).expect("driver should stop after keepalive test");
    let _ = shutdown_tx.send(());
    broker_handle.join().expect("broker thread should join");
}

#[test]
fn mqtt_same_driver_can_listen_and_control_subscriptions() {
    let Some((broker, shutdown_tx, broker_handle)) = spawn_keepalive_test_broker_v5() else {
        return;
    };
    let driver = make_driver(broker, vec![MqttProtocolVersion::V5_0]);

    let subscribe_packet = packet_request(
        &driver,
        &Command {
            id: "same-driver-sub".to_string(),
            source_device_id: None,
            target_device_id: driver.dvc.id.clone(),
            intent: Intent::Subscribe {
                channel: BrokerAddress {
                    name: "ferredge/control".to_string(),
                    kind: Some(BrokerChannelKind::Topic),
                },
                options: BrokerSubscriptionOptions::default(),
            },
            correlation: None,
        },
    );

    let unsubscribe_packet = packet_request(
        &driver,
        &Command {
            id: "same-driver-unsub".to_string(),
            source_device_id: None,
            target_device_id: driver.dvc.id.clone(),
            intent: Intent::Unsubscribe {
                channel: BrokerAddress {
                    name: "ferredge/control".to_string(),
                    kind: Some(BrokerChannelKind::Topic),
                },
            },
            correlation: None,
        },
    );

    let publish_packet = packet_request(
        &driver,
        &Command {
            id: "same-driver-pub".to_string(),
            source_device_id: None,
            target_device_id: driver.dvc.id.clone(),
            intent: Intent::Send {
                channel: BrokerAddress {
                    name: "ferredge/control".to_string(),
                    kind: Some(BrokerChannelKind::Topic),
                },
                payload: PayloadValue::Bytes(b"same-driver".to_vec().into()),
                options: BrokerMessageOptions::default(),
            },
            correlation: None,
        },
    );

    block_on(driver.start_listening(NoopSink)).expect("listener should start");
    block_on(driver.subscribe(subscribe_packet, NoopSink))
        .expect("same driver should subscribe while listening");
    block_on(driver.publish(publish_packet)).expect("same driver should publish while listening");
    block_on(driver.unsubscribe(unsubscribe_packet))
        .expect("same driver should unsubscribe while listening");

    assert_eq!(
        driver.listener_status().expect("listener status"),
        MqttListenerStatus::Running
    );

    block_on(driver.stop()).expect("driver should stop after same-driver control test");
    let _ = shutdown_tx.send(());
    broker_handle.join().expect("broker thread should join");
}

#[test]
fn mqtt_connect_auth_roundtrip_uses_handler_response() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("test broker should bind");
    let addr = listener
        .local_addr()
        .expect("test broker should have local addr");
    let (shutdown_tx, shutdown_rx) = mpsc::channel();

    let broker_handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("broker should accept client");
        stream
            .set_read_timeout(Some(Duration::from_millis(TEST_BROKER_READ_TIMEOUT_MS)))
            .expect("broker should set read timeout");

        let mut connection =
            mqtt::Connection::<mqtt::role::Server>::new(mqtt::Version::Undetermined);

        let connect = recv_server_packet(&mut stream, &mut connection);
        match connect {
            mqtt::packet::Packet::V5_0Connect(packet) => {
                let mut authentication_method = None;
                let mut authentication_data = None;
                for prop in packet.props() {
                    match prop {
                        mqtt::packet::Property::AuthenticationMethod(prop) => {
                            authentication_method = Some(prop.val().to_string());
                        }
                        mqtt::packet::Property::AuthenticationData(prop) => {
                            authentication_data = Some(prop.val().to_vec());
                        }
                        _ => {}
                    }
                }
                assert_eq!(authentication_method.as_deref(), Some("scram-sha-256"));
                assert_eq!(authentication_data, Some(b"client-first".to_vec()));
            }
            other => panic!("expected v5 connect packet, got {other:?}"),
        }

        let auth = mqtt::packet::v5_0::Auth::builder()
            .reason_code(mqtt::result_code::AuthReasonCode::ContinueAuthentication)
            .props(vec![
                mqtt::packet::Property::AuthenticationMethod(
                    mqtt::packet::AuthenticationMethod::new("scram-sha-256")
                        .expect("auth method should build"),
                ),
                mqtt::packet::Property::AuthenticationData(
                    mqtt::packet::AuthenticationData::new(b"server-first".to_vec())
                        .expect("auth data should build"),
                ),
                mqtt::packet::Property::ReasonString(
                    mqtt::packet::ReasonString::new("challenge")
                        .expect("reason string should build"),
                ),
                mqtt::packet::Property::UserProperty(
                    mqtt::packet::UserProperty::new("step", "1")
                        .expect("user property should build"),
                ),
            ])
            .build()
            .expect("auth should build");
        send_server_packet(&mut stream, &mut connection, auth.into());

        let auth_response = recv_server_packet(&mut stream, &mut connection);
        match auth_response {
            mqtt::packet::Packet::V5_0Auth(packet) => {
                assert_eq!(
                    packet.reason_code(),
                    Some(mqtt::result_code::AuthReasonCode::Success)
                );
                let mut authentication_method = None;
                let mut authentication_data = None;
                let mut reason_string = None;
                let mut user_property = None;
                if let Some(props) = packet.props() {
                    for prop in props {
                        match prop {
                            mqtt::packet::Property::AuthenticationMethod(prop) => {
                                authentication_method = Some(prop.val().to_string());
                            }
                            mqtt::packet::Property::AuthenticationData(prop) => {
                                authentication_data = Some(prop.val().to_vec());
                            }
                            mqtt::packet::Property::ReasonString(prop) => {
                                reason_string = Some(prop.val().to_string());
                            }
                            mqtt::packet::Property::UserProperty(prop) => {
                                user_property =
                                    Some((prop.key().to_string(), prop.val().to_string()));
                            }
                            _ => {}
                        }
                    }
                }
                assert_eq!(authentication_method.as_deref(), Some("scram-sha-256"));
                assert_eq!(authentication_data, Some(b"client-final".to_vec()));
                assert_eq!(reason_string.as_deref(), Some("proof"));
                assert_eq!(
                    user_property,
                    Some(("client-step".to_string(), "2".to_string()))
                );
            }
            other => panic!("expected v5 auth packet, got {other:?}"),
        }

        let connack = mqtt::packet::v5_0::Connack::builder()
            .session_present(false)
            .reason_code(mqtt::result_code::ConnectReasonCode::Success)
            .props(vec![mqtt::packet::Property::AuthenticationMethod(
                mqtt::packet::AuthenticationMethod::new("scram-sha-256")
                    .expect("auth method should build"),
            )])
            .build()
            .expect("connack should build");
        send_server_packet(&mut stream, &mut connection, connack.into());

        loop {
            if shutdown_rx.try_recv().is_ok() {
                break;
            }
            let mut buffer = [0u8; 4096];
            match stream.read(&mut buffer) {
                Ok(0) => break,
                Ok(_) => {}
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(_) => break,
            }
        }
    });

    let mut driver = make_driver(format!("mqtt://{addr}"), vec![MqttProtocolVersion::V5_0]);
    let seen_challenges = Arc::new(Mutex::new(Vec::new()));
    if let DeviceEndpoint::Mqtt(config) = &mut driver.dvc.endpoint {
        config.connect_properties.authentication_method = Some("scram-sha-256".to_string());
        config.connect_properties.authentication_data = Some(b"client-first".to_vec());
    }
    {
        let seen_challenges = Arc::clone(&seen_challenges);
        driver
            .set_auth_handler(move |challenge: MqttAuthChallenge| {
                seen_challenges
                    .lock()
                    .expect("seen challenge lock")
                    .push(challenge.clone());
                Ok(Some(MqttAuthResponse {
                    reason: MqttAuthFlowReason::Success,
                    authentication_method: None,
                    authentication_data: Some(b"client-final".to_vec()),
                    reason_string: Some("proof".to_string()),
                    user_properties: vec![("client-step".to_string(), "2".to_string())],
                }))
            })
            .expect("auth handler should register");
    }

    block_on(driver.start()).expect("driver should finish auth handshake");
    block_on(driver.stop()).expect("driver should stop cleanly after auth handshake");

    let challenges = seen_challenges.lock().expect("seen challenge lock");
    assert_eq!(challenges.len(), 1);
    assert_eq!(challenges[0].stage, MqttAuthStage::Connect);
    assert_eq!(
        challenges[0].reason,
        MqttAuthFlowReason::ContinueAuthentication
    );
    assert_eq!(
        challenges[0].authentication_method.as_deref(),
        Some("scram-sha-256")
    );
    assert_eq!(
        challenges[0].authentication_data,
        Some(b"server-first".to_vec())
    );
    assert_eq!(challenges[0].reason_string.as_deref(), Some("challenge"));
    assert_eq!(
        challenges[0].user_properties,
        vec![("step".to_string(), "1".to_string())]
    );

    let _ = shutdown_tx.send(());
    broker_handle.join().expect("broker thread should join");
}

#[test]
fn mqtt_listener_handles_reauthentication_auth_packets() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("test broker should bind");
    let addr = listener
        .local_addr()
        .expect("test broker should have local addr");
    let (shutdown_tx, shutdown_rx) = mpsc::channel();
    let (reauth_seen_tx, reauth_seen_rx) = mpsc::channel();
    let (reauth_start_tx, reauth_start_rx) = mpsc::channel();

    let broker_handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("broker should accept client");
        stream
            .set_read_timeout(Some(Duration::from_millis(TEST_BROKER_READ_TIMEOUT_MS)))
            .expect("broker should set read timeout");

        let mut connection =
            mqtt::Connection::<mqtt::role::Server>::new(mqtt::Version::Undetermined);

        let connect = recv_server_packet(&mut stream, &mut connection);
        assert!(matches!(connect, mqtt::packet::Packet::V5_0Connect(_)));

        let connack = mqtt::packet::v5_0::Connack::builder()
            .session_present(false)
            .reason_code(mqtt::result_code::ConnectReasonCode::Success)
            .build()
            .expect("connack should build");
        send_server_packet(&mut stream, &mut connection, connack.into());

        reauth_start_rx
            .recv_timeout(Duration::from_secs(TEST_STATUS_TIMEOUT_SECS))
            .expect("test should signal when listener is ready for reauth");

        let reauth = mqtt::packet::v5_0::Auth::builder()
            .reason_code(mqtt::result_code::AuthReasonCode::ReAuthenticate)
            .props(vec![
                mqtt::packet::Property::AuthenticationMethod(
                    mqtt::packet::AuthenticationMethod::new("scram-sha-256")
                        .expect("auth method should build"),
                ),
                mqtt::packet::Property::AuthenticationData(
                    mqtt::packet::AuthenticationData::new(b"server-reauth".to_vec())
                        .expect("auth data should build"),
                ),
            ])
            .build()
            .expect("reauth should build");
        send_server_packet(&mut stream, &mut connection, reauth.into());

        let auth_response = recv_server_packet(&mut stream, &mut connection);
        match auth_response {
            mqtt::packet::Packet::V5_0Auth(packet) => {
                assert_eq!(
                    packet.reason_code(),
                    Some(mqtt::result_code::AuthReasonCode::Success)
                );
                let mut authentication_data = None;
                if let Some(props) = packet.props() {
                    for prop in props {
                        if let mqtt::packet::Property::AuthenticationData(prop) = prop {
                            authentication_data = Some(prop.val().to_vec());
                        }
                    }
                }
                assert_eq!(authentication_data, Some(b"client-reauth".to_vec()));
                reauth_seen_tx
                    .send(())
                    .expect("reauth completion should signal test");
            }
            other => panic!("expected v5 auth packet, got {other:?}"),
        }

        loop {
            if shutdown_rx.try_recv().is_ok() {
                break;
            }
            let mut buffer = [0u8; 4096];
            match stream.read(&mut buffer) {
                Ok(0) => break,
                Ok(_) => {}
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(_) => break,
            }
        }
    });

    let mut driver = make_driver(format!("mqtt://{addr}"), vec![MqttProtocolVersion::V5_0]);
    let seen_challenges = Arc::new(Mutex::new(Vec::new()));
    if let DeviceEndpoint::Mqtt(config) = &mut driver.dvc.endpoint {
        config.connect_properties.authentication_method = Some("scram-sha-256".to_string());
    }
    {
        let seen_challenges = Arc::clone(&seen_challenges);
        driver
            .set_auth_handler(move |challenge: MqttAuthChallenge| {
                seen_challenges
                    .lock()
                    .expect("seen challenge lock")
                    .push(challenge.clone());
                Ok(Some(MqttAuthResponse {
                    reason: MqttAuthFlowReason::Success,
                    authentication_method: None,
                    authentication_data: Some(match challenge.stage {
                        MqttAuthStage::Connect => b"client-connect".to_vec(),
                        MqttAuthStage::Reauthenticate => b"client-reauth".to_vec(),
                    }),
                    reason_string: None,
                    user_properties: Vec::new(),
                }))
            })
            .expect("auth handler should register");
    }

    block_on(driver.start_listening(NoopSink)).expect("listener should start");
    reauth_start_tx
        .send(())
        .expect("test should signal broker to start reauth");
    reauth_seen_rx
        .recv_timeout(Duration::from_secs(TEST_STATUS_TIMEOUT_SECS))
        .expect("broker should receive reauth response");
    block_on(driver.stop()).expect("driver should stop cleanly after reauth");

    let challenges = seen_challenges.lock().expect("seen challenge lock");
    assert_eq!(challenges.len(), 1);
    assert_eq!(challenges[0].stage, MqttAuthStage::Reauthenticate);
    assert_eq!(challenges[0].reason, MqttAuthFlowReason::ReAuthenticate);
    assert_eq!(
        challenges[0].authentication_data,
        Some(b"server-reauth".to_vec())
    );

    let _ = shutdown_tx.send(());
    broker_handle.join().expect("broker thread should join");
}
