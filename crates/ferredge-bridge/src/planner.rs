use alloc::{borrow::Cow, string::String};

use ferredge_core::prelude::{
    Address, BrokerMessageProtocolOptions, BrokerSubscriptionProtocolOptions, Command,
    CommandResult, Correlation, DeliveryGuarantee, DeliveryState, EndpointRef, Intent,
    MqttPayloadFormat, PayloadValue, RoutedEvent, RoutedResult, TransportMeta,
};

use crate::BridgeResult::{Failure, Progress, Success};
use crate::{
    AddressedAccessMeta, BridgeCapability, BridgeCommand, BridgeCorrelation, BridgeEvent,
    BridgeFault, BridgeFaultCategory, BridgeHeaders, BridgeMessage, BridgeOp, BridgePayload,
    BridgePlannerError, BridgeRoute, BridgeTransportMeta, HttpBridgeMeta, MessagingAction,
    MessagingCapability, MessagingOp, MqttBridgeMeta, RegisterAccessAction,
    RegisterAccessCapability, RegisterAccessOp, RequestResponseAction, RequestResponseCapability,
    RequestResponseOp,
};

/// Plans a core command into a request/response bridge command.
pub fn command_to_request_response(
    command: Command,
) -> Result<BridgeMessage<'static>, BridgePlannerError> {
    let Command {
        id,
        source_device_id,
        target_device_id,
        intent,
        correlation,
    } = command;
    let (route, operation, payload, transport, headers) = match intent {
        Intent::Read { resource, options } => request_response_parts(
            Cow::Owned(resource),
            options,
            RequestResponseAction::Read,
            None,
        ),
        Intent::Write {
            resource,
            payload,
            options,
        } => request_response_parts(
            Cow::Owned(resource),
            options,
            RequestResponseAction::Write,
            Some(BridgePayload::from(payload)),
        ),
        Intent::Invoke {
            operation,
            args,
            options,
        } => request_response_parts(
            Cow::Owned(operation),
            options,
            RequestResponseAction::Invoke,
            args.map(BridgePayload::from),
        ),
        _ => return Err(BridgePlannerError::UnsupportedIntent),
    };

    Ok(BridgeMessage::Command(BridgeCommand {
        id,
        source_device_id,
        target_device_id,
        capability: BridgeCapability::RequestResponse(RequestResponseCapability {
            binary_payloads: true,
        }),
        operation: BridgeOp::RequestResponse(RequestResponseOp { action: operation }),
        payload,
        route,
        transport: Some(BridgeTransportMeta::Http(transport)),
        headers,
        correlation: bridge_correlation(correlation),
    }))
}

