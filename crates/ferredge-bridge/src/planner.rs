use alloc::{borrow::Cow, string::ToString};

use ferredge_core::prelude::{
    AddressedAccessMeta, Command, CommandResult, Correlation, DeliveryState, EndpointRef, Headers,
    Intent, PayloadValue, Route, RoutedEvent, RoutedResult, TransportMeta,
};

use crate::{
    BridgeCapability, BridgeCommand, BridgeEvent, BridgeFault, BridgeFaultCategory, BridgeMessage,
    BridgeMessageMeta, BridgeOp, BridgeOutcome, BridgePlannerError, BridgeResult, MessagingAction,
    MessagingCapability, MessagingOp, RegisterAccessAction, RegisterAccessCapability,
    RegisterAccessOp, RequestResponseAction, RequestResponseCapability, RequestResponseOp,
};

/// Plans a core command into a request/response bridge command.
pub fn command_to_request_response<'a>(
    command: Command<'a>,
) -> Result<BridgeMessage<'a>, BridgePlannerError> {
    let Command {
        id,
        source_device_id,
        target_device_id,
        intent,
        correlation,
    } = command;
    let (resource, options, action, payload) = match intent {
        Intent::Read { resource, options } => {
            (resource, options, RequestResponseAction::Read, None)
        }
        Intent::Write {
            resource,
            payload,
            options,
        } => (
            resource,
            options,
            RequestResponseAction::Write,
            Some(payload),
        ),
        Intent::Invoke {
            operation,
            args,
            options,
        } => (operation, options, RequestResponseAction::Invoke, args),
        _ => return Err(BridgePlannerError::UnsupportedIntent),
    };
    let ferredge_core::prelude::RequestOptions {
        headers,
        content_type,
        method,
        path,
        extensions,
    } = options;
    let (transport, headers) = capture_with_headers(extensions, headers);

    Ok(BridgeMessage::Command(BridgeCommand {
        id,
        source_device_id,
        target_device_id,
        capability: BridgeCapability::RequestResponse(RequestResponseCapability {
            binary_payloads: true,
        }),
        operation: BridgeOp::RequestResponse(RequestResponseOp {
            action,
            method,
            content_type,
        }),
        payload,
        route: Route::RequestResponse { resource, path },
        transport,
        headers,
        correlation,
    }))
}

/// Plans a core command into a messaging bridge command.
pub fn command_to_messaging<'a>(
    command: Command<'a>,
) -> Result<BridgeMessage<'a>, BridgePlannerError> {
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
        } => {
            let ferredge_core::prelude::BrokerMessageOptions {
                delivery,
                headers,
                extensions,
            } = options;
            let (transport, headers) = capture_with_headers(extensions, headers);
            Ok(BridgeMessage::Command(BridgeCommand {
                id,
                source_device_id,
                target_device_id,
                capability: BridgeCapability::Messaging(MessagingCapability {
                    binary_payloads: true,
                }),
                operation: BridgeOp::Messaging(MessagingOp {
                    action: MessagingAction::Publish,
                    delivery,
                    durable_name: None,
                    shared_group: None,
                }),
                payload: Some(payload),
                route: Route::Messaging {
                    topic: channel.name,
                },
                transport,
                headers,
                correlation,
            }))
        }
        Intent::Subscribe { channel, options } => {
            let ferredge_core::prelude::BrokerSubscriptionOptions {
                delivery,
                durable_name,
                shared_group,
                extensions,
            } = options;
            let (transport, headers) = capture_with_headers(extensions, Headers::default());
            Ok(BridgeMessage::Command(BridgeCommand {
                id,
                source_device_id,
                target_device_id,
                capability: BridgeCapability::Messaging(MessagingCapability {
                    binary_payloads: true,
                }),
                operation: BridgeOp::Messaging(MessagingOp {
                    action: MessagingAction::Subscribe,
                    delivery,
                    durable_name,
                    shared_group,
                }),
                payload: None,
                route: Route::Messaging {
                    topic: channel.name,
                },
                transport,
                headers,
                correlation,
            }))
        }
        Intent::Unsubscribe { channel } => Ok(BridgeMessage::Command(BridgeCommand {
            id,
            source_device_id,
            target_device_id,
            capability: BridgeCapability::Messaging(MessagingCapability {
                binary_payloads: true,
            }),
            operation: BridgeOp::Messaging(MessagingOp {
                action: MessagingAction::Unsubscribe,
                delivery: None,
                durable_name: None,
                shared_group: None,
            }),
            payload: None,
            route: Route::Messaging {
                topic: channel.name,
            },
            transport: None,
            headers: None,
            correlation,
        })),
        _ => Err(BridgePlannerError::UnsupportedIntent),
    }
}

/// Plans a core command into a register-access bridge command.
pub fn command_to_register_access<'a>(
    command: Command<'a>,
    register: AddressedAccessMeta,
    unit_id: u8,
) -> Result<BridgeMessage<'a>, BridgePlannerError> {
    let Command {
        id,
        source_device_id,
        target_device_id,
        intent,
        correlation,
    } = command;
    match intent {
        Intent::Read { resource, .. } => Ok(bridge_register_command(
            id,
            source_device_id,
            target_device_id,
            resource,
            register,
            unit_id,
            RegisterAccessAction::Read,
            None,
            correlation,
        )),
        Intent::Write {
            resource, payload, ..
        } => Ok(bridge_register_command(
            id,
            source_device_id,
            target_device_id,
            resource,
            register,
            unit_id,
            RegisterAccessAction::Write,
            Some(payload),
            correlation,
        )),
        _ => Err(BridgePlannerError::UnsupportedIntent),
    }
}

