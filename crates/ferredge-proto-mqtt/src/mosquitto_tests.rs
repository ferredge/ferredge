use std::{
    net::{TcpListener, TcpStream},
    process::{Child, Command as ProcessCommand, Stdio},
    sync::{Arc, Mutex, OnceLock},
    thread,
    time::{Duration, Instant},
};

use ferredge_core::prelude::{
    Address, BrokerAddress, BrokerBackoffStrategy, BrokerMessageOptions, BrokerReconnectConfig,
    BrokerSubscriptionOptions, BrokerChannelKind, Command, Correlation, DeliveryGuarantee, Device,
    DeviceEndpoint, DeviceStatus, EventSink, EventSource, Intent, Lifecycle, Map,
    MqttEndpointConfig, MqttProtocolVersion, PubSub, RoutedEvent, TransportMeta,
};

use crate::{
    runtime_stack::StackRuntime,
    types::{MqttCommandRef, MqttPacketRequest},
    MqttDriver, MqttListenerStatus,
};

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

fn block_on<F: core::future::Future>(future: F) -> F::Output {
    static RUNTIME: OnceLock<StackRuntime> = OnceLock::new();
    RUNTIME.get_or_init(StackRuntime::default).block_on(future)
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
            strategy: BrokerBackoffStrategy::Exponential,
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

fn reserve_free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("free port probe should bind")
        .local_addr()
        .expect("free port probe should have addr")
        .port()
}

struct MosquittoGuard {
    child: Option<Child>,
    port: u16,
}

impl MosquittoGuard {
    fn start() -> Self {
        let port = reserve_free_port();
        let mut guard = Self { child: None, port };
        guard.start_broker();
        guard
    }

