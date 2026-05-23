use alloc::string::String;

use ferredge_core::prelude::{
    Address, Command, CommandResult, Correlation, DeliveryState, EndpointRef, Intent, PayloadValue,
    RoutedEvent, RoutedResult,
};

use crate::{
    BridgeCapability, BridgeCommand, BridgeEvent, BridgeMessage, BridgeMeta, BridgeOp,
    BridgePayload, BridgePlannerError, BridgeProtocolHint, BridgeResult, MessagingAction,
    MessagingCapability, MessagingOp, RegisterAccessAction, RegisterAccessCapability,
    RegisterAccessOp, RegisterMeta, RequestResponseAction, RequestResponseCapability,
    RequestResponseOp,
};

/// Plans a core command into a request/response bridge command.
pub fn command_to_request_response(command: &Command) -> Result<BridgeMessage, BridgePlannerError> {
    match &command.intent {
        Intent::Read { resource } => Ok(BridgeMessage::Command(BridgeCommand {
            id: command.id.clone(),
            source_device_id: command.source_device_id.clone(),
            target_device_id: command.target_device_id.clone(),
            capability: BridgeCapability::RequestResponse(RequestResponseCapability {
                binary_payloads: true,
            }),
            operation: BridgeOp::RequestResponse(RequestResponseOp {
                action: RequestResponseAction::Read,
            }),
            payload: None,
            meta: BridgeMeta {
                resource: Some(resource.clone()),
                protocol: Some(BridgeProtocolHint::Http),
                ..BridgeMeta::default()
            },
            correlation: command.correlation.clone(),
        })),
        Intent::Write { resource, payload } => Ok(BridgeMessage::Command(BridgeCommand {
            id: command.id.clone(),
            source_device_id: command.source_device_id.clone(),
            target_device_id: command.target_device_id.clone(),
            capability: BridgeCapability::RequestResponse(RequestResponseCapability {
                binary_payloads: true,
            }),
            operation: BridgeOp::RequestResponse(RequestResponseOp {
                action: RequestResponseAction::Write,
            }),
            payload: Some(BridgePayload::from(payload.clone())),
            meta: BridgeMeta {
                resource: Some(resource.clone()),
                protocol: Some(BridgeProtocolHint::Http),
                ..BridgeMeta::default()
            },
            correlation: command.correlation.clone(),
        })),
        Intent::Invoke { operation, args } => Ok(BridgeMessage::Command(BridgeCommand {
            id: command.id.clone(),
            source_device_id: command.source_device_id.clone(),
            target_device_id: command.target_device_id.clone(),
            capability: BridgeCapability::RequestResponse(RequestResponseCapability {
                binary_payloads: true,
            }),
            operation: BridgeOp::RequestResponse(RequestResponseOp {
                action: RequestResponseAction::Invoke,
            }),
            payload: args.clone().map(BridgePayload::from),
            meta: BridgeMeta {
                resource: Some(operation.clone()),
                protocol: Some(BridgeProtocolHint::Http),
                ..BridgeMeta::default()
            },
            correlation: command.correlation.clone(),
        })),
        _ => Err(BridgePlannerError::UnsupportedIntent),
    }
}

