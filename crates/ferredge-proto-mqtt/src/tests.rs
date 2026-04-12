use std::collections::HashMap;

use ferredge_core::prelude::{
    Address, BrokerAddress, BrokerChannelKind, BrokerMessageOptions, BrokerSubscriptionOptions,
    Correlation, Device, DeviceEndpoint, DeviceStatus, Map, MqttEndpointConfig,
    MqttProtocolVersion, RoutedMessage,
};
use mqtt_protocol_core::mqtt;

use crate::{
    runtime::routed_message_from_packet,
    types::{MqttCommandRef, MqttPacketRequest, MqttWirePacket},
    MqttDriver, MqttListenerStatus,
};

fn make_driver(supported_versions: Vec<MqttProtocolVersion>) -> MqttDriver {
    MqttDriver::new(Device {
        id: "mqtt-device-1".to_string(),
        name: "MQTT Device".to_string(),
        status: DeviceStatus::Online,
        endpoint: DeviceEndpoint::mqtt(MqttEndpointConfig {
            broker: "mqtt://broker".to_string(),
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

#[test]
fn mqtt_send_prefers_v5_packet_when_available() {
    let driver = make_driver(vec![MqttProtocolVersion::V3_1_1, MqttProtocolVersion::V5_0]);
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
    let driver = make_driver(vec![MqttProtocolVersion::V3_1_1]);
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
    let driver = make_driver(vec![MqttProtocolVersion::V5_0]);
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
                durable_name: None,
                shared_group: None,
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
    assert!(matches!(packet.packet, MqttWirePacket::V5Subscribe(_)));
}

#[test]
fn mqtt_v5_publish_maps_reply_and_correlation_properties() {
    let driver = make_driver(vec![MqttProtocolVersion::V5_0]);
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
fn inbound_publish_packet_converts_to_routed_event() {
    let publish = mqtt::packet::v5_0::Publish::builder()
        .topic_name("sensors/temp")
        .unwrap()
        .payload("42")
        .props({
            let mut props = mqtt::packet::Properties::new();
            props.push(mqtt::packet::Property::ResponseTopic(
                mqtt::packet::ResponseTopic::new("rpc/reply").unwrap(),
            ));
            props.push(mqtt::packet::Property::CorrelationData(
                mqtt::packet::CorrelationData::new(b"corr-42".to_vec()).unwrap(),
            ));
            props
        })
        .build()
        .unwrap();

    let message = routed_message_from_packet(
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
        }
        other => panic!("expected routed event, got {other:?}"),
    }
}

#[test]
fn mqtt_listener_status_starts_stopped() {
    let driver = make_driver(vec![MqttProtocolVersion::V5_0]);

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
    let driver = make_driver(vec![MqttProtocolVersion::V5_0]);

    driver
        .clear_listener_error()
        .expect("clearing empty listener error should succeed");
    assert_eq!(
        driver.listener_status().expect("listener status should be readable"),
        MqttListenerStatus::Stopped
    );
}