/// Plans a core command into a messaging bridge command.
pub fn command_to_messaging(
    command: Command,
) -> Result<BridgeMessage<'static>, BridgePlannerError> {
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
            let mqtt = match options.protocol {
                Some(BrokerMessageProtocolOptions::Mqtt(mqtt)) => Some(mqtt),
                None => None,
            };
            let mut headers = options.headers;
            if let Some(mqtt) = mqtt.as_ref() {
                let user_properties = mqtt.user_properties.clone();
                headers.extend(user_properties);
            }
            Ok(BridgeMessage::Command(BridgeCommand {
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
                route: BridgeRoute::Messaging {
                    topic: Cow::Owned(channel.name),
                },
                transport: Some(BridgeTransportMeta::Mqtt(MqttBridgeMeta {
                    qos: options.delivery.map(qos_from_delivery),
                    retain: mqtt.as_ref().is_some_and(|mqtt| mqtt.retain),
                    duplicate: false,
                    packet_id: None,
                    content_type: mqtt
                        .as_ref()
                        .and_then(|mqtt| mqtt.content_type.clone())
                        .map(Cow::Owned),
                    payload_format: mqtt
                        .as_ref()
                        .and_then(|mqtt| mqtt.payload_format.as_ref())
                        .map(mqtt_payload_format_name),
                    message_expiry_interval_secs: mqtt
                        .as_ref()
                        .and_then(|mqtt| mqtt.message_expiry_interval_secs),
                    response_topic: mqtt
                        .as_ref()
                        .and_then(|mqtt| mqtt.response_topic.clone())
                        .or(options.reply_to.clone())
                        .map(Cow::Owned),
                    correlation_data: options.correlation_id.clone().map(Cow::Owned),
                    correlation_data_bytes: mqtt
                        .as_ref()
                        .and_then(|mqtt| mqtt.correlation_data.clone()),
                    topic_alias: mqtt.as_ref().and_then(|mqtt| mqtt.topic_alias),
                    subscription_identifiers: alloc::vec::Vec::new(),
                    reason_codes: alloc::vec::Vec::new(),
                    reason_string: None,
                    durable_name: None,
                    shared_group: None,
                    no_local: false,
                    retain_as_published: false,
                    retain_handling: None,
                })),
                headers: (!headers.is_empty()).then(|| BridgeHeaders::mqtt(headers)),
                correlation: bridge_correlation(correlation),
            }))
        }
        Intent::Subscribe { channel, options } => {
            let mqtt = match options.protocol {
                Some(BrokerSubscriptionProtocolOptions::Mqtt(mqtt)) => Some(mqtt),
                None => None,
            };
            Ok(BridgeMessage::Command(BridgeCommand {
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
                route: BridgeRoute::Messaging {
                    topic: Cow::Owned(channel.name),
                },
                transport: Some(BridgeTransportMeta::Mqtt(MqttBridgeMeta {
                    qos: options.delivery.map(qos_from_delivery),
                    retain: false,
                    duplicate: false,
                    packet_id: None,
                    content_type: None,
                    payload_format: None,
                    message_expiry_interval_secs: None,
                    response_topic: None,
                    correlation_data: None,
                    correlation_data_bytes: None,
                    topic_alias: None,
                    subscription_identifiers: mqtt
                        .as_ref()
                        .and_then(|mqtt| mqtt.subscription_identifier)
                        .into_iter()
                        .collect(),
                    reason_codes: alloc::vec::Vec::new(),
                    reason_string: None,
                    durable_name: options.durable_name.map(Cow::Owned),
                    shared_group: options.shared_group.map(Cow::Owned),
                    no_local: mqtt.as_ref().is_some_and(|mqtt| mqtt.no_local),
                    retain_as_published: mqtt.as_ref().is_some_and(|mqtt| mqtt.retain_as_published),
                    retain_handling: mqtt.as_ref().and_then(|mqtt| mqtt.retain_handling),
                })),
                headers: mqtt
                    .as_ref()
                    .filter(|mqtt| !mqtt.user_properties.is_empty())
                    .map(|mqtt| BridgeHeaders::mqtt(mqtt.user_properties.clone())),
                correlation: bridge_correlation(correlation),
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
            }),
            payload: None,
            route: BridgeRoute::Messaging {
                topic: Cow::Owned(channel.name),
            },
            transport: Some(BridgeTransportMeta::Mqtt(MqttBridgeMeta::default())),
            headers: None,
            correlation: bridge_correlation(correlation),
        })),
        _ => Err(BridgePlannerError::UnsupportedIntent),
    }
}

