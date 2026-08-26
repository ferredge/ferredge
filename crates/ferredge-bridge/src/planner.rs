use alloc::{
    borrow::Cow,
    string::{String, ToString},
    vec,
};

use ferredge_core::prelude::{
    Address, BrokerMessageProtocolOptions, BrokerSubscriptionProtocolOptions, Command,
    CommandResult, Correlation, DeliveryGuarantee, DeliveryState, EndpointRef, Header, Headers,
    HttpMeta, Intent, MqttPayloadFormat, PayloadValue, RoutedEvent, RoutedResult, TransportMeta,
};

use crate::{
    AddressedAccessMeta, BridgeCapability, BridgeCommand, BridgeEvent, BridgeFault,
    BridgeFaultCategory, BridgeMessage, BridgeMessageMeta, BridgeOp, BridgeOutcome,
    BridgePlannerError, BridgeResult, BridgeRoute, BridgeTransportMeta, HttpBridgeMeta,
    MessagingAction, MessagingCapability, MessagingOp, MqttBridgeMeta, RegisterAccessAction,
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
        Intent::Read { resource, options } => {
            request_response_parts(resource, options, RequestResponseAction::Read, None)
        }
        Intent::Write {
            resource,
            payload,
            options,
        } => request_response_parts(
            resource,
            options,
            RequestResponseAction::Write,
            Some(payload),
        ),
        Intent::Invoke {
            operation,
            args,
            options,
        } => request_response_parts(operation, options, RequestResponseAction::Invoke, args),
        _ => return Err(BridgePlannerError::UnsupportedIntent),
    };

    Ok(BridgeMessage::Command(BridgeCommand {
        id: Cow::Owned(id),
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
        correlation,
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
            let ferredge_core::prelude::BrokerMessageOptions {
                delivery,
                mut headers,
                reply_to,
                correlation_id,
                protocol,
            } = options;
            let mqtt = match protocol {
                Some(BrokerMessageProtocolOptions::Mqtt(mqtt)) => Some(mqtt),
                None => None,
            };
            if let Some(mqtt) = mqtt.as_ref() {
                headers
                    .0
                    .extend(mqtt.user_properties.clone().into_iter().map(Header::from));
            }
            Ok(BridgeMessage::Command(BridgeCommand {
                id: Cow::Owned(id),
                source_device_id,
                target_device_id,
                capability: BridgeCapability::Messaging(MessagingCapability {
                    binary_payloads: true,
                }),
                operation: BridgeOp::Messaging(MessagingOp {
                    action: MessagingAction::Publish,
                }),
                payload: Some(payload),
                route: BridgeRoute::Messaging {
                    topic: Cow::Owned(channel.name),
                },
                transport: Some(BridgeTransportMeta::Mqtt(MqttBridgeMeta {
                    qos: delivery.map(qos_from_delivery),
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
                        .or(reply_to)
                        .map(Cow::Owned),
                    correlation_data: correlation_id.map(Cow::Owned),
                    correlation_data_bytes: mqtt
                        .as_ref()
                        .and_then(|mqtt| mqtt.correlation_data.clone()),
                    topic_alias: mqtt.as_ref().and_then(|mqtt| mqtt.topic_alias),
                    subscription_identifiers: vec::Vec::new(),
                    reason_codes: vec::Vec::new(),
                    reason_string: None,
                    durable_name: None,
                    shared_group: None,
                    no_local: false,
                    retain_as_published: false,
                    retain_handling: None,
                })),
                headers: (!headers.is_empty()).then_some(headers),
                correlation,
            }))
        }
        Intent::Subscribe { channel, options } => {
            let ferredge_core::prelude::BrokerSubscriptionOptions {
                delivery,
                durable_name,
                shared_group,
                protocol,
            } = options;
            let mqtt = match protocol {
                Some(BrokerSubscriptionProtocolOptions::Mqtt(mqtt)) => Some(mqtt),
                None => None,
            };
            Ok(BridgeMessage::Command(BridgeCommand {
                id: Cow::Owned(id),
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
                    qos: delivery.map(qos_from_delivery),
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
                    reason_codes: vec::Vec::new(),
                    reason_string: None,
                    durable_name: durable_name.map(Cow::Owned),
                    shared_group: shared_group.map(Cow::Owned),
                    no_local: mqtt.as_ref().is_some_and(|mqtt| mqtt.no_local),
                    retain_as_published: mqtt.as_ref().is_some_and(|mqtt| mqtt.retain_as_published),
                    retain_handling: mqtt.as_ref().and_then(|mqtt| mqtt.retain_handling),
                })),
                headers: mqtt
                    .as_ref()
                    .filter(|mqtt| !mqtt.user_properties.is_empty())
                    .map(|mqtt| Headers::from(mqtt.user_properties.clone())),
                correlation,
            }))
        }
        Intent::Unsubscribe { channel } => Ok(BridgeMessage::Command(BridgeCommand {
            id: Cow::Owned(id),
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
            correlation,
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
            Cow::Owned(id),
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
            Cow::Owned(id),
            source_device_id,
            target_device_id,
            Cow::Owned(resource),
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
        address: Address::Channel(topic),
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
    let (transport, headers, route) = bridge_transport_parts(transport, None, None);
    let protocol = source.protocol.clone();
    let meta = BridgeMessageMeta {
        source,
        command_id: Cow::Owned(command_id),
        capability: None,
        operation: None,
        route,
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
        address,
        payload,
        correlation,
        transport: event_transport,
    } = event;
    let route = bridge_route_from_address(&address, event_transport.as_ref());
    let (transport, headers, _) = bridge_transport_parts(event_transport, None, Some(&address));

    BridgeMessage::Event(BridgeEvent {
        source,
        capability: BridgeCapability::Messaging(MessagingCapability {
            binary_payloads: true,
        }),
        operation: BridgeOp::Messaging(MessagingOp {
            action: MessagingAction::Publish,
        }),
        payload,
        route: route.unwrap_or_else(|| BridgeRoute::Messaging {
            topic: Cow::Owned(String::new()),
        }),
        transport,
        headers,
        correlation,
    })
}

fn bridge_register_command<'a>(
    id: Cow<'a, str>,
    source_device_id: Option<String>,
    target_device_id: String,
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
        route: BridgeRoute::AddressedAccess {
            resource,
            access: register,
            node_id: Some(u32::from(unit_id)),
        },
        transport: None,
        headers: None,
        correlation,
    })
}