/// Plans a core command into a messaging bridge command.
pub fn command_to_messaging(command: &Command) -> Result<BridgeMessage, BridgePlannerError> {
    match &command.intent {
        Intent::Send {
            channel,
            payload,
            options,
        } => Ok(BridgeMessage::Command(BridgeCommand {
            id: command.id.clone(),
            source_device_id: command.source_device_id.clone(),
            target_device_id: command.target_device_id.clone(),
            capability: BridgeCapability::Messaging(MessagingCapability {
                binary_payloads: true,
            }),
            operation: BridgeOp::Messaging(MessagingOp {
                action: MessagingAction::Publish,
            }),
            payload: Some(BridgePayload::from(payload.clone())),
            meta: BridgeMeta {
                topic: Some(channel.name.clone()),
                correlation_id: options.correlation_id.clone(),
                reply_to: options.reply_to.clone().map(Address::Channel),
                protocol: Some(BridgeProtocolHint::Mqtt),
                ..BridgeMeta::default()
            },
            correlation: command.correlation.clone(),
        })),
        Intent::Subscribe { channel, options } => Ok(BridgeMessage::Command(BridgeCommand {
            id: command.id.clone(),
            source_device_id: command.source_device_id.clone(),
            target_device_id: command.target_device_id.clone(),
            capability: BridgeCapability::Messaging(MessagingCapability {
                binary_payloads: true,
            }),
            operation: BridgeOp::Messaging(MessagingOp {
                action: MessagingAction::Subscribe,
            }),
            payload: None,
            meta: BridgeMeta {
                topic: Some(channel.name.clone()),
                correlation_id: options.durable_name.clone(),
                protocol: Some(BridgeProtocolHint::Mqtt),
                ..BridgeMeta::default()
            },
            correlation: command.correlation.clone(),
        })),
        Intent::Unsubscribe { channel } => Ok(BridgeMessage::Command(BridgeCommand {
            id: command.id.clone(),
            source_device_id: command.source_device_id.clone(),
            target_device_id: command.target_device_id.clone(),
            capability: BridgeCapability::Messaging(MessagingCapability {
                binary_payloads: true,
            }),
            operation: BridgeOp::Messaging(MessagingOp {
                action: MessagingAction::Unsubscribe,
            }),
            payload: None,
            meta: BridgeMeta {
                topic: Some(channel.name.clone()),
                protocol: Some(BridgeProtocolHint::Mqtt),
                ..BridgeMeta::default()
            },
            correlation: command.correlation.clone(),
        })),
        _ => Err(BridgePlannerError::UnsupportedIntent),
    }
}

/// Plans a core command into a register-access bridge command.
pub fn command_to_register_access(
    command: &Command,
    register: RegisterMeta,
    unit_id: u8,
) -> Result<BridgeMessage, BridgePlannerError> {
    match &command.intent {
        Intent::Read { resource } => Ok(BridgeMessage::Command(BridgeCommand {
            id: command.id.clone(),
            source_device_id: command.source_device_id.clone(),
            target_device_id: command.target_device_id.clone(),
            capability: BridgeCapability::RegisterAccess(RegisterAccessCapability {
                binary_payloads: true,
            }),
            operation: BridgeOp::RegisterAccess(RegisterAccessOp {
                action: RegisterAccessAction::Read,
            }),
            payload: None,
            meta: BridgeMeta {
                resource: Some(resource.clone()),
                register: Some(register),
                unit_id: Some(unit_id),
                protocol: Some(BridgeProtocolHint::Modbus),
                ..BridgeMeta::default()
            },
            correlation: command.correlation.clone(),
        })),
        Intent::Write { resource, payload } => Ok(BridgeMessage::Command(BridgeCommand {
            id: command.id.clone(),
            source_device_id: command.source_device_id.clone(),
            target_device_id: command.target_device_id.clone(),
            capability: BridgeCapability::RegisterAccess(RegisterAccessCapability {
                binary_payloads: true,
            }),
            operation: BridgeOp::RegisterAccess(RegisterAccessOp {
                action: RegisterAccessAction::Write,
            }),
            payload: Some(BridgePayload::from(payload.clone())),
            meta: BridgeMeta {
                resource: Some(resource.clone()),
                register: Some(register),
                unit_id: Some(unit_id),
                protocol: Some(BridgeProtocolHint::Modbus),
                ..BridgeMeta::default()
            },
            correlation: command.correlation.clone(),
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
pub fn routed_result_to_bridge(result: &RoutedResult) -> BridgeMessage {
    BridgeMessage::Result(BridgeResult {
        source: result.source.clone(),
        command_id: result.result.command_id.clone(),
        state: result.result.state.clone(),
        capability: None,
        operation: None,
        payload: result.result.payload.clone().map(BridgePayload::from),
        meta: BridgeMeta::default(),
        correlation: result.result.correlation.clone(),
    })
}

/// Wraps a routed core event in a bridge event envelope.
pub fn routed_event_to_bridge(event: &RoutedEvent) -> BridgeMessage {
    let topic = match &event.address {
        Address::Channel(topic) => Some(topic.clone()),
        Address::Resource(path) => Some(path.clone()),
    };

    BridgeMessage::Event(BridgeEvent {
        source: event.source.clone(),
        capability: BridgeCapability::Messaging(MessagingCapability {
            binary_payloads: true,
        }),
        operation: BridgeOp::Messaging(MessagingOp {
            action: MessagingAction::Publish,
        }),
        payload: BridgePayload::from(event.payload.clone()),
        meta: BridgeMeta {
            topic,
            ..BridgeMeta::default()
        },
        correlation: event.correlation.clone(),
    })
}