    fn start_broker(&mut self) {
        assert!(self.child.is_none(), "mosquitto broker already running");
        let child = ProcessCommand::new("mosquitto")
            .args(["-p", &self.port.to_string(), "-v"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("mosquitto should spawn");
        self.child = Some(child);

        let deadline = Instant::now() + Duration::from_secs(MOSQUITTO_START_TIMEOUT_SECS);
        loop {
            if TcpStream::connect(("127.0.0.1", self.port)).is_ok() {
                break;
            }
            assert!(Instant::now() < deadline, "mosquitto should start before timeout");
            thread::sleep(Duration::from_millis(MOSQUITTO_POLL_INTERVAL_MS));
        }
    }

    fn stop_broker(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    fn host(&self) -> String {
        "127.0.0.1".to_string()
    }

    fn port_string(&self) -> String {
        self.port.to_string()
    }

    fn broker_url(&self) -> String {
        format!("mqtt://127.0.0.1:{}", self.port)
    }
}

impl Drop for MosquittoGuard {
    fn drop(&mut self) {
        self.stop_broker();
    }
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
    MqttPacketRequest::try_from(MqttCommandRef {
        device: &driver.dvc,
        command: &Command {
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
        },
    })
    .expect("subscribe packet should build")
}

fn unsubscribe_packet(driver: &MqttDriver, id: &str, topic: &str) -> MqttPacketRequest {
    MqttPacketRequest::try_from(MqttCommandRef {
        device: &driver.dvc,
        command: &Command {
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
        },
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
    MqttPacketRequest::try_from(MqttCommandRef {
        device: &driver.dvc,
        command: &Command {
            id: id.to_string(),
            source_device_id: None,
            target_device_id: driver.dvc.id.clone(),
            intent: Intent::Send {
                channel: BrokerAddress {
                    name: topic.to_string(),
                    kind: Some(BrokerChannelKind::Topic),
                },
                payload: payload.to_vec(),
                options,
            },
            correlation: None,
        },
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
            .find(|event| event.payload == payload)
            .cloned()
        {
            return event;
        }
        assert!(Instant::now() < deadline, "expected inbound MQTT event");
        thread::sleep(Duration::from_millis(MOSQUITTO_POLL_INTERVAL_MS));
    }
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
#[ignore = "requires local mosquitto process and escalated execution"]
fn mosquitto_extended_client_flow() {
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
            broker.host().as_str(),
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
            broker.host().as_str(),
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
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "outbound-ok");

    block_on(subscriber.stop_listening()).expect("listener should stop before unsubscribe");
    block_on(subscriber.unsubscribe(unsubscribe_packet(
        &subscriber,
        "unsub-1",
        "ferredge/it/in",
    )))
    .expect("subscriber should unsubscribe");

    let prior_len = events.lock().expect("events lock").len();
    let pub_status = ProcessCommand::new("mosquitto_pub")
        .args([
            "-h",
            broker.host().as_str(),
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
#[ignore = "requires local mosquitto process and escalated execution"]
fn mosquitto_qos_matrix_roundtrip() {
    let broker = MosquittoGuard::start();

    assert_qos_roundtrip(&broker, DeliveryGuarantee::BestEffort, "qos0");
    assert_qos_roundtrip(&broker, DeliveryGuarantee::AtLeastOnce, "qos1");
    assert_qos_roundtrip(&broker, DeliveryGuarantee::ExactlyOnce, "qos2");
}

#[test]
#[ignore = "requires local mosquitto process and escalated execution"]
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
        driver.listener_status().expect("listener status should be readable"),
        MqttListenerStatus::Running
    );

    block_on(driver.stop()).expect("driver should stop cleanly");
}

#[test]
#[ignore = "requires local mosquitto process and escalated execution"]
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
            headers: vec![
                ("content-type".to_string(), "application/json".to_string()),
                ("x-trace".to_string(), "trace-123".to_string()),
                ("x-origin".to_string(), "ferredge".to_string()),
            ],
            reply_to: Some("ferredge/it/reply".to_string()),
            correlation_id: Some("corr-v5-123".to_string()),
        },
    )))
    .expect("publisher should publish");

    let event = wait_for_event_payload(&events, br#"{"ok":true}"#);
    assert_eq!(
        event.address,
        Address::Channel("ferredge/it/v5".to_string())
    );
    assert_eq!(event.payload, br#"{"ok":true}"#.to_vec());
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
            assert!(meta
                .user_properties
                .contains(&("x-trace".to_string(), "trace-123".to_string())));
            assert!(meta
                .user_properties
                .contains(&("x-origin".to_string(), "ferredge".to_string())));
        }
        other => panic!("expected MQTT transport metadata, got {other:?}"),
    }

    block_on(publisher.stop()).expect("publisher should stop cleanly");
    block_on(subscriber.stop()).expect("subscriber should stop cleanly");
}

#[test]
#[ignore = "requires local mosquitto process and escalated execution"]
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
        driver.listener_status().expect("listener status should be readable"),
        MqttListenerStatus::Running
    );

    block_on(driver.stop()).expect("driver should stop cleanly");
}

#[test]
#[ignore = "requires local mosquitto process and escalated execution"]
fn mosquitto_listener_reconnects_after_broker_restart_for_publish() {
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
            strategy: BrokerBackoffStrategy::Exponential,
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
            broker.host().as_str(),
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
        driver.listener_status().expect("listener status should be readable"),
        MqttListenerStatus::Running
    );

    block_on(driver.stop()).expect("driver should stop cleanly");
}

#[test]
#[ignore = "requires local mosquitto process and escalated execution"]
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
            strategy: BrokerBackoffStrategy::Fixed,
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
            assert!(!error.is_empty(), "failed listener should retain reconnect error");
        }
        other => panic!("expected failed listener status, got {other:?}"),
    }

    block_on(driver.stop()).expect("driver should stop cleanly");
}

#[test]
#[ignore = "requires local mosquitto process and escalated execution"]
fn mosquitto_replays_subscriptions_after_restart() {
    let mut broker = MosquittoGuard::start();
    let reconnect = BrokerReconnectConfig {
        enabled: true,
        initial_delay_ms: 100,
        max_delay_ms: 500,
        strategy: BrokerBackoffStrategy::Exponential,
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
            broker.host().as_str(),
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
#[ignore = "requires local mosquitto process and escalated execution"]
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
            strategy: BrokerBackoffStrategy::Exponential,
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