fn request_response_parts(
    resource: String,
    options: ferredge_core::prelude::RequestOptions,
    action: RequestResponseAction,
    payload: Option<PayloadValue<'static>>,
) -> (
    BridgeRoute<'static>,
    RequestResponseAction,
    Option<PayloadValue<'static>>,
    HttpBridgeMeta<'static>,
    Option<Headers<'static>>,
) {
    let ferredge_core::prelude::RequestOptions {
        headers,
        content_type,
        method,
        path,
    } = options;
    let headers = (!headers.is_empty()).then_some(headers);
    (
        BridgeRoute::RequestResponse {
            resource: Cow::Owned(resource),
            path: path.clone().map(Cow::Owned),
        },
        action,
        payload,
        HttpBridgeMeta {
            method: method.map(Cow::Owned),
            path: path.map(Cow::Owned),
            status_code: None,
            content_type: content_type.map(Cow::Owned),
        },
        headers,
    )
}

fn bridge_route_from_address<'a>(
    address: &Address<'a>,
    transport: Option<&TransportMeta<'a>>,
) -> Option<BridgeRoute<'a>> {
    match address {
        Address::Channel(topic) => Some(BridgeRoute::Messaging {
            topic: topic.clone(),
        }),
        Address::Resource(resource) => Some(BridgeRoute::RequestResponse {
            resource: resource.clone(),
            path: match transport {
                Some(TransportMeta::Http(meta)) => meta.path.clone(),
                _ => None,
            },
        }),
    }
}

fn bridge_transport_parts<'a>(
    transport: Option<TransportMeta<'a>>,
    fallback_route: Option<BridgeRoute<'a>>,
    address: Option<&Address<'a>>,
) -> (
    Option<BridgeTransportMeta<'a>>,
    Option<Headers<'a>>,
    Option<BridgeRoute<'a>>,
) {
    match transport {
        Some(TransportMeta::Http(meta)) => {
            let content_type = meta
                .headers
                .iter()
                .find(|header| header.key.eq_ignore_ascii_case("content-type"))
                .map(|header| header.value.clone());
            let route = address.and_then(|address| {
                bridge_route_from_address(address, Some(&TransportMeta::Http(meta.clone())))
            });
            let HttpMeta {
                method,
                path,
                status_code,
                headers,
            } = meta;
            let headers = (!headers.is_empty()).then_some(headers);
            (
                Some(BridgeTransportMeta::Http(HttpBridgeMeta {
                    method,
                    path,
                    status_code,
                    content_type,
                })),
                headers,
                route.or(fallback_route),
            )
        }
        Some(TransportMeta::Mqtt(meta)) => {
            let topic = meta.topic.clone();
            let headers = if meta.user_properties.is_empty() {
                None
            } else {
                Some(Headers::from(meta.user_properties.to_vec()))
            };
            (
                Some(BridgeTransportMeta::Mqtt(MqttBridgeMeta {
                    qos: Some(meta.qos),
                    retain: meta.retain,
                    duplicate: meta.duplicate,
                    packet_id: meta.packet_id,
                    content_type: meta.content_type,
                    payload_format: meta.payload_format,
                    message_expiry_interval_secs: meta.message_expiry_interval_secs,
                    response_topic: meta.response_topic,
                    correlation_data: meta.correlation_data,
                    correlation_data_bytes: meta
                        .correlation_data_bytes
                        .map(|value| value.into_owned()),
                    topic_alias: meta.topic_alias,
                    subscription_identifiers: meta.subscription_identifiers.to_vec(),
                    reason_codes: meta.reason_codes.to_vec(),
                    reason_string: meta.reason_string,
                    durable_name: None,
                    shared_group: None,
                    no_local: false,
                    retain_as_published: false,
                    retain_handling: None,
                })),
                headers,
                address
                    .and_then(|address| bridge_route_from_address(address, None))
                    .or_else(|| Some(BridgeRoute::Messaging { topic }))
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
