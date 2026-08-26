use alloc::{string::ToString, vec};

use ferredge_core::prelude::{
    Address, BrokerAddress, Command, CommandResult, Correlation, DeliveryGuarantee, EndpointRef,
    Intent, MetaValue, PayloadValue, ProtocolId, RequestOptions, RoutedResult, TransportMeta,
};

use crate::{
    BridgeFault, BridgeFaultCategory, BridgeMessage, BridgeOutcome, BridgePlannerError, Route,
    planner,
};

#[test]
fn payload_roundtrip_preserves_binary_and_structure() {
    let payload = PayloadValue::Map(vec![
        (
            "text".to_string().into(),
            PayloadValue::String("hello".to_string().into()),
        ),
        (
            "bytes".to_string().into(),
            PayloadValue::Bytes(vec![1, 2, 3].into()),
        ),
    ]);

    let roundtrip = payload.as_borrowed().into_owned();

    assert_eq!(roundtrip, payload);
}

#[test]
fn request_response_planner_preserves_correlation_and_resource() {
    let command = Command {
        id: "cmd-1".into(),
        source_device_id: Some("src".into()),
        target_device_id: "dst".into(),
        intent: Intent::Write {
            resource: "setpoint".into(),
            payload: PayloadValue::Bytes(vec![9, 1].into()),
            options: RequestOptions::default(),
        },
        correlation: Some(Correlation {
            request_id: "root".to_string().into(),
            reply_to: Some(Address::Resource("/reply".to_string().into())),
        }),
    };

    let BridgeMessage::Command(message) = planner::command_to_request_response(command).unwrap()
    else {
        panic!("expected bridge command");
    };

    assert!(matches!(
        message.route,
        Route::RequestResponse { resource, .. } if resource == "setpoint"
    ));
    assert_eq!(
        message
            .correlation
            .as_ref()
            .map(|value| value.request_id.as_ref()),
        Some("root")
    );
    assert_eq!(
        message.payload,
        Some(PayloadValue::Bytes(vec![9, 1].into()))
    );
}

#[test]
fn messaging_planner_preserves_topic_and_extension_capture() {
    let mut extensions = TransportMeta::new(ProtocolId::MQTT);
    extensions.set("response-topic", MetaValue::Str("topic/reply".into()));
    extensions.set(
        ferredge_core::prelude::META_KEY_HEADERS,
        MetaValue::Pairs(vec![("trace".to_string().into(), "abc".to_string().into())]),
    );
    let command = Command {
        id: "cmd-2".into(),
        source_device_id: None,
        target_device_id: "mqtt".into(),
        intent: Intent::Send {
            channel: ferredge_core::prelude::BrokerAddress {
                name: "topic/a".into(),
                kind: None,
            },
            payload: PayloadValue::String("hello".to_string().into()),
            options: ferredge_core::prelude::BrokerMessageOptions {
                delivery: Some(DeliveryGuarantee::AtLeastOnce),
                headers: Default::default(),
                extensions: Some(extensions),
            },
        },
        correlation: None,
    };

    let BridgeMessage::Command(message) = planner::command_to_messaging(command).unwrap() else {
        panic!("expected bridge command");
    };

    assert!(matches!(
        message.route,
        Route::Messaging { topic } if topic == "topic/a"
    ));
    let transport = message.transport.expect("mqtt transport expected");
    assert_eq!(transport.protocol, ProtocolId::MQTT);
    assert_eq!(transport.get_str("response-topic"), Some("topic/reply"));
    assert!(
        transport
            .get(ferredge_core::prelude::META_KEY_HEADERS)
            .is_none()
    );
    let headers = message
        .headers
        .expect("capture headers become envelope headers");
    assert_eq!(headers.get("trace"), Some("abc"));
}

