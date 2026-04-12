#[cfg(feature = "std")]
use std::{
    collections::HashMap,
    string::String,
    time::Duration,
    vec::Vec,
};

use ferredge_core::prelude::*;
#[cfg(feature = "std")]
use runtime_stack::{StackRuntime, StackSocket};
use mqtt_protocol_core::mqtt;
use mqtt_protocol_core::mqtt::packet::GenericPacketTrait;

use crate::types::{MqttPacketRequest, MqttWirePacket};
#[cfg(feature = "tokio-runtime")]
use crate::runtime_stack;
#[cfg(feature = "async-std-runtime")]
use crate::runtime_stack;

#[cfg(feature = "std")]
pub(crate) struct MqttClientSession {
    pub(crate) stream: StackSocket,
    pub(crate) connection: mqtt::Connection<mqtt::role::Client>,
    pub(crate) pending_command_ids: HashMap<u16, String>,
    pub(crate) pending_reply_routes: HashMap<String, PendingReplyRoute>,
}

#[cfg(feature = "std")]
#[derive(Debug, Clone)]
pub(crate) struct PendingReplyRoute {
    pub(crate) command_id: String,
    pub(crate) reply_to: Option<Address>,
}

#[cfg(feature = "std")]
pub(crate) fn normalize_broker_addr(broker: &str) -> String {
    let without_scheme = broker
        .strip_prefix("mqtt://")
        .or_else(|| broker.strip_prefix("tcp://"))
        .unwrap_or(broker);
    if without_scheme.contains(':') {
        without_scheme.to_string()
    } else {
        format!("{without_scheme}:1883")
    }
}

#[cfg(feature = "std")]
pub(crate) fn mqtt_version_from_core(version: MqttProtocolVersion) -> mqtt::Version {
    match version {
        MqttProtocolVersion::V3_1_1 => mqtt::Version::V3_1_1,
        MqttProtocolVersion::V5_0 => mqtt::Version::V5_0,
    }
}

#[cfg(feature = "std")]
pub(crate) fn build_connect_packet(
    config: &MqttEndpointConfig,
) -> Result<mqtt::packet::Packet, String> {
    match config.preferred_protocol_version() {
        MqttProtocolVersion::V5_0 => {
            let mut builder = mqtt::packet::v5_0::Connect::builder()
                .client_id(config.client_id.as_str())
                .map_err(|e| e.to_string())?
                .clean_start(config.clean_start)
                .keep_alive(config.keepalive_secs.unwrap_or(0));
            if let Some(auth) = &config.auth {
                if let Some(username) = &auth.username {
                    builder = builder
                        .user_name(username.as_str())
                        .map_err(|e| e.to_string())?;
                }
                if let Some(password) = &auth.password {
                    builder = builder
                        .password(password.as_bytes().to_vec())
                        .map_err(|e| e.to_string())?;
                }
            }
            builder.build().map(Into::into).map_err(|e| e.to_string())
        }
        MqttProtocolVersion::V3_1_1 => {
            let mut builder = mqtt::packet::v3_1_1::Connect::builder()
                .client_id(config.client_id.as_str())
                .map_err(|e| e.to_string())?
                .clean_start(config.clean_start)
                .keep_alive(config.keepalive_secs.unwrap_or(0));
            if let Some(auth) = &config.auth {
                if let Some(username) = &auth.username {
                    builder = builder
                        .user_name(username.as_str())
                        .map_err(|e| e.to_string())?;
                }
                if let Some(password) = &auth.password {
                    builder = builder
                        .password(password.as_bytes().to_vec())
                        .map_err(|e| e.to_string())?;
                }
            }
            builder.build().map(Into::into).map_err(|e| e.to_string())
        }
    }
}

#[cfg(feature = "std")]
pub(crate) fn packet_request_into_packet(request: MqttPacketRequest) -> mqtt::packet::Packet {
    match request.packet {
        MqttWirePacket::V5Publish(packet) => packet.into(),
        MqttWirePacket::V3Publish(packet) => packet.into(),
        MqttWirePacket::V5Subscribe(packet) => packet.into(),
        MqttWirePacket::V3Subscribe(packet) => packet.into(),
        MqttWirePacket::V5Unsubscribe(packet) => packet.into(),
        MqttWirePacket::V3Unsubscribe(packet) => packet.into(),
    }
}