/// Maps inbound messaging semantics back into a routed core event.
pub fn inbound_messaging_event<'a>(
    source: EndpointRef,
    topic: Cow<'a, str>,
    payload: PayloadValue<'a>,
    correlation: Option<Correlation<'a>>,
    _content_type: Option<Cow<'a, str>>,
) -> RoutedEvent<'a> {
    RoutedEvent {
        source,
        route: Route::Messaging { topic },
        payload,
        correlation,
        transport: None,
    }
}

/// Wraps a routed core result in the bridge result envelope.
pub fn routed_result_to_bridge<'a>(result: RoutedResult<'a>) -> BridgeMessage<'a> {
    let RoutedResult {
        source,
        result,
        transport,
    } = result;
    let CommandResult {
        command_id,
        device_id: _,
        state,
        payload,
        error,
        correlation,
    } = result;
    let (transport, headers) = split_capture_headers(transport);
    let protocol = source.protocol.clone();
    let meta = BridgeMessageMeta {
        source,
        command_id,
        capability: None,
        operation: None,
        transport,
        headers,
        correlation,
    };

    let outcome = match state {
        state @ (DeliveryState::Accepted | DeliveryState::Dispatched) => {
            BridgeOutcome::Progress { state }
        }
        DeliveryState::Completed => BridgeOutcome::Success { payload },
        state @ (DeliveryState::Rejected | DeliveryState::TimedOut) => {
            let (category, retryable) = if state == DeliveryState::TimedOut {
                (BridgeFaultCategory::Timeout, true)
            } else {
                (BridgeFaultCategory::Rejected, false)
            };
            let fault_detail = error.as_ref().map(|value| value.to_string());
            BridgeOutcome::Failure {
                state,
                payload,
                error,
                fault: BridgeFault {
                    category,
                    protocol_code: None,
                    retryable,
                    source: Some(crate::fault::BridgeFaultSource {
                        protocol: Some(protocol),
                        location: None,
                    }),
                    detail: fault_detail,
                },
            }
        }
    };

    BridgeMessage::Result(BridgeResult { meta, outcome })
}

/// Wraps a routed core event in a bridge event envelope.
pub fn routed_event_to_bridge<'a>(event: RoutedEvent<'a>) -> BridgeMessage<'a> {
    let RoutedEvent {
        source,
        route,
        payload,
        correlation,
        transport,
    } = event;
    let (transport, headers) = split_capture_headers(transport);

    BridgeMessage::Event(BridgeEvent {
        source,
        capability: BridgeCapability::Messaging(MessagingCapability {
            binary_payloads: true,
        }),
        operation: BridgeOp::Messaging(MessagingOp {
            action: MessagingAction::Publish,
            delivery: None,
            durable_name: None,
            shared_group: None,
        }),
        payload,
        route,
        transport,
        headers,
        correlation,
    })
}

fn bridge_register_command<'a>(
    id: Cow<'a, str>,
    source_device_id: Option<Cow<'a, str>>,
    target_device_id: Cow<'a, str>,
    resource: Cow<'a, str>,
    register: AddressedAccessMeta,
    unit_id: u8,
    action: RegisterAccessAction,
    payload: Option<PayloadValue<'a>>,
    correlation: Option<Correlation<'a>>,
) -> BridgeMessage<'a> {
    BridgeMessage::Command(BridgeCommand {
        id,
        source_device_id,
        target_device_id,
        capability: BridgeCapability::RegisterAccess(RegisterAccessCapability {
            binary_payloads: true,
        }),
        operation: BridgeOp::RegisterAccess(RegisterAccessOp { action }),
        payload,
        route: Route::AddressedAccess {
            resource,
            access: register,
            node_id: Some(u32::from(unit_id)),
        },
        transport: None,
        headers: None,
        correlation,
    })
}

/// Splits the capture-carried headers away from the passthrough capture.
fn split_capture_headers<'a>(
    transport: Option<TransportMeta<'a>>,
) -> (Option<TransportMeta<'a>>, Option<Headers<'a>>) {
    match transport {
        Some(mut meta) => {
            let headers = meta.headers.take().filter(|headers| !headers.is_empty());
            (Some(meta), headers)
        }
        None => (None, None),
    }
}

/// Merges capture-carried header pairs after the envelope set, dropping already-present keys.
fn capture_with_headers<'a>(
    transport: Option<TransportMeta<'a>>,
    mut headers: Headers<'a>,
) -> (Option<TransportMeta<'a>>, Option<Headers<'a>>) {
    let (transport, capture_headers) = split_capture_headers(transport);
    if let Some(capture_headers) = capture_headers {
        headers.merge(capture_headers);
    }
    (transport, (!headers.is_empty()).then_some(headers))
}
