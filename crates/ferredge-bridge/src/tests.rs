use alloc::{string::ToString, vec, vec::Vec};

use ferredge_core::prelude::{
    Address, Command, Correlation, DeliveryGuarantee, DeviceProtocol, EndpointRef, Intent,
    PayloadValue,
};

use crate::{
    BridgeFault, BridgeFaultCategory, BridgeMessage, BridgePayload, BridgeScalar, planner,
};

#[test]
fn payload_roundtrip_preserves_binary_and_structure() {
    let payload = PayloadValue::Map(vec![
        (
            "text".to_string(),
            PayloadValue::String("hello".to_string()),
        ),
        ("bytes".to_string(), PayloadValue::Bytes(vec![1, 2, 3])),
    ]);

    let bridge = BridgePayload::from(payload.clone());
    let roundtrip = PayloadValue::from(bridge);

    assert_eq!(roundtrip, payload);
}

#[test]
fn request_response_planner_preserves_correlation_and_resource() {
    let command = Command {
        id: "cmd-1".to_string(),
        source_device_id: Some("src".to_string()),
        target_device_id: "dst".to_string(),
        intent: Intent::Write {
            resource: "setpoint".to_string(),
            payload: PayloadValue::Bytes(vec![9, 1]),
        },
        correlation: Some(Correlation {
            request_id: "root".to_string(),
            reply_to: Some(Address::Resource("/reply".to_string())),
        }),
    };

    let BridgeMessage::Command(message) = planner::command_to_request_response(&command).unwrap()
    else {
        panic!("expected bridge command");
    };

    assert_eq!(message.meta.resource.as_deref(), Some("setpoint"));
    assert_eq!(message.correlation, command.correlation);
    assert_eq!(message.payload, Some(BridgePayload::Binary(vec![9, 1])));
}

#[test]
fn messaging_planner_preserves_topic_and_correlation_id() {
    let command = Command {
        id: "cmd-2".to_string(),
        source_device_id: None,
        target_device_id: "mqtt".to_string(),
        intent: Intent::Send {
            channel: ferredge_core::prelude::BrokerAddress {
                name: "topic/a".to_string(),
                kind: None,
            },
            payload: PayloadValue::String("hello".to_string()),
            options: ferredge_core::prelude::BrokerMessageOptions {
                delivery: Some(DeliveryGuarantee::AtLeastOnce),
                headers: Vec::new(),
                reply_to: Some("topic/reply".to_string()),
                correlation_id: Some("corr-1".to_string()),
                protocol: None,
            },
        },
        correlation: None,
    };

    let BridgeMessage::Command(message) = planner::command_to_messaging(&command).unwrap() else {
        panic!("expected bridge command");
    };

    assert_eq!(message.meta.topic.as_deref(), Some("topic/a"));
    assert_eq!(message.meta.correlation_id.as_deref(), Some("corr-1"));
    assert_eq!(
        message.meta.reply_to,
        Some(Address::Channel("topic/reply".to_string()))
    );
}

#[test]
fn inbound_register_result_maps_back_to_routed_result() {
    let source = EndpointRef {
        device_id: "modbus-1".to_string(),
        protocol: DeviceProtocol::Modbus,
    };

    let result = planner::inbound_register_result(
        source.clone(),
        "cmd-9".to_string(),
        Some(BridgePayload::Scalar(BridgeScalar::U64(42))),
        None,
    );

    assert_eq!(result.source, source);
    assert_eq!(result.result.command_id, "cmd-9");
    assert_eq!(result.result.payload, Some(PayloadValue::U64(42)));
}

#[test]
fn bridge_fault_preserves_category_code_and_retryability() {
    let fault = BridgeFault::protocol(
        "0x02",
        true,
        DeviceProtocol::Modbus,
        Some("illegal data address".to_string()),
    );

    assert_eq!(fault.category, BridgeFaultCategory::Protocol);
    assert_eq!(fault.protocol_code.as_deref(), Some("0x02"));
    assert!(fault.retryable);
}
