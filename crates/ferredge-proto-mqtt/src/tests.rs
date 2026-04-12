use std::{
    collections::HashMap,
    future::Future,
    io::{Read, Write},
    net::TcpListener,
    sync::mpsc,
    task::{Context, Poll, Waker},
    thread,
    time::Duration,
};

use ferredge_core::prelude::{
    Address, BrokerAddress, BrokerChannelKind, BrokerMessageOptions, BrokerSubscriptionOptions, EventSink,
    EventSource, Correlation, Device, DeviceEndpoint, DeviceStatus, Lifecycle, Map,
    MqttEndpointConfig, MqttProtocolVersion, RoutedEvent, RoutedMessage, TransportMeta,
};
use mqtt_protocol_core::mqtt;

use crate::{
    runtime::routed_message_from_packet,
    types::{MqttCommandRef, MqttPacketRequest, MqttWirePacket},
    MqttDriver, MqttListenerStatus,
};

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

fn block_on<F: Future>(future: F) -> F::Output {
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    let mut future = std::pin::pin!(future);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => return output,
            Poll::Pending => thread::yield_now(),
        }
    }
}

fn wait_for_status(
    rx: &mpsc::Receiver<MqttListenerStatus>,
    matcher: impl Fn(&MqttListenerStatus) -> bool,
) -> MqttListenerStatus {
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        let timeout = deadline.saturating_duration_since(std::time::Instant::now());
        let status = rx
            .recv_timeout(timeout)
            .expect("listener status should arrive before timeout");
        if matcher(&status) {
            return status;
        }
    }
}

fn spawn_test_broker_v5(
    publish_after_connack: Option<mqtt::packet::v5_0::Publish>,
) -> (String, mpsc::Sender<()>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("test broker should bind");
    let addr = listener.local_addr().expect("test broker should have local addr");
    let (shutdown_tx, shutdown_rx) = mpsc::channel();

    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("broker should accept client");
        stream
            .set_read_timeout(Some(Duration::from_millis(100)))
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

        if let Some(publish) = publish_after_connack {
            thread::sleep(Duration::from_millis(150));
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

    (format!("mqtt://{addr}"), shutdown_tx, handle)
}

struct NoopSink;

impl EventSink for NoopSink {
    type Event = RoutedEvent;
    type Error = ();

    fn handle(&mut self, _event: Self::Event) -> Result<(), Self::Error> {
        Ok(())
    }
}

struct FailOnEventSink;

impl EventSink for FailOnEventSink {
    type Event = RoutedEvent;
    type Error = ();

    fn handle(&mut self, _event: Self::Event) -> Result<(), Self::Error> {
        Err(())
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
            payload: b"42".to_vec(),
            options: BrokerMessageOptions {
                delivery: Some(ferredge_core::prelude::DeliveryGuarantee::AtLeastOnce),
                headers: Vec::new(),
                reply_to: None,
                correlation_id: None,
            },
        },
        correlation: None,
    };

    let packet = MqttPacketRequest::try_from(MqttCommandRef {
        device: &driver.dvc,
        command: &command,
    })
    .expect("v5 publish should build");

    assert_eq!(packet.command_id, "cmd-1");
    assert!(matches!(packet.packet, MqttWirePacket::V5Publish(_)));
}