#[cfg(feature = "std")]
pub(crate) fn packet_request_packet_id(packet: &MqttWirePacket) -> Option<u16> {
    match packet {
        MqttWirePacket::V5Publish(packet) => packet.packet_id(),
        MqttWirePacket::V3Publish(packet) => packet.packet_id(),
        MqttWirePacket::V5Subscribe(packet) => Some(packet.packet_id()),
        MqttWirePacket::V3Subscribe(packet) => Some(packet.packet_id()),
        MqttWirePacket::V5Unsubscribe(packet) => Some(packet.packet_id()),
        MqttWirePacket::V3Unsubscribe(packet) => Some(packet.packet_id()),
    }
}

#[cfg(feature = "std")]
pub(crate) fn send_packet_request(
    runtime: &StackRuntime,
    session: &mut MqttClientSession,
    device_id: &str,
    request: MqttPacketRequest,
) -> Result<(), String> {
    let request = assign_runtime_packet_id(session, request)?;
    if let Some((correlation_key, route)) = pending_reply_route_from_packet(&request) {
        session.pending_reply_routes.insert(correlation_key, route);
    }
    if let Some(packet_id) = packet_request_packet_id(&request.packet) {
        session
            .pending_command_ids
            .insert(packet_id, request.command_id.clone());
    }
    let events = session
        .connection
        .checked_send(packet_request_into_packet(request));
    let _ = handle_connection_events(runtime, session, device_id, events)?;
    Ok(())
}

#[cfg(feature = "std")]
fn assign_runtime_packet_id(
    session: &mut MqttClientSession,
    request: MqttPacketRequest,
) -> Result<MqttPacketRequest, String> {
    let maybe_packet_id = match &request.packet {
        MqttWirePacket::V5Publish(packet) if packet.qos() != mqtt::packet::Qos::AtMostOnce => {
            if packet.packet_id().is_some() {
                None
            } else {
                Some(
                    session
                        .connection
                        .acquire_packet_id()
                        .map_err(|e| e.to_string())?,
                )
            }
        }
        MqttWirePacket::V3Publish(packet) if packet.qos() != mqtt::packet::Qos::AtMostOnce => {
            if packet.packet_id().is_some() {
                None
            } else {
                Some(
                    session
                        .connection
                        .acquire_packet_id()
                        .map_err(|e| e.to_string())?,
                )
            }
        }
        MqttWirePacket::V5Subscribe(_)
        | MqttWirePacket::V3Subscribe(_)
        | MqttWirePacket::V5Unsubscribe(_)
        | MqttWirePacket::V3Unsubscribe(_) => Some(
            session
                .connection
                .acquire_packet_id()
                .map_err(|e| e.to_string())?,
        ),
        _ => None,
    };

    if let Some(packet_id) = maybe_packet_id {
        rebuild_with_packet_id(request, packet_id)
    } else {
        Ok(request)
    }
}

