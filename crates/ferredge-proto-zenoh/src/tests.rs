use ferredge_core::prelude::*;

use crate::convert::plan_from_command;
use crate::types::ZenohNativePlan;
use crate::{ZenohDevice, ZenohEndpointConfig};

use std::borrow::Cow;
use std::collections::HashMap as Map;

fn test_device() -> ZenohDevice {
    Device {
        id: DeviceId::new("test-device"),
        name: "Test Zenoh Device".to_string(),
        status: DeviceStatus::Online,
        endpoint: ZenohEndpointConfig {
            locators: vec!["tcp/localhost:7447".to_string()],
        },
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
        id: CommandId::new("cmd-1"),
        source_device_id: None,
        target_device_id: DeviceId::new("test-device"),
        intent: Intent::Send {
            channel: BrokerAddress {
                name: ChannelName::new("sensor/temperature"),
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
            // Payload is now PayloadValue
            match payload {
                PayloadValue::String(s) => assert_eq!(s.as_ref(), "25.5"),
                _ => panic!("Expected string payload"),
            }
        }
        _ => panic!("Expected Put plan"),
    }
}

#[test]
fn test_plan_from_subscribe_command() {
    let device = test_device();

    let command = Command {
        id: CommandId::new("cmd-2"),
        source_device_id: None,
        target_device_id: DeviceId::new("test-device"),
        intent: Intent::Subscribe {
            channel: BrokerAddress {
                name: ChannelName::new("sensor/+/temperature"),
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
        id: CommandId::new("cmd-3"),
        source_device_id: None,
        target_device_id: DeviceId::new("test-device"),
        intent: Intent::Read {
            resource: ResourceName::new("sensor/temperature"),
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
