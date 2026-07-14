use ferredge_core::prelude::*;

use crate::convert::plan_from_command;
use crate::types::{ZenohNativePlan, ZenohResourceAttributes};

use std::borrow::Cow;
use std::collections::HashMap as Map;

fn test_device() -> Device<ZenohResourceAttributes> {
    Device {
        id: "test-device".to_string(),
        name: "Test Zenoh Device".to_string(),
        status: DeviceStatus::Online,
        endpoint: DeviceEndpoint::mqtt(MqttEndpointConfig {
            broker: "tcp://localhost:7447".to_string(), // Using MQTT config for testing
            client_id: "test-zenoh-client".to_string(),
            auth: None,
            tls: None,
            keepalive_secs: Some(60),
            clean_start: true,
            session_expiry_secs: None,
            topic_prefix: None,
            connect_properties: MqttConnectProperties::default(),
            will: None,
            reconnect: BrokerReconnectConfig::default(),
            supported_versions: vec![MqttProtocolVersion::V5_0],
        }),
        metadata: None,
        max_connections: None,
        resources: Map::new(),
        message_endpoints: vec![],
    }
}

#[test]
fn test_plan_from_send_command() {
    let device = test_device();

    let command = Command {
        id: "cmd-1".to_string(),
        source_device_id: None,
        target_device_id: "test-device".to_string(),
        intent: Intent::Send {
            channel: BrokerAddress {
                name: "sensor/temperature".to_string(),
                kind: Some(BrokerChannelKind::Topic),
            },
            payload: PayloadValue::String(Cow::Borrowed("25.5")),
            options: Default::default(),
        },
        correlation: None,
    };

    let plan = plan_from_command(&command, &device).unwrap();

    match plan {
        ZenohNativePlan::Put {
            command_id,
            key_expr,
            payload,
        } => {
            assert_eq!(command_id, "cmd-1");
            assert_eq!(key_expr, "sensor/temperature");
            // Payload is now BridgePayload, check it's text
            match payload {
                ferredge_bridge::BridgePayload::Text(s) => assert_eq!(s, "25.5"),
                _ => panic!("Expected text payload"),
            }
        }
        _ => panic!("Expected Put plan"),
    }
}

#[test]
fn test_plan_from_subscribe_command() {
    let device = test_device();

    let command = Command {
        id: "cmd-2".to_string(),
        source_device_id: None,
        target_device_id: "test-device".to_string(),
        intent: Intent::Subscribe {
            channel: BrokerAddress {
                name: "sensor/+/temperature".to_string(),
                kind: Some(BrokerChannelKind::Topic),
            },
            options: Default::default(),
        },
        correlation: None,
    };

    let plan = plan_from_command(&command, &device).unwrap();

    match plan {
        ZenohNativePlan::Subscribe {
            command_id,
            key_expr,
        } => {
            assert_eq!(command_id, "cmd-2");
            assert_eq!(key_expr, "sensor/+/temperature");
        }
        _ => panic!("Expected Subscribe plan"),
    }
}

#[test]
fn test_plan_from_read_command() {
    let device = test_device();

    let command = Command {
        id: "cmd-3".to_string(),
        source_device_id: None,
        target_device_id: "test-device".to_string(),
        intent: Intent::Read {
            resource: "sensor/temperature".to_string(),
            options: Default::default(),
        },
        correlation: None,
    };

    let plan = plan_from_command(&command, &device).unwrap();

    match plan {
        ZenohNativePlan::Get {
            command_id,
            key_expr,
            parameters,
        } => {
            assert_eq!(command_id, "cmd-3");
            assert_eq!(key_expr, "sensor/temperature");
            assert!(parameters.is_none());
        }
        _ => panic!("Expected Get plan"),
    }
}