#[cfg(feature = "std")]
fn rebuild_with_packet_id(request: MqttPacketRequest, packet_id: u16) -> Result<MqttPacketRequest, String> {
    let command_id = request.command_id;
    let packet = match request.packet {
        MqttWirePacket::V5Publish(packet) => {
            let rebuilt = mqtt::packet::v5_0::Publish::builder()
                .topic_name(packet.topic_name())
                .map_err(|e| e.to_string())?
                .qos(packet.qos())
                .retain(packet.retain())
                .dup(packet.dup())
                .packet_id(packet_id)
                .payload(packet.payload().as_slice().to_vec())
                .props(packet.props().clone())
                .build()
                .map_err(|e| e.to_string())?;
            MqttWirePacket::V5Publish(rebuilt)
        }
        MqttWirePacket::V3Publish(packet) => {
            let rebuilt = mqtt::packet::v3_1_1::Publish::builder()
                .topic_name(packet.topic_name())
                .map_err(|e| e.to_string())?
                .qos(packet.qos())
                .retain(packet.retain())
                .dup(packet.dup())
                .packet_id(packet_id)
                .payload(packet.payload().as_slice().to_vec())
                .build()
                .map_err(|e| e.to_string())?;
            MqttWirePacket::V3Publish(rebuilt)
        }
        MqttWirePacket::V5Subscribe(packet) => {
            let rebuilt = mqtt::packet::v5_0::Subscribe::builder()
                .packet_id(packet_id)
                .entries(packet.entries().to_vec())
                .props(packet.props().clone())
                .build()
                .map_err(|e| e.to_string())?;
            MqttWirePacket::V5Subscribe(rebuilt)
        }
        MqttWirePacket::V3Subscribe(packet) => {
            let rebuilt = mqtt::packet::v3_1_1::Subscribe::builder()
                .packet_id(packet_id)
                .entries(packet.entries().to_vec())
                .build()
                .map_err(|e| e.to_string())?;
            MqttWirePacket::V3Subscribe(rebuilt)
        }
        MqttWirePacket::V5Unsubscribe(packet) => {
            let entries: Vec<String> = packet.entries().iter().map(|entry| entry.as_str().to_string()).collect();
            let entry_refs: Vec<&str> = entries.iter().map(String::as_str).collect();
            let rebuilt = mqtt::packet::v5_0::Unsubscribe::builder()
                .packet_id(packet_id)
                .entries(entry_refs)
                .map_err(|e| e.to_string())?
                .props(packet.props().clone())
                .build()
                .map_err(|e| e.to_string())?;
            MqttWirePacket::V5Unsubscribe(rebuilt)
        }
        MqttWirePacket::V3Unsubscribe(packet) => {
            let entries: Vec<String> = packet.entries().iter().map(|entry| entry.as_str().to_string()).collect();
            let entry_refs: Vec<&str> = entries.iter().map(String::as_str).collect();
            let rebuilt = mqtt::packet::v3_1_1::Unsubscribe::builder()
                .packet_id(packet_id)
                .entries(entry_refs)
                .map_err(|e| e.to_string())?
                .build()
                .map_err(|e| e.to_string())?;
            MqttWirePacket::V3Unsubscribe(rebuilt)
        }
    };

    Ok(MqttPacketRequest { command_id, packet })
}

#[cfg(feature = "std")]
pub(crate) fn read_from_session(
    runtime: &StackRuntime,
    session: &mut MqttClientSession,
    device_id: &str,
    timeout: Option<Duration>,
) -> Result<Vec<RoutedMessage>, String> {
    runtime.block_on(read_from_session_async(session, device_id, timeout))
}

#[cfg(feature = "std")]
pub(crate) async fn read_from_session_async(
    session: &mut MqttClientSession,
    device_id: &str,
    timeout: Option<Duration>,
) -> Result<Vec<RoutedMessage>, String> {
    session
        .stream
        .set_read_timeout(timeout)
        .map_err(|e| format!("failed to set MQTT read timeout: {e:?}"))?;

    let mut buffer = [0u8; 4096];
    match session.stream.read(&mut buffer).await {
        Ok(0) => Ok(Vec::new()),
        Ok(n) => {
            let mut cursor = mqtt::common::Cursor::new(&buffer[..n]);
            let events = session.connection.recv(&mut cursor);
            handle_connection_events_async(session, device_id, events).await
        }
        Err(NetError::TimedOut) => Ok(Vec::new()),
        Err(error) => Err(format!("failed reading MQTT stream: {error:?}")),
    }
}

#[cfg(feature = "std")]
pub(crate) fn handle_connection_events(
    runtime: &StackRuntime,
    session: &mut MqttClientSession,
    device_id: &str,
    events: Vec<mqtt::connection::Event>,
) -> Result<Vec<RoutedMessage>, String> {
    runtime.block_on(handle_connection_events_async(session, device_id, events))
}

