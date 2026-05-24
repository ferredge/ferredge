use alloc::{string::ToString, vec, vec::Vec};

use ferredge_core::prelude::{
    Address, BrokerAddress, Command, CommandResult, Correlation, DeliveryGuarantee, DeviceProtocol,
    EndpointRef, Intent, PayloadValue, RoutedResult,
};

use crate::{
    BridgeFault, BridgeFaultCategory, BridgeMessage, BridgePayload, BridgePlannerError,
    BridgeResult, BridgeScalar, planner,
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

    let expected_correlation = command.correlation.clone();
    let BridgeMessage::Command(message) = planner::command_to_request_response(command).unwrap()
    else {
        panic!("expected bridge command");
    };

    assert_eq!(message.meta.resource.as_deref(), Some("setpoint"));
    assert_eq!(message.correlation, expected_correlation);
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

    let BridgeMessage::Command(message) = planner::command_to_messaging(command).unwrap() else {
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
fn planners_reject_unsupported_intents() {
    let subscribe_command = Command {
        id: "cmd-unsupported".to_string(),
        source_device_id: None,
        target_device_id: "dst".to_string(),
        intent: Intent::Subscribe {
            channel: BrokerAddress {
                name: "alerts".to_string(),
                kind: None,
            },
            options: Default::default(),
        },
        correlation: None,
    };
    let read_command = Command {
        id: "cmd-read-unsupported".to_string(),
        source_device_id: None,
        target_device_id: "dst".to_string(),
        intent: Intent::Read {
            resource: "temp".to_string(),
        },
        correlation: None,
    };

    assert_eq!(
        planner::command_to_request_response(subscribe_command.clone()).unwrap_err(),
        BridgePlannerError::UnsupportedIntent
    );
    assert_eq!(
        planner::command_to_register_access(
            subscribe_command,
            crate::RegisterMeta {
                address: 1,
                kind: crate::RegisterKind::HoldingRegister,
                quantity: None,
            },
            1,
        )
        .unwrap_err(),
        BridgePlannerError::UnsupportedIntent
    );
    assert_eq!(
        planner::command_to_messaging(read_command).unwrap_err(),
        BridgePlannerError::UnsupportedIntent
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
fn routed_result_to_bridge_preserves_progress_success_and_failure() {
    let source = EndpointRef {
        device_id: "mqtt-1".to_string(),
        protocol: DeviceProtocol::MQTT,
    };
    let correlation = Some(Correlation {
        request_id: "corr-1".to_string(),
        reply_to: Some(Address::Channel("reply/topic".to_string())),
    });

    let progress = RoutedResult {
        source: source.clone(),
        result: CommandResult {
            command_id: "cmd-progress".to_string(),
            device_id: "mqtt-1".to_string(),
            state: ferredge_core::prelude::DeliveryState::Accepted,
            payload: None,
            error: None,
            correlation: correlation.clone(),
        },
        transport: None,
    };
    let success = RoutedResult {
        source: source.clone(),
        result: CommandResult {
            command_id: "cmd-success".to_string(),
            device_id: "mqtt-1".to_string(),
            state: ferredge_core::prelude::DeliveryState::Completed,
            payload: Some(PayloadValue::Bytes(vec![4, 2])),
            error: None,
            correlation: correlation.clone(),
        },
        transport: None,
    };
    let rejected = RoutedResult {
        source: source.clone(),
        result: CommandResult {
            command_id: "cmd-rejected".to_string(),
            device_id: "mqtt-1".to_string(),
            state: ferredge_core::prelude::DeliveryState::Rejected,
            payload: Some(PayloadValue::String("partial".to_string())),
            error: Some("denied".to_string()),
            correlation: correlation.clone(),
        },
        transport: None,
    };
    let timed_out = RoutedResult {
        source: source.clone(),
        result: CommandResult {
            command_id: "cmd-timeout".to_string(),
            device_id: "mqtt-1".to_string(),
            state: ferredge_core::prelude::DeliveryState::TimedOut,
            payload: None,
            error: Some("timed out".to_string()),
            correlation,
        },
        transport: None,
    };

    let BridgeMessage::Result(progress) = planner::routed_result_to_bridge(progress) else {
        panic!("expected bridge result");
    };
    assert!(matches!(
        progress,
        BridgeResult::Progress {
            command_id,
            state: ferredge_core::prelude::DeliveryState::Accepted,
            correlation: Some(_),
            ..
        } if command_id == "cmd-progress"
    ));

    let BridgeMessage::Result(success) = planner::routed_result_to_bridge(success) else {
        panic!("expected bridge result");
    };
    assert!(matches!(
        success,
        BridgeResult::Success {
            command_id,
            payload: Some(BridgePayload::Binary(bytes)),
            ..
        } if command_id == "cmd-success" && bytes == vec![4, 2]
    ));

    let BridgeMessage::Result(rejected) = planner::routed_result_to_bridge(rejected) else {
        panic!("expected bridge result");
    };
    match rejected {
        BridgeResult::Failure {
            command_id,
            state,
            error,
            fault,
            correlation,
            ..
        } => {
            assert_eq!(command_id, "cmd-rejected");
            assert_eq!(state, ferredge_core::prelude::DeliveryState::Rejected);
            assert_eq!(error.as_deref(), Some("denied"));
            assert_eq!(fault.category, BridgeFaultCategory::Rejected);
            assert_eq!(fault.detail.as_deref(), Some("denied"));
            assert!(!fault.retryable);
            assert_eq!(
                fault.source.and_then(|source| source.protocol),
                Some(DeviceProtocol::MQTT)
            );
            assert!(correlation.is_some());
        }
        other => panic!("expected failure bridge result, got {other:?}"),
    }

    let BridgeMessage::Result(timed_out) = planner::routed_result_to_bridge(timed_out) else {
        panic!("expected bridge result");
    };
    match timed_out {
        BridgeResult::Failure {
            command_id,
            state,
            error,
            fault,
            ..
        } => {
            assert_eq!(command_id, "cmd-timeout");
            assert_eq!(state, ferredge_core::prelude::DeliveryState::TimedOut);
            assert_eq!(error.as_deref(), Some("timed out"));
            assert_eq!(fault.category, BridgeFaultCategory::Timeout);
            assert_eq!(fault.detail.as_deref(), Some("timed out"));
            assert!(fault.retryable);
        }
        other => panic!("expected failure bridge result, got {other:?}"),
    }
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