#[test]
fn mqtt_send_falls_back_to_v3_when_v5_not_available() {
    let driver = make_driver("mqtt://broker".to_string(), vec![MqttProtocolVersion::V3_1_1]);
    let command = ferredge_core::prelude::Command {
        id: "cmd-2".to_string(),
        source_device_id: None,
        target_device_id: "mqtt-device-1".to_string(),
        intent: ferredge_core::prelude::Intent::Send {
            channel: BrokerAddress {
                name: "sensors/temp".to_string(),
                kind: Some(BrokerChannelKind::Topic),
            },
            payload: b"42".to_vec(),
            options: BrokerMessageOptions::default(),
        },
        correlation: None,
    };

    let packet = MqttPacketRequest::try_from(MqttCommandRef {
        device: &driver.dvc,
        command: &command,
    })
    .expect("v3 publish should build");

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
            },
        },
        correlation: None,
    };

    let packet = MqttPacketRequest::try_from(MqttCommandRef {
        device: &driver.dvc,
        command: &command,
    })
    .expect("subscribe should build");

    assert_eq!(packet.command_id, "cmd-3");
    match packet.packet {
        MqttWirePacket::V5Subscribe(packet) => {
            let mut saw_subscription_identifier = false;
            let mut saw_durable = false;
            let mut saw_shared = false;
            for prop in packet.props() {
                match prop {
                    mqtt::packet::Property::SubscriptionIdentifier(prop) => {
                        saw_subscription_identifier = prop.val() == 1;
                    }
                    mqtt::packet::Property::UserProperty(prop) => {
                        if prop.key() == "ferredge-durable-name" && prop.val() == "durable-a" {
                            saw_durable = true;
                        }
                        if prop.key() == "ferredge-shared-group" && prop.val() == "shared-a" {
                            saw_shared = true;
                        }
                    }
                    _ => {}
                }
            }
            assert!(saw_subscription_identifier);
            assert!(saw_durable);
            assert!(saw_shared);
        }
        other => panic!("expected v5 subscribe packet, got {other:?}"),
    }
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

    let packet = MqttPacketRequest::try_from(MqttCommandRef {
        device: &driver.dvc,
        command: &command,
    })
    .expect("unsubscribe should build");

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
            payload: b"{}".to_vec(),
            options: BrokerMessageOptions {
                delivery: Some(ferredge_core::prelude::DeliveryGuarantee::AtLeastOnce),
                headers: vec![("content-type".to_string(), "application/json".to_string())],
                reply_to: Some("rpc/reply".to_string()),
                correlation_id: Some("corr-123".to_string()),
            },
        },
        correlation: None,
    };

    let packet = MqttPacketRequest::try_from(MqttCommandRef {
        device: &driver.dvc,
        command: &command,
    })
    .expect("v5 publish should build");

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
            payload: b"{}".to_vec(),
            options: BrokerMessageOptions {
                delivery: Some(ferredge_core::prelude::DeliveryGuarantee::AtLeastOnce),
                headers: Vec::new(),
                reply_to: Some("rpc/reply".to_string()),
                correlation_id: None,
            },
        },
        correlation: None,
    };

    let packet = MqttPacketRequest::try_from(MqttCommandRef {
        device: &driver.dvc,
        command: &command,
    })
    .expect("v5 publish should build");

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
            assert_eq!(event.address, Address::Channel("sensors/temp".to_string()));
            assert_eq!(event.payload, b"42".to_vec());
            assert_eq!(
                event.correlation,
                Some(Correlation {
                    request_id: "corr-42".to_string(),
                    reply_to: Some(Address::Channel("rpc/reply".to_string())),
                })
            );
            match event.transport {
                Some(TransportMeta::Mqtt(meta)) => {
                    assert_eq!(meta.content_type, Some("application/json".to_string()));
                    assert_eq!(meta.response_topic, Some("rpc/reply".to_string()));
                    assert_eq!(meta.correlation_data, Some("corr-42".to_string()));
                    assert_eq!(meta.subscription_identifiers, vec![7]);
                    assert_eq!(
                        meta.user_properties,
                        vec![("source".to_string(), "broker-a".to_string())]
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
                reply_to: Some(Address::Channel("rpc/reply".to_string())),
            },
        )]),
        "mqtt-device-1",
        mqtt::packet::Packet::V5_0Publish(publish),
    )
    .expect("reply publish should convert");

    match message {
        RoutedMessage::Result(result) => {
            assert_eq!(result.result.command_id, "cmd-77");
            assert_eq!(result.result.state, ferredge_core::prelude::DeliveryState::Completed);
            assert_eq!(result.result.payload, Some(b"done".to_vec()));
            assert_eq!(
                result.result.correlation,
                Some(Correlation {
                    request_id: "cmd-77".to_string(),
                    reply_to: Some(Address::Channel("rpc/reply".to_string())),
                })
            );
        }
        other => panic!("expected routed result, got {other:?}"),
    }
}

#[test]
fn mqtt_listener_status_starts_stopped() {
    let driver = make_default_driver();

    assert_eq!(
        driver.listener_status().expect("listener status should be readable"),
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
        driver.listener_status().expect("listener status should be readable"),
        MqttListenerStatus::Stopped
    );
}

#[test]
fn mqtt_listener_status_subscription_receives_initial_state() {
    let driver = make_default_driver();

    let rx = driver
        .subscribe_listener_status()
        .expect("listener status subscription should succeed");

    assert_eq!(
        rx.recv().expect("initial listener status should be sent"),
        MqttListenerStatus::Stopped
    );
}

#[test]
fn mqtt_listener_status_subscription_receives_clear_transition() {
    let driver = make_default_driver();
    let rx = driver
        .subscribe_listener_status()
        .expect("listener status subscription should succeed");
    let _ = rx.recv().expect("initial listener status should be sent");

    driver
        .clear_listener_error()
        .expect("clearing empty listener error should succeed");

    assert_eq!(
        rx.recv().expect("listener status transition should be sent"),
        MqttListenerStatus::Stopped
    );
}

#[test]
fn mqtt_listener_can_start_stop_and_restart() {
    let (broker, shutdown_tx, broker_handle) = spawn_test_broker_v5(None);
    let driver = make_driver(broker, vec![MqttProtocolVersion::V5_0]);
    let rx = driver
        .subscribe_listener_status()
        .expect("listener status subscription should succeed");
    let _ = rx.recv().expect("initial listener status should be sent");

    block_on(driver.start_listening(NoopSink)).expect("listener should start");
    assert_eq!(
        wait_for_status(&rx, |status| matches!(status, MqttListenerStatus::Running)),
        MqttListenerStatus::Running
    );

    block_on(driver.stop_listening()).expect("listener should stop");
    assert_eq!(
        wait_for_status(&rx, |status| matches!(status, MqttListenerStatus::Stopped)),
        MqttListenerStatus::Stopped
    );

    block_on(driver.start_listening(NoopSink)).expect("listener should restart");
    assert_eq!(
        wait_for_status(&rx, |status| matches!(status, MqttListenerStatus::Running)),
        MqttListenerStatus::Running
    );

    block_on(driver.stop()).expect("driver should stop cleanly");
    assert!(matches!(
        wait_for_status(&rx, |status| matches!(status, MqttListenerStatus::Stopped)),
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
    let (broker, shutdown_tx, broker_handle) = spawn_test_broker_v5(Some(publish));
    let driver = make_driver(broker, vec![MqttProtocolVersion::V5_0]);
    let rx = driver
        .subscribe_listener_status()
        .expect("listener status subscription should succeed");
    let _ = rx.recv().expect("initial listener status should be sent");

    block_on(driver.start_listening(FailOnEventSink)).expect("listener should start");
    let failed = wait_for_status(&rx, |status| matches!(status, MqttListenerStatus::Failed(_)));
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