#[cfg(feature = "std")]
pub(crate) async fn handle_connection_events_async(
    session: &mut MqttClientSession,
    device_id: &str,
    events: Vec<mqtt::connection::Event>,
) -> Result<Vec<RoutedMessage>, String> {
    let mut routed = Vec::new();

    for event in events {
        match event {
            mqtt::connection::Event::RequestSendPacket { packet, .. } => {
                let bytes = packet.to_continuous_buffer();
                write_all_socket_async(&mut session.stream, &bytes)
                    .await
                    .map_err(|e| format!("failed to write MQTT packet: {e:?}"))?;
                session
                    .stream
                    .flush()
                    .await
                    .map_err(|e| format!("failed to flush MQTT packet: {e:?}"))?;
            }
            mqtt::connection::Event::NotifyPacketReceived(packet) => {
                if let Some(message) = routed_message_from_packet(
                    &mut session.pending_command_ids,
                    &mut session.pending_reply_routes,
                    device_id,
                    packet,
                ) {
                    routed.push(message);
                }
            }
            mqtt::connection::Event::NotifyError(error) => {
                return Err(format!("mqtt protocol error: {error:?}"));
            }
            mqtt::connection::Event::RequestClose => {
                return Err("mqtt connection requested close".to_string());
            }
            mqtt::connection::Event::NotifyPacketIdReleased(_)
            | mqtt::connection::Event::RequestTimerReset { .. }
            | mqtt::connection::Event::RequestTimerCancel(_) => {}
        }
    }

    Ok(routed)
}

#[cfg(feature = "std")]
async fn write_all_socket_async(
    socket: &mut StackSocket,
    mut buf: &[u8],
) -> Result<(), NetError> {
    while !buf.is_empty() {
        let written = socket.write(buf).await?;
        if written == 0 {
            return Err(NetError::Closed);
        }
        buf = &buf[written..];
    }
    Ok(())
}


#[cfg(feature = "std")]
pub(crate) fn routed_message_from_packet(
    pending_command_ids: &mut HashMap<u16, String>,
    pending_reply_routes: &mut HashMap<String, PendingReplyRoute>,
    device_id: &str,
    packet: mqtt::packet::Packet,
) -> Option<RoutedMessage> {
    match packet {
        mqtt::packet::Packet::V5_0Publish(packet) => {
            routed_reply_or_event_from_v5_publish(pending_reply_routes, device_id, &packet)
        }
        mqtt::packet::Packet::V3_1_1Publish(packet) => Some(RoutedMessage::Event(
            routed_event_from_v3_publish(device_id, &packet),
        )),
        mqtt::packet::Packet::V5_0Puback(packet) => {
            Some(RoutedMessage::Result(routed_result_from_packet_id(
                pending_command_ids,
                device_id,
                packet.packet_id(),
                DeliveryState::Completed,
                true,
            )))
        }
        mqtt::packet::Packet::V3_1_1Puback(packet) => {
            Some(RoutedMessage::Result(routed_result_from_packet_id(
                pending_command_ids,
                device_id,
                packet.packet_id(),
                DeliveryState::Completed,
                true,
            )))
        }
        mqtt::packet::Packet::V5_0Pubrec(packet) => {
            Some(RoutedMessage::Result(routed_result_from_packet_id(
                pending_command_ids,
                device_id,
                packet.packet_id(),
                DeliveryState::Dispatched,
                false,
            )))
        }
        mqtt::packet::Packet::V3_1_1Pubrec(packet) => {
            Some(RoutedMessage::Result(routed_result_from_packet_id(
                pending_command_ids,
                device_id,
                packet.packet_id(),
                DeliveryState::Dispatched,
                false,
            )))
        }
        mqtt::packet::Packet::V5_0Pubcomp(packet) => {
            Some(RoutedMessage::Result(routed_result_from_packet_id(
                pending_command_ids,
                device_id,
                packet.packet_id(),
                DeliveryState::Completed,
                true,
            )))
        }
        mqtt::packet::Packet::V3_1_1Pubcomp(packet) => {
            Some(RoutedMessage::Result(routed_result_from_packet_id(
                pending_command_ids,
                device_id,
                packet.packet_id(),
                DeliveryState::Completed,
                true,
            )))
        }
        mqtt::packet::Packet::V5_0Suback(packet) => {
            Some(RoutedMessage::Result(routed_result_from_packet_id(
                pending_command_ids,
                device_id,
                packet.packet_id(),
                DeliveryState::Completed,
                true,
            )))
        }
        mqtt::packet::Packet::V3_1_1Suback(packet) => {
            Some(RoutedMessage::Result(routed_result_from_packet_id(
                pending_command_ids,
                device_id,
                packet.packet_id(),
                DeliveryState::Completed,
                true,
            )))
        }
        mqtt::packet::Packet::V5_0Unsuback(packet) => {
            Some(RoutedMessage::Result(routed_result_from_packet_id(
                pending_command_ids,
                device_id,
                packet.packet_id(),
                DeliveryState::Completed,
                true,
            )))
        }
        mqtt::packet::Packet::V3_1_1Unsuback(packet) => {
            Some(RoutedMessage::Result(routed_result_from_packet_id(
                pending_command_ids,
                device_id,
                packet.packet_id(),
                DeliveryState::Completed,
                true,
            )))
        }
        _ => None,
    }
}

