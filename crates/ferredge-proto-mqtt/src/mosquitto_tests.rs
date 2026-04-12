use std::{
    net::{TcpListener, TcpStream},
    process::{Child, Command as ProcessCommand, Stdio},
    sync::{Arc, Mutex, OnceLock},
    thread,
    time::{Duration, Instant},
};

use ferredge_core::prelude::{
    Address, BrokerAddress, BrokerChannelKind, BrokerMessageOptions, BrokerSubscriptionOptions,
    Command, Correlation, DeliveryGuarantee, Device, DeviceEndpoint, DeviceStatus, EventSink,
    EventSource, Intent, Lifecycle, Map, MqttEndpointConfig, MqttProtocolVersion, PubSub,
    RoutedEvent, TransportMeta,
};

use crate::{
    runtime_stack::StackRuntime,
    types::{MqttCommandRef, MqttPacketRequest},
    MqttDriver, MqttListenerStatus,
};

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
            clean_start: true,
            session_expiry_secs: None,
            topic_prefix: None,
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
    child: Child,
    port: u16,
}

impl MosquittoGuard {
    fn start() -> Self {
        let port = reserve_free_port();
        let child = ProcessCommand::new("mosquitto")
            .args(["-p", &port.to_string(), "-v"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("mosquitto should spawn");

        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if TcpStream::connect(("127.0.0.1", port)).is_ok() {
                break;
            }
            assert!(Instant::now() < deadline, "mosquitto should start before timeout");
            thread::sleep(Duration::from_millis(25));
        }

        Self { child, port }
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
        let _ = self.child.kill();
        let _ = self.child.wait();
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
    let deadline = Instant::now() + Duration::from_secs(5);
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
        thread::sleep(Duration::from_millis(25));
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

    thread::sleep(Duration::from_millis(200));

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
    thread::sleep(Duration::from_millis(200));

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
    thread::sleep(Duration::from_millis(500));
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

    block_on(driver.start()).expect("driver should connect");
    block_on(driver.start_listening(RecordingSink {
        events: Arc::new(Mutex::new(Vec::new())),
    }))
    .expect("listener should start");

    thread::sleep(Duration::from_secs(5));

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

    thread::sleep(Duration::from_secs(35));

    assert_eq!(
        driver.listener_status().expect("listener status should be readable"),
        MqttListenerStatus::Running
    );

    block_on(driver.stop()).expect("driver should stop cleanly");
}
