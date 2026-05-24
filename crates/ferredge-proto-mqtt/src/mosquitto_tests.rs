use std::{
    process::{Command as ProcessCommand, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use ferredge_test_support::{
    mosquitto::MosquittoGuard, process::require_command, runtime::block_on,
};

use ferredge_core::prelude::{
    Address, BackoffStrategy, BrokerAddress, BrokerChannelKind, BrokerMessageOptions,
    BrokerMessageProtocolOptions, BrokerReconnectConfig, BrokerSubscriptionOptions,
    BrokerSubscriptionProtocolOptions, Command, Correlation, DeliveryGuarantee, Device,
    DeviceEndpoint, DeviceStatus, EventSink, EventSource, Intent, Lifecycle, Map,
    MqttConnectProperties, MqttEndpointConfig, MqttMessageOptions, MqttPayloadFormat,
    MqttProtocolVersion, MqttSubscriptionOptions, MqttWillConfig, PayloadValue, PubSub,
    RoutedEvent, TransportMeta,
};

use crate::{MqttDriver, MqttListenerStatus, types::MqttPacketRequest};

const MOSQUITTO_START_TIMEOUT_SECS: u64 = 5;
const MOSQUITTO_EVENT_WAIT_TIMEOUT_SECS: u64 = 5;
const MOSQUITTO_POLL_INTERVAL_MS: u64 = 25;
const MOSQUITTO_CONNECT_RETRY_INTERVAL_MS: u64 = 50;
const MOSQUITTO_SUBSCRIBER_STARTUP_MS: u64 = 200;
const MOSQUITTO_UNSUBSCRIBE_SETTLE_MS: u64 = 500;
const MOSQUITTO_RESTART_DOWN_WAIT_MS: u64 = 400;
const MOSQUITTO_RECONNECT_SETTLE_MS: u64 = 1_200;
const MOSQUITTO_KEEPALIVE_SHORT_SECS: u64 = 5;
const MOSQUITTO_KEEPALIVE_LONG_SECS: u64 = 35;

fn require_mosquitto_cli() {
    require_command("mosquitto_pub");
    require_command("mosquitto_sub");
}

fn make_driver_with_client_id_and_keepalive(
    broker: String,
    device_id: String,
    client_id: String,
    keepalive_secs: Option<u16>,
) -> MqttDriver {
    make_driver_with_config(
        broker,
        device_id,
        client_id,
        keepalive_secs,
        BrokerReconnectConfig {
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
        true,
        None,
    )
}

fn make_driver_with_config(
    broker: String,
    device_id: String,
    client_id: String,
    keepalive_secs: Option<u16>,
    reconnect: BrokerReconnectConfig,
    clean_start: bool,
    session_expiry_secs: Option<u32>,
) -> MqttDriver {
    MqttDriver::new(Device {
        id: device_id,
        name: "MQTT Mosquitto Device".to_string(),
        status: DeviceStatus::Online,
        endpoint: DeviceEndpoint::mqtt(MqttEndpointConfig {
            broker,
            client_id,
            auth: None,
            tls: None,
            keepalive_secs,
            clean_start,
            session_expiry_secs,
            topic_prefix: None,
            connect_properties: MqttConnectProperties::default(),
            will: None,
            reconnect,
            supported_versions: vec![MqttProtocolVersion::V5_0],
        }),
        metadata: None,
        max_connections: Some(4),
        resources: Map::default(),
        message_endpoints: Vec::new(),
    })
}

fn make_named_driver(broker: String, device_id: &str, client_id: &str) -> MqttDriver {
    make_driver_with_client_id_and_keepalive(
        broker,
        device_id.to_string(),
        client_id.to_string(),
        Some(5),
    )
}

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

fn subscribe_packet(driver: &MqttDriver, id: &str, topic: &str) -> MqttPacketRequest {
    driver
        .bridge_packet_request(Command {
            id: id.to_string(),
            source_device_id: None,
            target_device_id: driver.dvc.id.clone(),
            intent: Intent::Subscribe {
                channel: BrokerAddress {
                    name: topic.to_string(),
                    kind: Some(BrokerChannelKind::Topic),
                },
                options: BrokerSubscriptionOptions::default(),
            },
            correlation: None,
        })
        .expect("subscribe packet should build")
}

fn unsubscribe_packet(driver: &MqttDriver, id: &str, topic: &str) -> MqttPacketRequest {
    driver
        .bridge_packet_request(Command {
            id: id.to_string(),
            source_device_id: None,
            target_device_id: driver.dvc.id.clone(),
            intent: Intent::Unsubscribe {
                channel: BrokerAddress {
                    name: topic.to_string(),
                    kind: Some(BrokerChannelKind::Topic),
                },
            },
            correlation: None,
        })
        .expect("unsubscribe packet should build")
}

fn publish_packet(
    driver: &MqttDriver,
    id: &str,
    topic: &str,
    payload: &[u8],
    options: BrokerMessageOptions,
) -> MqttPacketRequest {
    driver
        .bridge_packet_request(Command {
            id: id.to_string(),
            source_device_id: None,
            target_device_id: driver.dvc.id.clone(),
            intent: Intent::Send {
                channel: BrokerAddress {
                    name: topic.to_string(),
                    kind: Some(BrokerChannelKind::Topic),
                },
                payload: payload.into(),
                options,
            },
            correlation: None,
        })
        .expect("publish packet should build")
}

fn wait_for_event_payload(events: &Arc<Mutex<Vec<RoutedEvent>>>, payload: &[u8]) -> RoutedEvent {
    let deadline = Instant::now() + Duration::from_secs(MOSQUITTO_EVENT_WAIT_TIMEOUT_SECS);
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
        thread::sleep(Duration::from_millis(MOSQUITTO_POLL_INTERVAL_MS));
    }
}

fn payload_matches(actual: &PayloadValue, expected: &[u8]) -> bool {
    *actual == PayloadValue::from(expected)
        || std::str::from_utf8(expected)
            .map(|value| *actual == PayloadValue::String(value.to_string()))
            .unwrap_or(false)
}

fn wait_for_driver_start(driver: &MqttDriver) {
    let deadline = Instant::now() + Duration::from_secs(MOSQUITTO_START_TIMEOUT_SECS);
    loop {
        match block_on(driver.start()) {
            Ok(()) => return,
            Err(error) => {
                assert!(
                    Instant::now() < deadline,
                    "driver should connect before timeout: {error}"
                );
                thread::sleep(Duration::from_millis(MOSQUITTO_CONNECT_RETRY_INTERVAL_MS));
            }
        }
    }
}

fn assert_qos_roundtrip(broker: &MosquittoGuard, delivery: DeliveryGuarantee, suffix: &str) {
    let topic = format!("ferredge/it/qos/{suffix}");
    let subscriber = make_named_driver(
        broker.broker_url(),
        &format!("mqtt-subscriber-{suffix}"),
        &format!("ferredge-subscriber-{suffix}"),
    );
    let publisher = make_named_driver(
        broker.broker_url(),
        &format!("mqtt-publisher-{suffix}"),
        &format!("ferredge-publisher-{suffix}"),
    );
    let events = Arc::new(Mutex::new(Vec::new()));
    let payload = format!("payload-{suffix}");

    block_on(subscriber.start()).expect("subscriber should connect");
    block_on(subscriber.subscribe(
        subscribe_packet(&subscriber, &format!("sub-{suffix}"), &topic),
        RecordingSink {
            events: Arc::clone(&events),
        },
    ))
    .expect("subscriber should subscribe");
    block_on(subscriber.start_listening(RecordingSink {
        events: Arc::clone(&events),
    }))
    .expect("subscriber listener should start");

    block_on(publisher.start()).expect("publisher should connect");
    block_on(publisher.publish(publish_packet(
        &publisher,
        &format!("pub-{suffix}"),
        &topic,
        payload.as_bytes(),
        BrokerMessageOptions {
            delivery: Some(delivery),
            ..BrokerMessageOptions::default()
        },
    )))
    .expect("publisher should publish");

    let event = wait_for_event_payload(&events, payload.as_bytes());
    assert_eq!(event.address, Address::Channel(topic));

    block_on(publisher.stop()).expect("publisher should stop cleanly");
    block_on(subscriber.stop()).expect("subscriber should stop cleanly");
}

#[test]
fn mosquitto_extended_client_flow() {
    require_mosquitto_cli();
    let broker = MosquittoGuard::start();
    let subscriber = make_named_driver(
        broker.broker_url(),
        "mqtt-mosquitto-subscriber",
        "ferredge-mosquitto-subscriber",
    );
    let publisher = make_named_driver(
        broker.broker_url(),
        "mqtt-mosquitto-publisher",
        "ferredge-mosquitto-publisher",
    );
    let events = Arc::new(Mutex::new(Vec::new()));

    block_on(subscriber.start()).expect("subscriber should connect");
    block_on(subscriber.subscribe(
        subscribe_packet(&subscriber, "sub-1", "ferredge/it/in"),
        RecordingSink {
            events: Arc::clone(&events),
        },
    ))
    .expect("subscriber should subscribe");
    block_on(subscriber.start_listening(RecordingSink {
        events: Arc::clone(&events),
    }))
    .expect("listener should start");

    thread::sleep(Duration::from_millis(MOSQUITTO_SUBSCRIBER_STARTUP_MS));

    let pub_status = ProcessCommand::new("mosquitto_pub")
        .args([
            "-h",
            broker.host(),
            "-p",
            broker.port_string().as_str(),
            "-V",
            "mqttv5",
            "-t",
            "ferredge/it/in",
            "-m",
            "inbound-ok",
        ])
        .status()
        .expect("mosquitto_pub should run");
    assert!(pub_status.success());

    wait_for_event_payload(&events, b"inbound-ok");

    block_on(publisher.start()).expect("publisher should connect");

    let sub_child = ProcessCommand::new("mosquitto_sub")
        .args([
            "-h",
            broker.host(),
            "-p",
            broker.port_string().as_str(),
            "-V",
            "mqttv5",
            "-t",
            "ferredge/it/out",
            "-C",
            "1",
            "-W",
            "5",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("mosquitto_sub should spawn");
    thread::sleep(Duration::from_millis(MOSQUITTO_SUBSCRIBER_STARTUP_MS));

    block_on(publisher.publish(publish_packet(
        &publisher,
        "pub-1",
        "ferredge/it/out",
        b"outbound-ok",
        BrokerMessageOptions::default(),
    )))
    .expect("publisher should publish");

    let output = sub_child
        .wait_with_output()
        .expect("mosquitto_sub should finish");
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "outbound-ok"
    );

    block_on(subscriber.stop_listening()).expect("listener should stop before unsubscribe");
    block_on(subscriber.unsubscribe(unsubscribe_packet(&subscriber, "unsub-1", "ferredge/it/in")))
        .expect("subscriber should unsubscribe");

    let prior_len = events.lock().expect("events lock").len();
    let pub_status = ProcessCommand::new("mosquitto_pub")
        .args([
            "-h",
            broker.host(),
            "-p",
            broker.port_string().as_str(),
            "-V",
            "mqttv5",
            "-t",
            "ferredge/it/in",
            "-m",
            "should-not-arrive",
        ])
        .status()
        .expect("mosquitto_pub should run after unsubscribe");
    assert!(pub_status.success());
    thread::sleep(Duration::from_millis(MOSQUITTO_UNSUBSCRIBE_SETTLE_MS));
    assert_eq!(events.lock().expect("events lock").len(), prior_len);

    block_on(publisher.stop()).expect("publisher should stop cleanly");
    block_on(subscriber.stop()).expect("subscriber should stop cleanly");
}

#[test]
fn mosquitto_qos_matrix_roundtrip() {
    let broker = MosquittoGuard::start();

    assert_qos_roundtrip(&broker, DeliveryGuarantee::BestEffort, "qos0");
    assert_qos_roundtrip(&broker, DeliveryGuarantee::AtLeastOnce, "qos1");
    assert_qos_roundtrip(&broker, DeliveryGuarantee::ExactlyOnce, "qos2");
}

#[test]
fn mosquitto_keepalive_client_flow_five_seconds() {
    let broker = MosquittoGuard::start();
    let driver = make_driver_with_client_id_and_keepalive(
        broker.broker_url(),
        "mqtt-mosquitto-keepalive-device".to_string(),
        "ferredge-mosquitto-keepalive".to_string(),
        Some(2),
    );

    wait_for_driver_start(&driver);
    block_on(driver.start_listening(RecordingSink {
        events: Arc::new(Mutex::new(Vec::new())),
    }))
    .expect("listener should start");

    thread::sleep(Duration::from_secs(MOSQUITTO_KEEPALIVE_SHORT_SECS));

    assert_eq!(
        driver
            .listener_status()
            .expect("listener status should be readable"),
        MqttListenerStatus::Running
    );

    block_on(driver.stop()).expect("driver should stop cleanly");
}

#[test]
fn mosquitto_v5_complex_property_roundtrip() {
    let broker = MosquittoGuard::start();
    let publisher = make_named_driver(
        broker.broker_url(),
        "mqtt-v5-publisher",
        "ferredge-mosquitto-publisher",
    );
    let subscriber = make_named_driver(
        broker.broker_url(),
        "mqtt-v5-subscriber",
        "ferredge-mosquitto-subscriber",
    );
    let events = Arc::new(Mutex::new(Vec::new()));

    block_on(subscriber.start()).expect("subscriber should connect");
    block_on(subscriber.subscribe(
        subscribe_packet(&subscriber, "sub-v5-1", "ferredge/it/v5"),
        RecordingSink {
            events: Arc::clone(&events),
        },
    ))
    .expect("subscriber should subscribe");
    block_on(subscriber.start_listening(RecordingSink {
        events: Arc::clone(&events),
    }))
    .expect("subscriber listener should start");

    block_on(publisher.start()).expect("publisher should connect");
    block_on(publisher.publish(publish_packet(
        &publisher,
        "pub-v5-1",
        "ferredge/it/v5",
        br#"{"ok":true}"#,
        BrokerMessageOptions {
            delivery: Some(DeliveryGuarantee::BestEffort),
            reply_to: Some("ferredge/it/reply".to_string()),
            correlation_id: Some("corr-v5-123".to_string()),
            protocol: Some(BrokerMessageProtocolOptions::Mqtt(MqttMessageOptions {
                content_type: Some("application/json".to_string()),
                user_properties: vec![
                    ("x-trace".to_string(), "trace-123".to_string()),
                    ("x-origin".to_string(), "ferredge".to_string()),
                ],
                ..MqttMessageOptions::default()
            })),
            ..BrokerMessageOptions::default()
        },
    )))
    .expect("publisher should publish");

    let event = wait_for_event_payload(&events, br#"{"ok":true}"#);
    assert_eq!(
        event.address,
        Address::Channel("ferredge/it/v5".to_string())
    );
    assert_eq!(event.payload, br#"{"ok":true}"#.as_slice().into());
    assert_eq!(
        event.correlation,
        Some(Correlation {
            request_id: "corr-v5-123".to_string(),
            reply_to: Some(Address::Channel("ferredge/it/reply".to_string())),
        })
    );

    match event.transport {
        Some(TransportMeta::Mqtt(meta)) => {
            assert_eq!(meta.content_type, Some("application/json".to_string()));
            assert_eq!(meta.response_topic, Some("ferredge/it/reply".to_string()));
            assert_eq!(meta.correlation_data, Some("corr-v5-123".to_string()));
            assert!(
                meta.user_properties
                    .contains(&("x-trace".to_string(), "trace-123".to_string()))
            );
            assert!(
                meta.user_properties
                    .contains(&("x-origin".to_string(), "ferredge".to_string()))
            );
        }
        other => panic!("expected MQTT transport metadata, got {other:?}"),
    }

    block_on(publisher.stop()).expect("publisher should stop cleanly");
    block_on(subscriber.stop()).expect("subscriber should stop cleanly");
}

#[test]
fn mosquitto_will_publishes_after_ungraceful_driver_drop() {
    require_mosquitto_cli();
    let broker = MosquittoGuard::start();
    let will_topic = "ferredge/it/will";
    let subscriber = ProcessCommand::new("mosquitto_sub")
        .args([
            "-h",
            broker.host(),
            "-p",
            broker.port_string().as_str(),
            "-V",
            "mqttv5",
            "-t",
            will_topic,
            "-C",
            "1",
            "-W",
            "5",
            "-F",
            "%t|%p|%C|%D|%E|%F|%R|%P",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("mosquitto_sub should spawn");
    thread::sleep(Duration::from_millis(MOSQUITTO_SUBSCRIBER_STARTUP_MS));

    let mut driver = make_named_driver(
        broker.broker_url(),
        "mqtt-will-publisher",
        "ferredge-mosquitto-will-publisher",
    );
    if let DeviceEndpoint::Mqtt(config) = &mut driver.dvc.endpoint {
        config.will = Some(MqttWillConfig {
            topic: will_topic.to_string(),
            payload: b"offline".to_vec(),
            delivery: Some(DeliveryGuarantee::AtLeastOnce),
            retain: true,
            delay_interval_secs: Some(1),
            payload_format: Some(MqttPayloadFormat::Utf8),
            message_expiry_interval_secs: Some(30),
            content_type: Some("text/plain".to_string()),
            response_topic: Some("ferredge/it/will/reply".to_string()),
            correlation_data: Some(b"will-corr".to_vec()),
            user_properties: vec![("source".to_string(), "ferredge".to_string())],
        });
    }

    block_on(driver.start()).expect("driver should connect with will configured");
    thread::sleep(Duration::from_millis(MOSQUITTO_SUBSCRIBER_STARTUP_MS));
    drop(driver);

    let output = subscriber
        .wait_with_output()
        .expect("mosquitto_sub should finish after will publish");
    assert!(output.status.success(), "will subscriber should succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(will_topic), "missing will topic: {stdout}");
    assert!(stdout.contains("offline"), "missing will payload: {stdout}");
    assert!(
        stdout.contains("text/plain"),
        "missing will content type: {stdout}"
    );
    assert!(
        stdout.contains("will-corr"),
        "missing will correlation data: {stdout}"
    );
    assert!(stdout.contains("30"), "missing will expiry: {stdout}");
    assert!(
        stdout.contains("ferredge/it/will/reply"),
        "missing will response topic: {stdout}"
    );
    assert!(
        stdout.contains("source"),
        "missing will user property key: {stdout}"
    );
    assert!(
        stdout.contains("ferredge"),
        "missing will user property value: {stdout}"
    );
}

#[test]
fn mosquitto_v5_topic_alias_publish_is_accepted_by_broker() {
    require_mosquitto_cli();
    let broker = MosquittoGuard::start();
    let publisher = make_named_driver(
        broker.broker_url(),
        "mqtt-topic-alias-publisher",
        "ferredge-mosquitto-topic-alias-publisher",
    );
    let subscriber = ProcessCommand::new("mosquitto_sub")
        .args([
            "-h",
            broker.host(),
            "-p",
            broker.port_string().as_str(),
            "-V",
            "mqttv5",
            "-t",
            "ferredge/it/topic-alias",
            "-C",
            "1",
            "-W",
            "5",
            "-F",
            "%A|%t|%p",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("mosquitto_sub should spawn");
    thread::sleep(Duration::from_millis(MOSQUITTO_SUBSCRIBER_STARTUP_MS));

    block_on(publisher.start()).expect("publisher should connect");
    block_on(publisher.publish(publish_packet(
        &publisher,
        "pub-topic-alias-1",
        "ferredge/it/topic-alias",
        b"alias-ok",
        BrokerMessageOptions {
            protocol: Some(BrokerMessageProtocolOptions::Mqtt(MqttMessageOptions {
                topic_alias: Some(7),
                ..MqttMessageOptions::default()
            })),
            ..BrokerMessageOptions::default()
        },
    )))
    .expect("publisher should publish with topic alias");

    let output = subscriber
        .wait_with_output()
        .expect("mosquitto_sub should finish after topic alias publish");
    assert!(
        output.status.success(),
        "topic alias subscriber should succeed"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("ferredge/it/topic-alias"),
        "missing topic: {stdout}"
    );
    assert!(stdout.contains("alias-ok"), "missing payload: {stdout}");

    block_on(publisher.stop()).expect("publisher should stop cleanly");
}

#[test]
fn mosquitto_retained_publish_roundtrip_preserves_meta() {
    require_mosquitto_cli();
    let broker = MosquittoGuard::start();
    let publisher = make_named_driver(
        broker.broker_url(),
        "mqtt-retain-publisher",
        "ferredge-mosquitto-retain-publisher",
    );
    let subscriber = make_named_driver(
        broker.broker_url(),
        "mqtt-retain-subscriber",
        "ferredge-mosquitto-retain-subscriber",
    );
    let events = Arc::new(Mutex::new(Vec::new()));
    let topic = "ferredge/it/retain";
    let payload = br#"{"retained":true}"#;

    block_on(publisher.start()).expect("publisher should connect");
    block_on(publisher.publish(publish_packet(
        &publisher,
        "pub-retain-1",
        topic,
        payload,
        BrokerMessageOptions {
            delivery: Some(DeliveryGuarantee::AtLeastOnce),
            protocol: Some(BrokerMessageProtocolOptions::Mqtt(MqttMessageOptions {
                retain: true,
                payload_format: Some(MqttPayloadFormat::Utf8),
                content_type: Some("application/json".to_string()),
                message_expiry_interval_secs: Some(30),
                ..MqttMessageOptions::default()
            })),
            ..BrokerMessageOptions::default()
        },
    )))
    .expect("publisher should publish retained payload");
    block_on(publisher.stop()).expect("publisher should stop cleanly");

    block_on(subscriber.start()).expect("subscriber should connect");
    block_on(subscriber.subscribe(
        subscribe_packet(&subscriber, "sub-retain-1", topic),
        RecordingSink {
            events: Arc::clone(&events),
        },
    ))
    .expect("subscriber should subscribe");
    block_on(subscriber.start_listening(RecordingSink {
        events: Arc::clone(&events),
    }))
    .expect("subscriber listener should start");

    let event = wait_for_event_payload(&events, payload);
    assert_eq!(event.address, Address::Channel(topic.to_string()));
    match event.transport {
        Some(TransportMeta::Mqtt(meta)) => {
            assert!(meta.retain, "retained publish should stay marked retained");
            assert_eq!(meta.content_type, Some("application/json".to_string()));
            assert_eq!(meta.payload_format, Some("1".to_string()));
            assert_eq!(meta.message_expiry_interval_secs, Some(30));
        }
        other => panic!("expected MQTT transport metadata, got {other:?}"),
    }

    let clear_status = ProcessCommand::new("mosquitto_pub")
        .args([
            "-h",
            broker.host(),
            "-p",
            broker.port_string().as_str(),
            "-V",
            "mqttv5",
            "-t",
            topic,
            "-n",
            "-r",
        ])
        .status()
        .expect("mosquitto_pub should clear retained message");
    assert!(clear_status.success());

    block_on(subscriber.stop()).expect("subscriber should stop cleanly");
}

#[test]
fn mosquitto_v5_subscription_identifier_and_no_local_work() {
    require_mosquitto_cli();
    let broker = MosquittoGuard::start();
    let driver = make_named_driver(
        broker.broker_url(),
        "mqtt-subopts-device",
        "ferredge-mosquitto-subopts",
    );
    let events = Arc::new(Mutex::new(Vec::new()));
    let topic = "ferredge/it/subopts";

    block_on(driver.start()).expect("driver should connect");
    block_on(
        driver.subscribe(
            driver
                .bridge_packet_request(Command {
                    id: "subopts-sub".to_string(),
                    source_device_id: None,
                    target_device_id: driver.dvc.id.clone(),
                    intent: Intent::Subscribe {
                        channel: BrokerAddress {
                            name: topic.to_string(),
                            kind: Some(BrokerChannelKind::Topic),
                        },
                        options: BrokerSubscriptionOptions {
                            delivery: Some(DeliveryGuarantee::AtLeastOnce),
                            protocol: Some(BrokerSubscriptionProtocolOptions::Mqtt(
                                MqttSubscriptionOptions {
                                    no_local: true,
                                    subscription_identifier: Some(41),
                                    ..MqttSubscriptionOptions::default()
                                },
                            )),
                            ..BrokerSubscriptionOptions::default()
                        },
                    },
                    correlation: None,
                })
                .expect("subscribe packet should build"),
            RecordingSink {
                events: Arc::clone(&events),
            },
        ),
    )
    .expect("driver should subscribe");
    block_on(driver.start_listening(RecordingSink {
        events: Arc::clone(&events),
    }))
    .expect("driver listener should start");

    block_on(driver.publish(publish_packet(
        &driver,
        "subopts-pub-self",
        topic,
        b"self-should-not-loop",
        BrokerMessageOptions::default(),
    )))
    .expect("driver should publish to same topic");
    thread::sleep(Duration::from_millis(MOSQUITTO_UNSUBSCRIBE_SETTLE_MS));
    assert!(
        events.lock().expect("events lock").is_empty(),
        "no_local subscription should suppress own publish"
    );

    let pub_status = ProcessCommand::new("mosquitto_pub")
        .args([
            "-h",
            broker.host(),
            "-p",
            broker.port_string().as_str(),
            "-V",
            "mqttv5",
            "-t",
            topic,
            "-m",
            "remote-should-arrive",
        ])
        .status()
        .expect("mosquitto_pub should publish remote message");
    assert!(pub_status.success());

    let event = wait_for_event_payload(&events, b"remote-should-arrive");
    match event.transport {
        Some(TransportMeta::Mqtt(meta)) => {
            assert_eq!(meta.subscription_identifiers, vec![41]);
        }
        other => panic!("expected MQTT transport metadata, got {other:?}"),
    }

    block_on(driver.stop()).expect("driver should stop cleanly");
}

#[test]
fn mosquitto_shared_subscriptions_load_balance() {
    let broker = MosquittoGuard::start();
    let subscriber_a = make_named_driver(
        broker.broker_url(),
        "mqtt-shared-subscriber-a",
        "ferredge-mosquitto-shared-a",
    );
    let subscriber_b = make_named_driver(
        broker.broker_url(),
        "mqtt-shared-subscriber-b",
        "ferredge-mosquitto-shared-b",
    );
    let publisher = make_named_driver(
        broker.broker_url(),
        "mqtt-shared-publisher",
        "ferredge-mosquitto-shared-publisher",
    );
    let events_a = Arc::new(Mutex::new(Vec::new()));
    let events_b = Arc::new(Mutex::new(Vec::new()));
    let topic = "ferredge/it/shared";

    for (driver, events, sub_id) in [
        (&subscriber_a, Arc::clone(&events_a), "shared-sub-a"),
        (&subscriber_b, Arc::clone(&events_b), "shared-sub-b"),
    ] {
        block_on(driver.start()).expect("subscriber should connect");
        block_on(
            driver.subscribe(
                driver
                    .bridge_packet_request(Command {
                        id: sub_id.to_string(),
                        source_device_id: None,
                        target_device_id: driver.dvc.id.clone(),
                        intent: Intent::Subscribe {
                            channel: BrokerAddress {
                                name: topic.to_string(),
                                kind: Some(BrokerChannelKind::Topic),
                            },
                            options: BrokerSubscriptionOptions {
                                shared_group: Some("workers".to_string()),
                                ..BrokerSubscriptionOptions::default()
                            },
                        },
                        correlation: None,
                    })
                    .expect("shared subscribe should build"),
                RecordingSink {
                    events: Arc::clone(&events),
                },
            ),
        )
        .expect("subscriber should subscribe shared topic");
        block_on(driver.start_listening(RecordingSink {
            events: Arc::clone(&events),
        }))
        .expect("subscriber listener should start");
    }

    block_on(publisher.start()).expect("publisher should connect");
    block_on(publisher.publish(publish_packet(
        &publisher,
        "shared-pub-1",
        topic,
        b"shared-only-once",
        BrokerMessageOptions::default(),
    )))
    .expect("publisher should publish shared test message");

    let deadline = Instant::now() + Duration::from_secs(MOSQUITTO_EVENT_WAIT_TIMEOUT_SECS);
    loop {
        let count = events_a.lock().expect("events_a lock").len()
            + events_b.lock().expect("events_b lock").len();
        if count == 1 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "shared subscription should deliver to exactly one subscriber"
        );
        thread::sleep(Duration::from_millis(MOSQUITTO_POLL_INTERVAL_MS));
    }

    assert_eq!(
        events_a.lock().expect("events_a lock").len()
            + events_b.lock().expect("events_b lock").len(),
        1
    );

    block_on(publisher.stop()).expect("publisher should stop cleanly");
    block_on(subscriber_a.stop()).expect("subscriber A should stop cleanly");
    block_on(subscriber_b.stop()).expect("subscriber B should stop cleanly");
}

#[test]
fn mosquitto_keepalive_client_flow_thirty_five_seconds() {
    let broker = MosquittoGuard::start();
    let driver = make_driver_with_client_id_and_keepalive(
        broker.broker_url(),
        "mqtt-mosquitto-keepalive-long-device".to_string(),
        "ferredge-mosquitto-keepalive-long".to_string(),
        Some(10),
    );

    block_on(driver.start()).expect("driver should connect");
    block_on(driver.start_listening(RecordingSink {
        events: Arc::new(Mutex::new(Vec::new())),
    }))
    .expect("listener should start");

    thread::sleep(Duration::from_secs(MOSQUITTO_KEEPALIVE_LONG_SECS));

    assert_eq!(
        driver
            .listener_status()
            .expect("listener status should be readable"),
        MqttListenerStatus::Running
    );

    block_on(driver.stop()).expect("driver should stop cleanly");
}

#[test]
fn mosquitto_listener_reconnects_after_broker_restart_for_publish() {
    require_mosquitto_cli();
    let mut broker = MosquittoGuard::start();
    let driver = make_driver_with_config(
        broker.broker_url(),
        "mqtt-mosquitto-reconnect-device".to_string(),
        "ferredge-mosquitto-reconnect".to_string(),
        Some(2),
        BrokerReconnectConfig {
            enabled: true,
            initial_delay_ms: 100,
            max_delay_ms: 500,
            strategy: BackoffStrategy::Exponential,
            multiplier: 2,
            max_attempts: None,
            replay_subscriptions: true,
            queue_requests_while_disconnected: true,
            max_queued_requests: 64,
        },
        true,
        None,
    );

    block_on(driver.start()).expect("driver should connect");
    block_on(driver.start_listening(RecordingSink {
        events: Arc::new(Mutex::new(Vec::new())),
    }))
    .expect("listener should start");

    broker.stop_broker();
    thread::sleep(Duration::from_millis(MOSQUITTO_RESTART_DOWN_WAIT_MS));
    broker.start_broker();

    let sub_child = ProcessCommand::new("mosquitto_sub")
        .args([
            "-h",
            broker.host(),
            "-p",
            broker.port_string().as_str(),
            "-V",
            "mqttv5",
            "-t",
            "ferredge/it/reconnect/out",
            "-C",
            "1",
            "-W",
            "10",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("mosquitto_sub should spawn");

    thread::sleep(Duration::from_millis(MOSQUITTO_RECONNECT_SETTLE_MS));

    block_on(driver.publish(publish_packet(
        &driver,
        "pub-reconnect-1",
        "ferredge/it/reconnect/out",
        b"reconnected-outbound-ok",
        BrokerMessageOptions::default(),
    )))
    .expect("driver should publish after reconnect");

    let output = sub_child
        .wait_with_output()
        .expect("mosquitto_sub should finish after reconnect");
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "reconnected-outbound-ok"
    );
    assert_eq!(
        driver
            .listener_status()
            .expect("listener status should be readable"),
        MqttListenerStatus::Running
    );

    block_on(driver.stop()).expect("driver should stop cleanly");
}

#[test]
fn mosquitto_listener_fails_after_reconnect_attempt_budget_exhausted() {
    let mut broker = MosquittoGuard::start();
    let driver = make_driver_with_config(
        broker.broker_url(),
        "mqtt-mosquitto-reconnect-fail-device".to_string(),
        "ferredge-mosquitto-reconnect-fail".to_string(),
        Some(2),
        BrokerReconnectConfig {
            enabled: true,
            initial_delay_ms: 100,
            max_delay_ms: 100,
            strategy: BackoffStrategy::Fixed,
            multiplier: 1,
            max_attempts: Some(2),
            replay_subscriptions: true,
            queue_requests_while_disconnected: true,
            max_queued_requests: 64,
        },
        true,
        None,
    );

    block_on(driver.start()).expect("driver should connect");
    block_on(driver.start_listening(RecordingSink {
        events: Arc::new(Mutex::new(Vec::new())),
    }))
    .expect("listener should start");

    broker.stop_broker();

    let deadline = Instant::now() + Duration::from_secs(MOSQUITTO_EVENT_WAIT_TIMEOUT_SECS);
    loop {
        let status = driver
            .listener_status()
            .expect("listener status should be readable");
        if matches!(status, MqttListenerStatus::Failed(_)) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "listener should fail after reconnect attempts are exhausted"
        );
        thread::sleep(Duration::from_millis(MOSQUITTO_POLL_INTERVAL_MS));
    }

    match driver
        .listener_status()
        .expect("listener status should be readable")
    {
        MqttListenerStatus::Failed(error) => {
            assert!(
                !error.is_empty(),
                "failed listener should retain reconnect error"
            );
        }
        other => panic!("expected failed listener status, got {other:?}"),
    }

    block_on(driver.stop()).expect("driver should stop cleanly");
}

#[test]
fn mosquitto_replays_subscriptions_after_restart() {
    require_mosquitto_cli();
    let mut broker = MosquittoGuard::start();
    let reconnect = BrokerReconnectConfig {
        enabled: true,
        initial_delay_ms: 100,
        max_delay_ms: 500,
        strategy: BackoffStrategy::Exponential,
        multiplier: 2,
        max_attempts: None,
        replay_subscriptions: true,
        queue_requests_while_disconnected: true,
        max_queued_requests: 64,
    };
    let subscriber = make_driver_with_config(
        broker.broker_url(),
        "mqtt-mosquitto-resub-subscriber".to_string(),
        "ferredge-mosquitto-resub-subscriber".to_string(),
        Some(2),
        reconnect,
        true,
        None,
    );
    let events = Arc::new(Mutex::new(Vec::new()));

    block_on(subscriber.start()).expect("subscriber should connect");
    block_on(subscriber.subscribe(
        subscribe_packet(&subscriber, "sub-recovery-1", "ferredge/it/recovery"),
        RecordingSink {
            events: Arc::clone(&events),
        },
    ))
    .expect("subscriber should subscribe before outage");
    block_on(subscriber.start_listening(RecordingSink {
        events: Arc::clone(&events),
    }))
    .expect("subscriber listener should start");

    broker.stop_broker();
    thread::sleep(Duration::from_millis(MOSQUITTO_RESTART_DOWN_WAIT_MS));
    broker.start_broker();

    thread::sleep(Duration::from_millis(MOSQUITTO_RECONNECT_SETTLE_MS));

    let pub_status = ProcessCommand::new("mosquitto_pub")
        .args([
            "-h",
            broker.host(),
            "-p",
            broker.port_string().as_str(),
            "-V",
            "mqttv5",
            "-t",
            "ferredge/it/recovery",
            "-m",
            "resub-ok",
        ])
        .status()
        .expect("mosquitto_pub should run after broker restart");
    assert!(pub_status.success());

    let event = wait_for_event_payload(&events, b"resub-ok");
    assert_eq!(
        event.address,
        Address::Channel("ferredge/it/recovery".to_string())
    );
    assert_eq!(
        subscriber
            .listener_status()
            .expect("subscriber listener status should be readable"),
        MqttListenerStatus::Running
    );
    block_on(subscriber.stop()).expect("subscriber should stop cleanly");
}

#[test]
fn mosquitto_replays_queued_publish_after_restart() {
    let mut broker = MosquittoGuard::start();
    let driver = make_driver_with_config(
        broker.broker_url(),
        "mqtt-mosquitto-queued-recovery".to_string(),
        "ferredge-mosquitto-queued-recovery".to_string(),
        Some(2),
        BrokerReconnectConfig {
            enabled: true,
            initial_delay_ms: 100,
            max_delay_ms: 500,
            strategy: BackoffStrategy::Exponential,
            multiplier: 2,
            max_attempts: None,
            replay_subscriptions: true,
            queue_requests_while_disconnected: true,
            max_queued_requests: 64,
        },
        true,
        None,
    );
    let events = Arc::new(Mutex::new(Vec::new()));

    block_on(driver.start()).expect("driver should connect");
    block_on(driver.subscribe(
        subscribe_packet(&driver, "sub-queued-recovery-1", "ferredge/it/recovery/out"),
        RecordingSink {
            events: Arc::clone(&events),
        },
    ))
    .expect("driver should subscribe before outage");
    block_on(driver.start_listening(RecordingSink {
        events: Arc::clone(&events),
    }))
    .expect("driver listener should start");

    broker.stop_broker();
    thread::sleep(Duration::from_millis(MOSQUITTO_RESTART_DOWN_WAIT_MS));

    block_on(driver.publish(publish_packet(
        &driver,
        "pub-queued-recovery-1",
        "ferredge/it/recovery/out",
        b"queued-recovery-ok",
        BrokerMessageOptions {
            delivery: Some(DeliveryGuarantee::AtLeastOnce),
            ..BrokerMessageOptions::default()
        },
    )))
    .expect("driver should queue publish during outage");

    broker.start_broker();

    let event = wait_for_event_payload(&events, b"queued-recovery-ok");
    assert_eq!(
        event.address,
        Address::Channel("ferredge/it/recovery/out".to_string())
    );
    assert_eq!(
        driver
            .listener_status()
            .expect("driver listener status should be readable"),
        MqttListenerStatus::Running
    );

    block_on(driver.stop()).expect("driver should stop cleanly");
}