#[cfg(feature = "std")]
fn pending_reply_route_from_packet(
    request: &MqttPacketRequest,
) -> Option<(String, PendingReplyRoute)> {
    match &request.packet {
        MqttWirePacket::V5Publish(packet) => {
            let mut correlation_key = None;
            let mut reply_to = None;
            for prop in packet.props() {
                match prop {
                    mqtt::packet::Property::CorrelationData(prop) => {
                        correlation_key =
                            Some(String::from_utf8_lossy(prop.val()).into_owned());
                    }
                    mqtt::packet::Property::ResponseTopic(prop) => {
                        reply_to = Some(Address::Channel(prop.val().to_string()));
                    }
                    _ => {}
                }
            }

            correlation_key.map(|correlation_key| {
                (
                    correlation_key,
                    PendingReplyRoute {
                        command_id: request.command_id.clone(),
                        reply_to,
                    },
                )
            })
        }
        _ => None,
    }
}

#[cfg(feature = "std")]
pub(crate) fn mqtt_source(device_id: &str) -> EndpointRef {
    EndpointRef {
        device_id: device_id.to_string(),
        protocol: DeviceProtocol::MQTT,
    }
}

#[cfg(feature = "std")]
pub(crate) fn routed_event_from_v5_publish(
    device_id: &str,
    packet: &mqtt::packet::v5_0::Publish,
) -> RoutedEvent {
    let mut correlation_id = None;
    let mut reply_to = None;
    let mut content_type = None;
    let mut subscription_identifiers = Vec::new();
    let mut user_properties = Vec::new();
    for prop in packet.props() {
        match prop {
            mqtt::packet::Property::CorrelationData(prop) => {
                correlation_id = Some(String::from_utf8_lossy(prop.val()).into_owned());
            }
            mqtt::packet::Property::ResponseTopic(prop) => {
                reply_to = Some(Address::Channel(prop.val().to_string()));
            }
            mqtt::packet::Property::ContentType(prop) => {
                content_type = Some(prop.val().to_string());
            }
            mqtt::packet::Property::SubscriptionIdentifier(prop) => {
                subscription_identifiers.push(prop.val());
            }
            mqtt::packet::Property::UserProperty(prop) => {
                user_properties.push((prop.key().to_string(), prop.val().to_string()));
            }
            _ => {}
        }
    }
    let response_topic = reply_to.as_ref().and_then(|address| match address {
        Address::Channel(channel) => Some(channel.clone()),
        Address::Resource(_) => None,
    });
    RoutedEvent {
        source: mqtt_source(device_id),
        address: Address::Channel(packet.topic_name().to_string()),
        payload: packet.payload().as_slice().to_vec(),
        correlation: if correlation_id.is_some() || reply_to.is_some() {
            Some(Correlation {
                request_id: correlation_id.clone().unwrap_or_default(),
                reply_to,
            })
        } else {
            None
        },
        transport: Some(TransportMeta::Mqtt(MqttMeta {
            topic: packet.topic_name().to_string(),
            qos: packet.qos() as u8,
            retain: packet.retain(),
            duplicate: packet.dup(),
            packet_id: packet.packet_id(),
            content_type,
            response_topic,
            correlation_data: correlation_id.clone(),
            subscription_identifiers,
            user_properties,
        })),
    }
}