/// Plans a core command into a register-access bridge command.
pub fn command_to_register_access(
    command: Command,
    register: AddressedAccessMeta,
    unit_id: u8,
) -> Result<BridgeMessage<'static>, BridgePlannerError> {
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
            Cow::Owned(resource),
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
            Cow::Owned(resource),
            register,
            unit_id,
            RegisterAccessAction::Write,
            Some(BridgePayload::from(payload)),
            correlation,
        )),
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
pub fn routed_result_to_bridge(result: RoutedResult) -> BridgeMessage<'static> {
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
    let (transport, headers, route) = owned_bridge_transport_parts(transport, None, None);
    let payload = payload.map(BridgePayload::from);
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
            route,
            transport,
            headers,
            correlation: bridge_correlation(correlation),
        },
        DeliveryState::Dispatched => Progress {
            source,
            command_id,
            state: DeliveryState::Dispatched,
            capability,
            operation,
            route,
            transport,
            headers,
            correlation: bridge_correlation(correlation),
        },
        DeliveryState::Completed => Success {
            source,
            command_id,
            capability,
            operation,
            payload,
            route,
            transport,
            headers,
            correlation: bridge_correlation(correlation),
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
                route,
                transport,
                headers,
                correlation: bridge_correlation(correlation),
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
                route,
                transport,
                headers,
                correlation: bridge_correlation(correlation),
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
pub fn routed_event_to_bridge(event: RoutedEvent) -> BridgeMessage<'static> {
    let RoutedEvent {
        source,
        address,
        payload,
        correlation,
        transport: event_transport,
    } = event;
    let route = owned_bridge_route_from_address(&address, event_transport.as_ref());
    let (transport, headers, _) =
        owned_bridge_transport_parts(event_transport, None, Some(&address));

    BridgeMessage::Event(BridgeEvent {
        source,
        capability: BridgeCapability::Messaging(MessagingCapability {
            binary_payloads: true,
        }),
        operation: BridgeOp::Messaging(MessagingOp {
            action: MessagingAction::Publish,
        }),
        payload: BridgePayload::from(payload),
        route: route.unwrap_or_else(|| BridgeRoute::Messaging {
            topic: Cow::Owned(String::new()),
        }),
        transport,
        headers,
        correlation: bridge_correlation(correlation),
    })
}

fn bridge_register_command(
    id: String,
    source_device_id: Option<String>,
    target_device_id: String,
    resource: Cow<'static, str>,
    register: AddressedAccessMeta,
    unit_id: u8,
    action: RegisterAccessAction,
    payload: Option<BridgePayload>,
    correlation: Option<Correlation>,
) -> BridgeMessage<'static> {
    BridgeMessage::Command(BridgeCommand {
        id,
        source_device_id,
        target_device_id,
        capability: BridgeCapability::RegisterAccess(RegisterAccessCapability {
            binary_payloads: true,
        }),
        operation: BridgeOp::RegisterAccess(RegisterAccessOp { action }),
        payload,
        route: BridgeRoute::AddressedAccess {
            resource,
            access: register,
            node_id: Some(u32::from(unit_id)),
        },
        transport: None,
        headers: None,
        correlation: bridge_correlation(correlation),
    })
}

fn request_response_parts(
    resource: Cow<'static, str>,
    options: ferredge_core::prelude::RequestOptions,
    action: RequestResponseAction,
    payload: Option<BridgePayload>,
) -> (
    BridgeRoute<'static>,
    RequestResponseAction,
    Option<BridgePayload>,
    HttpBridgeMeta<'static>,
    Option<BridgeHeaders<'static>>,
) {
    let headers = (!options.headers.is_empty()).then(|| BridgeHeaders::http(options.headers));
    (
        BridgeRoute::RequestResponse {
            resource,
            path: options.path.clone().map(Cow::Owned),
        },
        action,
        payload,
        HttpBridgeMeta {
            method: options.method.map(Cow::Owned),
            path: options.path.map(Cow::Owned),
            status_code: None,
            content_type: options.content_type.map(Cow::Owned),
        },
        headers,
    )
}

fn bridge_correlation(correlation: Option<Correlation>) -> Option<BridgeCorrelation<'static>> {
    correlation.map(|correlation| BridgeCorrelation {
        request_id: Cow::Owned(correlation.request_id),
        reply_to: correlation.reply_to,
    })
}