#[test]
fn planners_reject_unsupported_intents() {
    let subscribe_command = Command {
        id: "cmd-unsupported".into(),
        source_device_id: None,
        target_device_id: "dst".into(),
        intent: Intent::Subscribe {
            channel: BrokerAddress {
                name: "alerts".into(),
                kind: None,
            },
            options: Default::default(),
        },
        correlation: None,
    };
    let read_command = Command {
        id: "cmd-read-unsupported".into(),
        source_device_id: None,
        target_device_id: "dst".into(),
        intent: Intent::Read {
            resource: "temp".into(),
            options: RequestOptions::default(),
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
            crate::AddressedAccessMeta {
                address: 1,
                domain: "holding-register".into(),
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
fn routed_result_to_bridge_preserves_progress_success_and_failure() {
    let source = EndpointRef {
        device_id: "mqtt-1".to_string(),
        protocol: ProtocolId::MQTT,
    };
    let correlation = Some(Correlation {
        request_id: "corr-1".to_string().into(),
        reply_to: Some(Address::Channel("reply/topic".to_string().into())),
    });

    let progress = RoutedResult {
        source: source.clone(),
        result: CommandResult {
            command_id: "cmd-progress".into(),
            device_id: "mqtt-1".into(),
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
            command_id: "cmd-success".into(),
            device_id: "mqtt-1".into(),
            state: ferredge_core::prelude::DeliveryState::Completed,
            payload: Some(PayloadValue::Bytes(vec![4, 2].into())),
            error: None,
            correlation: correlation.clone(),
        },
        transport: None,
    };
    let rejected = RoutedResult {
        source: source.clone(),
        result: CommandResult {
            command_id: "cmd-rejected".into(),
            device_id: "mqtt-1".into(),
            state: ferredge_core::prelude::DeliveryState::Rejected,
            payload: Some(PayloadValue::String("partial".to_string().into())),
            error: Some("denied".to_string().into()),
            correlation: correlation.clone(),
        },
        transport: None,
    };
    let timed_out = RoutedResult {
        source: source.clone(),
        result: CommandResult {
            command_id: "cmd-timeout".into(),
            device_id: "mqtt-1".into(),
            state: ferredge_core::prelude::DeliveryState::TimedOut,
            payload: None,
            error: Some("timed out".to_string().into()),
            correlation,
        },
        transport: None,
    };

    let BridgeMessage::Result(progress) = planner::routed_result_to_bridge(progress) else {
        panic!("expected bridge result");
    };
    assert!(matches!(
        progress.outcome,
        BridgeOutcome::Progress {
            state: ferredge_core::prelude::DeliveryState::Accepted,
        }
    ));
    assert_eq!(progress.meta.command_id, "cmd-progress");
    assert!(progress.meta.correlation.is_some());

    let BridgeMessage::Result(success) = planner::routed_result_to_bridge(success) else {
        panic!("expected bridge result");
    };
    assert_eq!(success.meta.command_id, "cmd-success");
    assert!(matches!(
        success.outcome,
        BridgeOutcome::Success {
            payload: Some(PayloadValue::Bytes(bytes)),
        } if bytes == vec![4, 2]
    ));

    let BridgeMessage::Result(rejected) = planner::routed_result_to_bridge(rejected) else {
        panic!("expected bridge result");
    };
    assert_eq!(rejected.meta.command_id, "cmd-rejected");
    assert!(rejected.meta.correlation.is_some());
    match rejected.outcome {
        BridgeOutcome::Failure {
            state,
            error,
            fault,
            ..
        } => {
            assert_eq!(state, ferredge_core::prelude::DeliveryState::Rejected);
            assert_eq!(error.as_deref(), Some("denied"));
            assert_eq!(fault.category, BridgeFaultCategory::Rejected);
            assert_eq!(fault.detail.as_deref(), Some("denied"));
            assert!(!fault.retryable);
            assert_eq!(
                fault.source.and_then(|source| source.protocol),
                Some(ProtocolId::MQTT)
            );
        }
        other => panic!("expected failure bridge outcome, got {other:?}"),
    }

    let BridgeMessage::Result(timed_out) = planner::routed_result_to_bridge(timed_out) else {
        panic!("expected bridge result");
    };
    assert_eq!(timed_out.meta.command_id, "cmd-timeout");
    match timed_out.outcome {
        BridgeOutcome::Failure {
            state,
            error,
            fault,
            ..
        } => {
            assert_eq!(state, ferredge_core::prelude::DeliveryState::TimedOut);
            assert_eq!(error.as_deref(), Some("timed out"));
            assert_eq!(fault.category, BridgeFaultCategory::Timeout);
            assert_eq!(fault.detail.as_deref(), Some("timed out"));
            assert!(fault.retryable);
        }
        other => panic!("expected failure bridge outcome, got {other:?}"),
    }
}

#[test]
fn bridge_fault_preserves_category_code_and_retryability() {
    let fault = BridgeFault::protocol(
        "0x02",
        true,
        ProtocolId::MODBUS,
        Some("illegal data address".to_string()),
    );

    assert_eq!(fault.category, BridgeFaultCategory::Protocol);
    assert_eq!(fault.protocol_code.as_deref(), Some("0x02"));
    assert!(fault.retryable);
}
