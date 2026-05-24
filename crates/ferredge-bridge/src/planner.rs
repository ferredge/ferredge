use alloc::string::String;

use ferredge_core::prelude::{
    Address, Command, CommandResult, Correlation, DeliveryState, EndpointRef, Intent, PayloadValue,
    RoutedEvent, RoutedResult,
};

use crate::BridgeResult::{Failure, Progress, Success};
use crate::{
    BridgeCapability, BridgeCommand, BridgeEvent, BridgeFault, BridgeFaultCategory, BridgeMessage,
    BridgeMeta, BridgeOp, BridgePayload, BridgePlannerError, BridgeProtocolHint, MessagingAction,
    MessagingCapability, MessagingOp, RegisterAccessAction, RegisterAccessCapability,
    RegisterAccessOp, RegisterMeta, RequestResponseAction, RequestResponseCapability,
    RequestResponseOp,
};

/// Plans a core command into a request/response bridge command.
pub fn command_to_request_response(command: Command) -> Result<BridgeMessage, BridgePlannerError> {
    let Command {
        id,
        source_device_id,
        target_device_id,
        intent,
        correlation,
    } = command;

    match intent {
        Intent::Read { resource } => Ok(BridgeMessage::Command(BridgeCommand {
            id,
            source_device_id,
            target_device_id,
            capability: BridgeCapability::RequestResponse(RequestResponseCapability {
                binary_payloads: true,
            }),
            operation: BridgeOp::RequestResponse(RequestResponseOp {
                action: RequestResponseAction::Read,
            }),
            payload: None,
            meta: BridgeMeta {
                resource: Some(resource),
                protocol: Some(BridgeProtocolHint::Http),
                ..BridgeMeta::default()
            },
            correlation,
        })),
        Intent::Write { resource, payload } => Ok(BridgeMessage::Command(BridgeCommand {
            id,
            source_device_id,
            target_device_id,
            capability: BridgeCapability::RequestResponse(RequestResponseCapability {
                binary_payloads: true,
            }),
            operation: BridgeOp::RequestResponse(RequestResponseOp {
                action: RequestResponseAction::Write,
            }),
            payload: Some(BridgePayload::from(payload)),
            meta: BridgeMeta {
                resource: Some(resource),
                protocol: Some(BridgeProtocolHint::Http),
                ..BridgeMeta::default()
            },
            correlation,
        })),
        Intent::Invoke { operation, args } => Ok(BridgeMessage::Command(BridgeCommand {
            id,
            source_device_id,
            target_device_id,
            capability: BridgeCapability::RequestResponse(RequestResponseCapability {
                binary_payloads: true,
            }),
            operation: BridgeOp::RequestResponse(RequestResponseOp {
                action: RequestResponseAction::Invoke,
            }),
            payload: args.map(BridgePayload::from),
            meta: BridgeMeta {
                resource: Some(operation),
                protocol: Some(BridgeProtocolHint::Http),
                ..BridgeMeta::default()
            },
            correlation,
        })),
        _ => Err(BridgePlannerError::UnsupportedIntent),
    }
}

/// Plans a core command into a messaging bridge command.
pub fn command_to_messaging(command: Command) -> Result<BridgeMessage, BridgePlannerError> {
    let Command {
        id,
        source_device_id,
        target_device_id,
        intent,
        correlation,
    } = command;

    match intent {
        Intent::Send {
            channel,
            payload,
            options,
        } => Ok(BridgeMessage::Command(BridgeCommand {
            id,
            source_device_id,
            target_device_id,
            capability: BridgeCapability::Messaging(MessagingCapability {
                binary_payloads: true,
            }),
            operation: BridgeOp::Messaging(MessagingOp {
                action: MessagingAction::Publish,
            }),
            payload: Some(BridgePayload::from(payload)),
            meta: BridgeMeta {
                topic: Some(channel.name),
                correlation_id: options.correlation_id,
                reply_to: options.reply_to.map(Address::Channel),
                protocol: Some(BridgeProtocolHint::Mqtt),
                ..BridgeMeta::default()
            },
            correlation,
        })),
        Intent::Subscribe { channel, options } => Ok(BridgeMessage::Command(BridgeCommand {
            id,
            source_device_id,
            target_device_id,
            capability: BridgeCapability::Messaging(MessagingCapability {
                binary_payloads: true,
            }),
            operation: BridgeOp::Messaging(MessagingOp {
                action: MessagingAction::Subscribe,
            }),
            payload: None,
            meta: BridgeMeta {
                topic: Some(channel.name),
                correlation_id: options.durable_name,
                protocol: Some(BridgeProtocolHint::Mqtt),
                ..BridgeMeta::default()
            },
            correlation,
        })),
        Intent::Unsubscribe { channel } => Ok(BridgeMessage::Command(BridgeCommand {
            id,
            source_device_id,
            target_device_id,
            capability: BridgeCapability::Messaging(MessagingCapability {
                binary_payloads: true,
            }),
            operation: BridgeOp::Messaging(MessagingOp {
                action: MessagingAction::Unsubscribe,
            }),
            payload: None,
            meta: BridgeMeta {
                topic: Some(channel.name),
                protocol: Some(BridgeProtocolHint::Mqtt),
                ..BridgeMeta::default()
            },
            correlation,
        })),
        _ => Err(BridgePlannerError::UnsupportedIntent),
    }
}