#[cfg(feature = "std")]
fn routed_reply_or_event_from_v5_publish(
    pending_reply_routes: &mut HashMap<String, PendingReplyRoute>,
    device_id: &str,
    packet: &mqtt::packet::v5_0::Publish,
) -> Option<RoutedMessage> {
    let mut correlation_key = None;
    for prop in packet.props() {
        if let mqtt::packet::Property::CorrelationData(prop) = prop {
            correlation_key = Some(String::from_utf8_lossy(prop.val()).into_owned());
            break;
        }
    }

    if let Some(correlation_key) = correlation_key
        && let Some(route) = pending_reply_routes.remove(&correlation_key)
    {
        return Some(RoutedMessage::Result(RoutedResult {
            source: mqtt_source(device_id),
            result: CommandResult {
                command_id: route.command_id.clone(),
                device_id: device_id.to_string(),
                state: DeliveryState::Completed,
                payload: Some(packet.payload().as_slice().to_vec()),
                error: None,
                correlation: Some(Correlation {
                    request_id: route.command_id,
                    reply_to: route.reply_to,
                }),
            },
            transport: Some(TransportMeta::Mqtt(MqttMeta {
                topic: packet.topic_name().to_string(),
                qos: packet.qos() as u8,
                retain: packet.retain(),
                duplicate: packet.dup(),
                packet_id: packet.packet_id(),
                content_type: packet.props().iter().find_map(|prop| match prop {
                    mqtt::packet::Property::ContentType(prop) => Some(prop.val().to_string()),
                    _ => None,
                }),
                response_topic: packet.props().iter().find_map(|prop| match prop {
                    mqtt::packet::Property::ResponseTopic(prop) => Some(prop.val().to_string()),
                    _ => None,
                }),
                correlation_data: Some(correlation_key),
                subscription_identifiers: packet
                    .props()
                    .iter()
                    .filter_map(|prop| match prop {
                        mqtt::packet::Property::SubscriptionIdentifier(prop) => Some(prop.val()),
                        _ => None,
                    })
                    .collect(),
                user_properties: packet
                    .props()
                    .iter()
                    .filter_map(|prop| match prop {
                        mqtt::packet::Property::UserProperty(prop) => {
                            Some((prop.key().to_string(), prop.val().to_string()))
                        }
                        _ => None,
                    })
                    .collect(),
            })),
        }));
    }

    Some(RoutedMessage::Event(routed_event_from_v5_publish(
        device_id, packet,
    )))
}

#[cfg(feature = "std")]
pub(crate) fn routed_event_from_v3_publish(
    device_id: &str,
    packet: &mqtt::packet::v3_1_1::Publish,
) -> RoutedEvent {
    RoutedEvent {
        source: mqtt_source(device_id),
        address: Address::Channel(packet.topic_name().to_string()),
        payload: packet.payload().as_slice().to_vec(),
        correlation: None,
        transport: Some(TransportMeta::Mqtt(MqttMeta {
            topic: packet.topic_name().to_string(),
            qos: packet.qos() as u8,
            retain: packet.retain(),
            duplicate: packet.dup(),
            packet_id: packet.packet_id(),
            content_type: None,
            response_topic: None,
            correlation_data: None,
            subscription_identifiers: Vec::new(),
            user_properties: Vec::new(),
        })),
    }
}

#[cfg(feature = "std")]
pub(crate) fn routed_result_from_packet_id(
    pending_command_ids: &mut HashMap<u16, String>,
    device_id: &str,
    packet_id: u16,
    state: DeliveryState,
    remove: bool,
) -> RoutedResult {
    let command_id = if remove {
        pending_command_ids
            .remove(&packet_id)
            .unwrap_or_else(|| packet_id.to_string())
    } else {
        pending_command_ids
            .get(&packet_id)
            .cloned()
            .unwrap_or_else(|| packet_id.to_string())
    };
    RoutedResult {
        source: mqtt_source(device_id),
        result: CommandResult {
            command_id,
            device_id: device_id.to_string(),
            state,
            payload: None,
            error: None,
            correlation: None,
        },
        transport: None,
    }
}