fn owned_bridge_route_from_address(
    address: &Address,
    transport: Option<&TransportMeta>,
) -> Option<BridgeRoute<'static>> {
    match address {
        Address::Channel(topic) => Some(BridgeRoute::Messaging {
            topic: Cow::Owned(topic.clone()),
        }),
        Address::Resource(resource) => Some(BridgeRoute::RequestResponse {
            resource: Cow::Owned(resource.clone()),
            path: match transport {
                Some(TransportMeta::Http(meta)) => meta.path.clone().map(Cow::Owned),
                _ => None,
            },
        }),
    }
}

fn owned_bridge_transport_parts(
    transport: Option<TransportMeta>,
    fallback_route: Option<BridgeRoute<'static>>,
    address: Option<&Address>,
) -> (
    Option<BridgeTransportMeta<'static>>,
    Option<BridgeHeaders<'static>>,
    Option<BridgeRoute<'static>>,
) {
    match transport {
        Some(TransportMeta::Http(meta)) => {
            let content_type = header_value(&meta.headers, "content-type")
                .map(|value| Cow::Owned(value.to_string()));
            let route = address.and_then(|address| {
                owned_bridge_route_from_address(address, Some(&TransportMeta::Http(meta.clone())))
            });
            (
                Some(BridgeTransportMeta::Http(HttpBridgeMeta {
                    method: meta.method.map(Cow::Owned),
                    path: meta.path.map(Cow::Owned),
                    status_code: meta.status_code,
                    content_type,
                })),
                (!meta.headers.is_empty()).then(|| BridgeHeaders::http(meta.headers)),
                route.or(fallback_route),
            )
        }
        Some(TransportMeta::Mqtt(meta)) => {
            let topic = meta.topic.clone();
            (
                Some(BridgeTransportMeta::Mqtt(MqttBridgeMeta {
                    qos: Some(meta.qos),
                    retain: meta.retain,
                    duplicate: meta.duplicate,
                    packet_id: meta.packet_id,
                    content_type: meta.content_type.map(Cow::Owned),
                    payload_format: meta.payload_format.map(Cow::Owned),
                    message_expiry_interval_secs: meta.message_expiry_interval_secs,
                    response_topic: meta.response_topic.map(Cow::Owned),
                    correlation_data: meta.correlation_data.map(Cow::Owned),
                    correlation_data_bytes: meta.correlation_data_bytes,
                    topic_alias: meta.topic_alias,
                    subscription_identifiers: meta.subscription_identifiers,
                    reason_codes: meta.reason_codes.into_iter().map(Cow::Owned).collect(),
                    reason_string: meta.reason_string.map(Cow::Owned),
                    durable_name: None,
                    shared_group: None,
                    no_local: false,
                    retain_as_published: false,
                    retain_handling: None,
                })),
                (!meta.user_properties.is_empty())
                    .then(|| BridgeHeaders::mqtt(meta.user_properties)),
                address
                    .and_then(|address| owned_bridge_route_from_address(address, None))
                    .or_else(|| {
                        Some(BridgeRoute::Messaging {
                            topic: Cow::Owned(topic),
                        })
                    })
                    .or(fallback_route),
            )
        }
        None => (None, None, fallback_route),
    }
}

fn qos_from_delivery(delivery: DeliveryGuarantee) -> u8 {
    match delivery {
        DeliveryGuarantee::BestEffort => 0,
        DeliveryGuarantee::AtLeastOnce => 1,
        DeliveryGuarantee::ExactlyOnce => 2,
    }
}

fn mqtt_payload_format_name(value: &MqttPayloadFormat) -> Cow<'static, str> {
    match value {
        MqttPayloadFormat::Bytes => Cow::Borrowed("bytes"),
        MqttPayloadFormat::Utf8 => Cow::Borrowed("utf8"),
    }
}

fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers.iter().find_map(|(key, value)| {
        if key.eq_ignore_ascii_case(name) {
            Some(value.as_str())
        } else {
            None
        }
    })
}