/// Plans a core command into a register-access bridge command.
pub fn command_to_register_access(
    command: Command,
    register: RegisterMeta,
    unit_id: u8,
) -> Result<BridgeMessage, BridgePlannerError> {
    let Command {
        id,
        source_device_id,
        target_device_id,
        intent,
        correlation,
    } = command;

    match intent {
        Intent::Read { resource } => Ok(BridgeMessage::Command(BridgeCommand {
            id,
            source_device_id,
            target_device_id,
            capability: BridgeCapability::RegisterAccess(RegisterAccessCapability {
                binary_payloads: true,
            }),
            operation: BridgeOp::RegisterAccess(RegisterAccessOp {
                action: RegisterAccessAction::Read,
            }),
            payload: None,
            meta: BridgeMeta {
                resource: Some(resource),
                register: Some(register),
                unit_id: Some(unit_id),
                protocol: Some(BridgeProtocolHint::Modbus),
                ..BridgeMeta::default()
            },
            correlation,
        })),
        Intent::Write { resource, payload } => Ok(BridgeMessage::Command(BridgeCommand {
            id,
            source_device_id,
            target_device_id,
            capability: BridgeCapability::RegisterAccess(RegisterAccessCapability {
                binary_payloads: true,
            }),
            operation: BridgeOp::RegisterAccess(RegisterAccessOp {
                action: RegisterAccessAction::Write,
            }),
            payload: Some(BridgePayload::from(payload)),
            meta: BridgeMeta {
                resource: Some(resource),
                register: Some(register),
                unit_id: Some(unit_id),
                protocol: Some(BridgeProtocolHint::Modbus),
                ..BridgeMeta::default()
            },
            correlation,
        })),
        _ => Err(BridgePlannerError::UnsupportedIntent),
    }
}

/// Maps inbound messaging semantics back into a routed core event.
pub fn inbound_messaging_event(
    source: EndpointRef,
    topic: String,
    payload: BridgePayload,
    correlation: Option<Correlation>,
    _content_type: Option<String>,
) -> RoutedEvent {
    RoutedEvent {
        source,
        address: Address::Channel(topic),
        payload: payload.into(),
        correlation,
        transport: None,
    }
}

/// Maps inbound register semantics back into a routed core result.
pub fn inbound_register_result(
    source: EndpointRef,
    command_id: String,
    payload: Option<BridgePayload>,
    correlation: Option<Correlation>,
) -> RoutedResult {
    RoutedResult {
        source: source.clone(),
        result: CommandResult {
            command_id,
            device_id: source.device_id,
            state: DeliveryState::Completed,
            payload: payload.map(PayloadValue::from),
            error: None,
            correlation,
        },
        transport: None,
    }
}

/// Wraps a routed core result in the bridge result envelope.
pub fn routed_result_to_bridge(result: RoutedResult) -> BridgeMessage {
    let RoutedResult {
        source,
        result,
        transport: _,
    } = result;
    let CommandResult {
        command_id,
        device_id: _,
        state,
        payload,
        error,
        correlation,
    } = result;
    let payload = payload.map(BridgePayload::from);
    let meta = BridgeMeta::default();
    let capability = None;
    let operation = None;
    let protocol = source.protocol.clone();

    let bridge_result = match state {
        DeliveryState::Accepted => Progress {
            source,
            command_id,
            state: DeliveryState::Accepted,
            capability,
            operation,
            meta,
            correlation,
        },
        DeliveryState::Dispatched => Progress {
            source,
            command_id,
            state: DeliveryState::Dispatched,
            capability,
            operation,
            meta,
            correlation,
        },
        DeliveryState::Completed => Success {
            source,
            command_id,
            capability,
            operation,
            payload,
            meta,
            correlation,
        },
        DeliveryState::Rejected => {
            let fault_detail = error.clone();
            Failure {
                source,
                command_id,
                state: DeliveryState::Rejected,
                capability,
                operation,
                payload,
                error,
                meta,
                correlation,
                fault: BridgeFault {
                    category: BridgeFaultCategory::Rejected,
                    protocol_code: None,
                    retryable: false,
                    source: Some(crate::fault::BridgeFaultSource {
                        protocol: Some(protocol),
                        location: None,
                    }),
                    detail: fault_detail,
                },
            }
        }
        DeliveryState::TimedOut => {
            let fault_detail = error.clone();
            Failure {
                source,
                command_id,
                state: DeliveryState::TimedOut,
                capability,
                operation,
                payload,
                error,
                meta,
                correlation,
                fault: BridgeFault {
                    category: BridgeFaultCategory::Timeout,
                    protocol_code: None,
                    retryable: true,
                    source: Some(crate::fault::BridgeFaultSource {
                        protocol: Some(protocol),
                        location: None,
                    }),
                    detail: fault_detail,
                },
            }
        }
    };

    BridgeMessage::Result(bridge_result)
}

/// Wraps a routed core event in a bridge event envelope.
pub fn routed_event_to_bridge(event: RoutedEvent) -> BridgeMessage {
    let RoutedEvent {
        source,
        address,
        payload,
        correlation,
        transport: _,
    } = event;
    let topic = match address {
        Address::Channel(topic) => Some(topic),
        Address::Resource(path) => Some(path),
    };

    BridgeMessage::Event(BridgeEvent {
        source,
        capability: BridgeCapability::Messaging(MessagingCapability {
            binary_payloads: true,
        }),
        operation: BridgeOp::Messaging(MessagingOp {
            action: MessagingAction::Publish,
        }),
        payload: BridgePayload::from(payload),
        meta: BridgeMeta {
            topic,
            ..BridgeMeta::default()
        },
        correlation,
    })
}
