use std::{collections::HashMap, string::String, time::Duration, vec::Vec};

use ferredge_core::prelude::RuntimeInstant as RuntimeInstantExt;
use ferredge_core::prelude::*;
use mqtt_protocol_core::mqtt;
use mqtt_protocol_core::mqtt::packet::GenericPacketTrait;
use runtime_stack::StackSocket;

use crate::convert::qos_from_delivery;
#[cfg(feature = "tokio-runtime")]
use crate::runtime_stack;
#[cfg(feature = "async-std-runtime")]
use crate::runtime_stack;
use crate::types::{MqttPacketRequest, MqttWirePacket};
use crate::{
    MqttAuthChallenge, MqttAuthFlowReason, MqttAuthProvider, MqttAuthResponse, MqttAuthStage,
};

type RuntimeMutex<T> = <runtime_stack::StackRuntime as AsyncRuntime>::Mutex<T>;
type StackInstant = <runtime_stack::StackRuntime as AsyncRuntime>::Instant;

pub(crate) struct MqttClientSession {
    pub(crate) stream: StackSocket,
    pub(crate) connection: mqtt::Connection<mqtt::role::Client>,
    pub(crate) protocol_version: MqttProtocolVersion,
    pub(crate) keepalive_secs: Option<u16>,
    pub(crate) last_activity: StackInstant,
    pub(crate) awaiting_pingresp: bool,
    pub(crate) pending_command_ids: HashMap<u16, String>,
    pub(crate) pending_reply_routes: HashMap<String, PendingReplyRoute>,
    pub(crate) authentication_method: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct PendingReplyRoute {
    pub(crate) command_id: String,
    pub(crate) reply_to: Option<Address>,
}

pub(crate) fn normalize_broker_addr(broker: &str) -> String {
    let without_scheme = broker
        .strip_prefix("mqtt://")
        .or_else(|| broker.strip_prefix("tcp://"))
        .unwrap_or(broker);
    if without_scheme.starts_with('[') {
        return if without_scheme.contains("]:") {
            without_scheme.to_string()
        } else {
            format!("{without_scheme}:1883")
        };
    }
    if without_scheme.contains(':') {
        without_scheme.to_string()
    } else {
        format!("{without_scheme}:1883")
    }
}

pub(crate) fn mqtt_version_from_core(version: MqttProtocolVersion) -> mqtt::Version {
    match version {
        MqttProtocolVersion::V3_1_1 => mqtt::Version::V3_1_1,
        MqttProtocolVersion::V5_0 => mqtt::Version::V5_0,
    }
}

pub(crate) fn build_connect_packet(
    config: &MqttEndpointConfig,
) -> Result<mqtt::packet::Packet, String> {
    match config.preferred_protocol_version() {
        MqttProtocolVersion::V5_0 => {
            let mut props = mqtt::packet::Properties::new();
            if let Some(session_expiry_secs) = config
                .connect_properties
                .session_expiry_interval_secs
                .or(config.session_expiry_secs)
            {
                props.push(mqtt::packet::Property::SessionExpiryInterval(
                    mqtt::packet::SessionExpiryInterval::new(session_expiry_secs)
                        .map_err(|e| e.to_string())?,
                ));
            }
            if let Some(receive_maximum) = config.connect_properties.receive_maximum {
                props.push(mqtt::packet::Property::ReceiveMaximum(
                    mqtt::packet::ReceiveMaximum::new(receive_maximum)
                        .map_err(|e| e.to_string())?,
                ));
            }
            if let Some(maximum_packet_size) = config.connect_properties.maximum_packet_size {
                props.push(mqtt::packet::Property::MaximumPacketSize(
                    mqtt::packet::MaximumPacketSize::new(maximum_packet_size)
                        .map_err(|e| e.to_string())?,
                ));
            }
            if let Some(topic_alias_maximum) = config.connect_properties.topic_alias_maximum {
                props.push(mqtt::packet::Property::TopicAliasMaximum(
                    mqtt::packet::TopicAliasMaximum::new(topic_alias_maximum)
                        .map_err(|e| e.to_string())?,
                ));
            }
            if let Some(request_problem_information) =
                config.connect_properties.request_problem_information
            {
                props.push(mqtt::packet::Property::RequestProblemInformation(
                    mqtt::packet::RequestProblemInformation::new(request_problem_information as u8)
                        .map_err(|e| e.to_string())?,
                ));
            }
            if let Some(request_response_information) =
                config.connect_properties.request_response_information
            {
                props.push(mqtt::packet::Property::RequestResponseInformation(
                    mqtt::packet::RequestResponseInformation::new(
                        request_response_information as u8,
                    )
                    .map_err(|e| e.to_string())?,
                ));
            }
            if let Some(authentication_method) = &config.connect_properties.authentication_method {
                props.push(mqtt::packet::Property::AuthenticationMethod(
                    mqtt::packet::AuthenticationMethod::new(authentication_method.as_str())
                        .map_err(|e| e.to_string())?,
                ));
            }
            if let Some(authentication_data) = &config.connect_properties.authentication_data {
                props.push(mqtt::packet::Property::AuthenticationData(
                    mqtt::packet::AuthenticationData::new(authentication_data.clone())
                        .map_err(|e| e.to_string())?,
                ));
            }
            for (key, value) in &config.connect_properties.user_properties {
                props.push(mqtt::packet::Property::UserProperty(
                    mqtt::packet::UserProperty::new(key.as_str(), value.as_str())
                        .map_err(|e| e.to_string())?,
                ));
            }
            let mut builder = mqtt::packet::v5_0::Connect::builder()
                .client_id(config.client_id.as_str())
                .map_err(|e| e.to_string())?
                .clean_start(config.clean_start)
                .keep_alive(config.keepalive_secs.unwrap_or(0))
                .props(props);
            if let Some(will) = &config.will {
                builder = builder
                    .will_message(
                        will.topic.as_str(),
                        will.payload.clone(),
                        qos_from_delivery(will.delivery),
                        will.retain,
                    )
                    .map_err(|e| e.to_string())?;
                let will_props = build_v5_will_props(will)?;
                builder = builder.will_props(will_props);
            }
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

fn publish_requires_packet_id(packet: &MqttWirePacket) -> bool {
    match packet {
        MqttWirePacket::V5Publish(packet) => packet.qos() != mqtt::packet::Qos::AtMostOnce,
        MqttWirePacket::V3Publish(packet) => packet.qos() != mqtt::packet::Qos::AtMostOnce,
        _ => false,
    }
}

pub(crate) async fn send_packet_request_async(
    runtime: &runtime_stack::StackRuntime,
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
    let _ = handle_connection_events_async(runtime, session, device_id, None, None, events).await?;
    Ok(())
}

fn assign_runtime_packet_id(
    session: &mut MqttClientSession,
    request: MqttPacketRequest,
) -> Result<MqttPacketRequest, String> {
    let maybe_packet_id = match &request.packet {
        packet if publish_requires_packet_id(packet) => Some(
            session
                .connection
                .acquire_packet_id()
                .map_err(|e| e.to_string())?,
        ),
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

fn rebuild_with_packet_id(
    request: MqttPacketRequest,
    packet_id: u16,
) -> Result<MqttPacketRequest, String> {
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
            let entries: Vec<String> = packet
                .entries()
                .iter()
                .map(|entry| entry.as_str().to_string())
                .collect();
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
            let entries: Vec<String> = packet
                .entries()
                .iter()
                .map(|entry| entry.as_str().to_string())
                .collect();
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

pub(crate) async fn read_from_session_async(
    runtime: &runtime_stack::StackRuntime,
    session: &mut MqttClientSession,
    device_id: &str,
    negotiated_connack: Option<&Shared<RuntimeMutex<Option<MqttConnackProperties>>>>,
    auth_handler: Option<&Shared<RuntimeMutex<Option<Shared<dyn MqttAuthProvider>>>>>,
    timeout: Option<Duration>,
) -> Result<Vec<RoutedMessage>, String> {
    session
        .stream
        .set_read_timeout(timeout)
        .map_err(|e| format!("failed to set MQTT read timeout: {e:?}"))?;

    let mut buffer = [0u8; 4096];
    match session.stream.read(&mut buffer).await {
        Ok(0) => Err("mqtt stream closed".to_string()),
        Ok(n) => {
            session.last_activity = runtime.now();
            session.awaiting_pingresp = false;
            let mut cursor = mqtt::common::Cursor::new(&buffer[..n]);
            let events = session.connection.recv(&mut cursor);
            handle_connection_events_async(
                runtime,
                session,
                device_id,
                negotiated_connack,
                auth_handler,
                events,
            )
            .await
        }
        Err(NetError::TimedOut) => {
            send_keepalive_ping_if_due(runtime, session, device_id).await?;
            Ok(Vec::new())
        }
        Err(error) => Err(format!("failed reading MQTT stream: {error:?}")),
    }
}

pub(crate) async fn handle_connection_events_async(
    runtime: &runtime_stack::StackRuntime,
    session: &mut MqttClientSession,
    device_id: &str,
    negotiated_connack: Option<&Shared<RuntimeMutex<Option<MqttConnackProperties>>>>,
    auth_handler: Option<&Shared<RuntimeMutex<Option<Shared<dyn MqttAuthProvider>>>>>,
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
                session.last_activity = runtime.now();
            }
            mqtt::connection::Event::NotifyPacketReceived(packet) => {
                if let Some(negotiated_connack) = negotiated_connack {
                    update_negotiated_connack(negotiated_connack, &packet).await?;
                }
                if let Some(auth_handler) = auth_handler {
                    process_incoming_auth_async(runtime, session, auth_handler, &packet).await?;
                }
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

async fn process_incoming_auth_async(
    runtime: &runtime_stack::StackRuntime,
    session: &mut MqttClientSession,
    auth_handler: &Shared<RuntimeMutex<Option<Shared<dyn MqttAuthProvider>>>>,
    packet: &mqtt::packet::Packet,
) -> Result<(), String> {
    let challenge = match packet {
        mqtt::packet::Packet::V5_0Auth(packet) => Some(auth_challenge_from_auth(session, packet)?),
        _ => None,
    };
    let Some(challenge) = challenge else {
        return Ok(());
    };

    let handler = auth_handler
        .lock()
        .await
        .map_err(|_| "failed to lock MQTT auth handler".to_string())?
        .clone();
    let Some(handler) = handler else {
        return Err(
            "mqtt broker requested enhanced authentication but no auth handler is configured"
                .to_string(),
        );
    };
    let Some(response) = handler.respond(challenge)? else {
        return Err("mqtt auth handler returned no response for broker challenge".to_string());
    };
    let packet = build_auth_packet(session, response)?;
    let events = session.connection.checked_send(packet);
    process_outbound_auth_events_async(runtime, session, events).await?;
    Ok(())
}

async fn process_outbound_auth_events_async(
    runtime: &runtime_stack::StackRuntime,
    session: &mut MqttClientSession,
    events: Vec<mqtt::connection::Event>,
) -> Result<(), String> {
    for event in events {
        match event {
            mqtt::connection::Event::RequestSendPacket { packet, .. } => {
                let bytes = packet.to_continuous_buffer();
                write_all_socket_async(&mut session.stream, &bytes)
                    .await
                    .map_err(|e| format!("failed to write MQTT auth packet: {e:?}"))?;
                session
                    .stream
                    .flush()
                    .await
                    .map_err(|e| format!("failed to flush MQTT auth packet: {e:?}"))?;
                session.last_activity = runtime.now();
            }
            mqtt::connection::Event::NotifyError(error) => {
                return Err(format!(
                    "mqtt protocol error during auth exchange: {error:?}"
                ));
            }
            mqtt::connection::Event::RequestClose => {
                return Err("mqtt auth exchange requested close".to_string());
            }
            mqtt::connection::Event::NotifyPacketReceived(_)
            | mqtt::connection::Event::NotifyPacketIdReleased(_)
            | mqtt::connection::Event::RequestTimerReset { .. }
            | mqtt::connection::Event::RequestTimerCancel(_) => {}
        }
    }
    Ok(())
}

fn auth_challenge_from_auth(
    session: &mut MqttClientSession,
    packet: &mqtt::packet::v5_0::Auth,
) -> Result<MqttAuthChallenge, String> {
    let mut authentication_method = session.authentication_method.clone();
    let mut authentication_data = None;
    let mut reason_string = None;
    let mut user_properties = Vec::new();

    if let Some(props) = packet.props() {
        for prop in props {
            match prop {
                mqtt::packet::Property::AuthenticationMethod(prop) => {
                    authentication_method = Some(prop.val().to_string());
                }
                mqtt::packet::Property::AuthenticationData(prop) => {
                    authentication_data = Some(prop.val().to_vec());
                }
                mqtt::packet::Property::ReasonString(prop) => {
                    reason_string = Some(prop.val().to_string());
                }
                mqtt::packet::Property::UserProperty(prop) => {
                    user_properties.push((prop.key().to_string(), prop.val().to_string()));
                }
                _ => {}
            }
        }
    }
    session.authentication_method = authentication_method.clone();

    Ok(MqttAuthChallenge {
        stage: if packet.reason_code() == Some(mqtt::result_code::AuthReasonCode::ReAuthenticate) {
            MqttAuthStage::Reauthenticate
        } else {
            MqttAuthStage::Connect
        },
        reason: auth_flow_reason_from_v5(
            packet
                .reason_code()
                .unwrap_or(mqtt::result_code::AuthReasonCode::Success),
        ),
        authentication_method,
        authentication_data,
        reason_string,
        user_properties,
    })
}

fn auth_flow_reason_from_v5(code: mqtt::result_code::AuthReasonCode) -> MqttAuthFlowReason {
    match code {
        mqtt::result_code::AuthReasonCode::Success => MqttAuthFlowReason::Success,
        mqtt::result_code::AuthReasonCode::ContinueAuthentication => {
            MqttAuthFlowReason::ContinueAuthentication
        }
        mqtt::result_code::AuthReasonCode::ReAuthenticate => MqttAuthFlowReason::ReAuthenticate,
    }
}

fn build_auth_packet(
    session: &mut MqttClientSession,
    response: MqttAuthResponse,
) -> Result<mqtt::packet::v5_0::Auth, String> {
    let mut props = mqtt::packet::Properties::new();
    let method = response
        .authentication_method
        .clone()
        .or_else(|| session.authentication_method.clone());
    if let Some(authentication_method) = &method {
        props.push(mqtt::packet::Property::AuthenticationMethod(
            mqtt::packet::AuthenticationMethod::new(authentication_method.as_str())
                .map_err(|e| e.to_string())?,
        ));
    }
    if let Some(authentication_data) = response.authentication_data {
        props.push(mqtt::packet::Property::AuthenticationData(
            mqtt::packet::AuthenticationData::new(authentication_data)
                .map_err(|e| e.to_string())?,
        ));
    }
    if let Some(reason_string) = &response.reason_string {
        props.push(mqtt::packet::Property::ReasonString(
            mqtt::packet::ReasonString::new(reason_string.as_str()).map_err(|e| e.to_string())?,
        ));
    }
    for (key, value) in &response.user_properties {
        props.push(mqtt::packet::Property::UserProperty(
            mqtt::packet::UserProperty::new(key.as_str(), value.as_str())
                .map_err(|e| e.to_string())?,
        ));
    }
    session.authentication_method = method;

    mqtt::packet::v5_0::Auth::builder()
        .reason_code(match response.reason {
            MqttAuthFlowReason::Success => mqtt::result_code::AuthReasonCode::Success,
            MqttAuthFlowReason::ContinueAuthentication => {
                mqtt::result_code::AuthReasonCode::ContinueAuthentication
            }
            MqttAuthFlowReason::ReAuthenticate => mqtt::result_code::AuthReasonCode::ReAuthenticate,
        })
        .props(props)
        .build()
        .map_err(|e| e.to_string())
}

pub(crate) async fn disconnect_session(
    runtime: &runtime_stack::StackRuntime,
    session: &mut MqttClientSession,
) -> Result<(), String> {
    let events = match session.protocol_version {
        MqttProtocolVersion::V5_0 => {
            let disconnect = mqtt::packet::v5_0::Disconnect::builder()
                .reason_code(mqtt::result_code::DisconnectReasonCode::NormalDisconnection)
                .props(mqtt::packet::Properties::new())
                .build()
                .map_err(|e| e.to_string())?;
            session.connection.checked_send(disconnect)
        }
        MqttProtocolVersion::V3_1_1 => {
            let disconnect = mqtt::packet::v3_1_1::Disconnect::builder()
                .build()
                .map_err(|e| e.to_string())?;
            session.connection.checked_send(disconnect)
        }
    };
    let _ = handle_connection_events_async(runtime, session, "", None, None, events).await?;
    session
        .stream
        .close()
        .await
        .map_err(|e| format!("failed to close MQTT socket: {e:?}"))
}

async fn send_keepalive_ping_if_due(
    runtime: &runtime_stack::StackRuntime,
    session: &mut MqttClientSession,
    device_id: &str,
) -> Result<(), String> {
    let Some(keepalive_secs) = session.keepalive_secs else {
        return Ok(());
    };
    if keepalive_secs == 0 {
        return Ok(());
    }

    if session.last_activity.elapsed() < Duration::from_secs(keepalive_secs as u64) {
        return Ok(());
    }
    if session.awaiting_pingresp {
        return Err("mqtt keepalive timed out waiting for ping response".to_string());
    }

    let events = match session.protocol_version {
        MqttProtocolVersion::V5_0 => {
            let pingreq = mqtt::packet::v5_0::Pingreq::builder()
                .build()
                .map_err(|e| e.to_string())?;
            session.connection.checked_send(pingreq)
        }
        MqttProtocolVersion::V3_1_1 => {
            let pingreq = mqtt::packet::v3_1_1::Pingreq::builder()
                .build()
                .map_err(|e| e.to_string())?;
            session.connection.checked_send(pingreq)
        }
    };
    session.awaiting_pingresp = true;
    let _ = handle_connection_events_async(runtime, session, device_id, None, None, events).await?;
    Ok(())
}

async fn write_all_socket_async(socket: &mut StackSocket, mut buf: &[u8]) -> Result<(), NetError> {
    while !buf.is_empty() {
        let written = socket.write(buf).await?;
        if written == 0 {
            return Err(NetError::Closed);
        }
        buf = &buf[written..];
    }
    Ok(())
}

async fn update_negotiated_connack(
    negotiated_connack: &Shared<RuntimeMutex<Option<MqttConnackProperties>>>,
    packet: &mqtt::packet::Packet,
) -> Result<(), String> {
    let snapshot = match packet {
        mqtt::packet::Packet::V5_0Connack(packet) => Some(connack_snapshot_from_v5(packet)?),
        mqtt::packet::Packet::V3_1_1Connack(packet) => Some(MqttConnackProperties {
            session_present: packet.session_present(),
            reason_code: Some(packet.return_code().to_string()),
            ..MqttConnackProperties::default()
        }),
        _ => None,
    };

    if let Some(snapshot) = snapshot {
        let mut guard = negotiated_connack
            .lock()
            .await
            .map_err(|_| "failed to lock negotiated MQTT CONNACK state".to_string())?;
        *guard = Some(snapshot);
    }
    Ok(())
}

fn connack_snapshot_from_v5(
    packet: &mqtt::packet::v5_0::Connack,
) -> Result<MqttConnackProperties, String> {
    let mut assigned_client_identifier = None;
    let mut response_information = None;
    let mut server_reference = None;
    let mut receive_maximum = None;
    let mut maximum_packet_size = None;
    let mut topic_alias_maximum = None;
    let mut session_expiry_interval_secs = None;
    let mut maximum_qos = None;
    let mut retain_available = None;
    let mut server_keep_alive = None;
    let mut wildcard_subscription_available = None;
    let mut subscription_identifier_available = None;
    let mut shared_subscription_available = None;
    let mut authentication_method = None;
    let mut authentication_data = None;
    let mut reason_string = None;
    let mut user_properties = Vec::new();

    for prop in packet.props() {
        match prop {
            mqtt::packet::Property::AssignedClientIdentifier(prop) => {
                assigned_client_identifier = Some(prop.val().to_string());
            }
            mqtt::packet::Property::ResponseInformation(prop) => {
                response_information = Some(prop.val().to_string());
            }
            mqtt::packet::Property::ServerReference(prop) => {
                server_reference = Some(prop.val().to_string());
            }
            mqtt::packet::Property::ReceiveMaximum(prop) => {
                receive_maximum = Some(prop.val());
            }
            mqtt::packet::Property::MaximumPacketSize(prop) => {
                maximum_packet_size = Some(prop.val());
            }
            mqtt::packet::Property::TopicAliasMaximum(prop) => {
                topic_alias_maximum = Some(prop.val());
            }
            mqtt::packet::Property::SessionExpiryInterval(prop) => {
                session_expiry_interval_secs = Some(prop.val());
            }
            mqtt::packet::Property::MaximumQos(prop) => {
                maximum_qos = Some(prop.val());
            }
            mqtt::packet::Property::RetainAvailable(prop) => {
                retain_available = Some(prop.val() != 0);
            }
            mqtt::packet::Property::ServerKeepAlive(prop) => {
                server_keep_alive = Some(prop.val());
            }
            mqtt::packet::Property::WildcardSubscriptionAvailable(prop) => {
                wildcard_subscription_available = Some(prop.val() != 0);
            }
            mqtt::packet::Property::SubscriptionIdentifierAvailable(prop) => {
                subscription_identifier_available = Some(prop.val() != 0);
            }
            mqtt::packet::Property::SharedSubscriptionAvailable(prop) => {
                shared_subscription_available = Some(prop.val() != 0);
            }
            mqtt::packet::Property::AuthenticationMethod(prop) => {
                authentication_method = Some(prop.val().to_string());
            }
            mqtt::packet::Property::AuthenticationData(prop) => {
                authentication_data = Some(prop.val().to_vec());
            }
            mqtt::packet::Property::ReasonString(prop) => {
                reason_string = Some(prop.val().to_string());
            }
            mqtt::packet::Property::UserProperty(prop) => {
                user_properties.push((prop.key().to_string(), prop.val().to_string()));
            }
            _ => {}
        }
    }

    Ok(MqttConnackProperties {
        session_present: packet.session_present(),
        reason_code: Some(packet.reason_code().to_string()),
        reason_string,
        assigned_client_identifier,
        response_information,
        server_reference,
        receive_maximum,
        maximum_packet_size,
        topic_alias_maximum,
        session_expiry_interval_secs,
        maximum_qos,
        retain_available,
        server_keep_alive,
        wildcard_subscription_available,
        subscription_identifier_available,
        shared_subscription_available,
        authentication_method,
        authentication_data,
        user_properties,
    })
}

fn build_v5_will_props(will: &MqttWillConfig) -> Result<mqtt::packet::Properties, String> {
    let mut props = mqtt::packet::Properties::new();
    if let Some(delay) = will.delay_interval_secs {
        props.push(mqtt::packet::Property::WillDelayInterval(
            mqtt::packet::WillDelayInterval::new(delay).map_err(|e| e.to_string())?,
        ));
    }
    if let Some(payload_format) = will.payload_format {
        props.push(mqtt::packet::Property::PayloadFormatIndicator(
            mqtt::packet::PayloadFormatIndicator::new(match payload_format {
                MqttPayloadFormat::Bytes => mqtt::packet::PayloadFormat::Binary,
                MqttPayloadFormat::Utf8 => mqtt::packet::PayloadFormat::String,
            })
            .map_err(|e| e.to_string())?,
        ));
    }
    if let Some(expiry) = will.message_expiry_interval_secs {
        props.push(mqtt::packet::Property::MessageExpiryInterval(
            mqtt::packet::MessageExpiryInterval::new(expiry).map_err(|e| e.to_string())?,
        ));
    }
    if let Some(content_type) = &will.content_type {
        props.push(mqtt::packet::Property::ContentType(
            mqtt::packet::ContentType::new(content_type.as_str()).map_err(|e| e.to_string())?,
        ));
    }
    if let Some(response_topic) = &will.response_topic {
        props.push(mqtt::packet::Property::ResponseTopic(
            mqtt::packet::ResponseTopic::new(response_topic.as_str()).map_err(|e| e.to_string())?,
        ));
    }
    if let Some(correlation_data) = &will.correlation_data {
        props.push(mqtt::packet::Property::CorrelationData(
            mqtt::packet::CorrelationData::new(correlation_data.clone())
                .map_err(|e| e.to_string())?,
        ));
    }
    for (key, value) in &will.user_properties {
        props.push(mqtt::packet::Property::UserProperty(
            mqtt::packet::UserProperty::new(key.as_str(), value.as_str())
                .map_err(|e| e.to_string())?,
        ));
    }
    Ok(props)
}

fn reason_string_from_props(props: &mqtt::packet::Properties) -> Option<String> {
    props.iter().find_map(|prop| match prop {
        mqtt::packet::Property::ReasonString(prop) => Some(prop.val().to_string()),
        _ => None,
    })
}

fn reason_string_from_props_opt(props: &Option<mqtt::packet::Properties>) -> Option<String> {
    props.as_ref().and_then(reason_string_from_props)
}

fn mqtt_meta_from_v5_publish(
    packet: &mqtt::packet::v5_0::Publish,
    correlation_override: Option<String>,
) -> MqttMeta {
    let mut content_type = None;
    let mut payload_format = None;
    let mut message_expiry_interval_secs = None;
    let mut response_topic = None;
    let mut correlation_data = correlation_override;
    let mut correlation_data_bytes = None;
    let mut topic_alias = None;
    let mut subscription_identifiers = Vec::new();
    let mut user_properties = Vec::new();

    for prop in packet.props() {
        match prop {
            mqtt::packet::Property::ContentType(prop) => {
                content_type = Some(prop.val().to_string())
            }
            mqtt::packet::Property::PayloadFormatIndicator(prop) => {
                payload_format = Some(prop.val().to_string())
            }
            mqtt::packet::Property::MessageExpiryInterval(prop) => {
                message_expiry_interval_secs = Some(prop.val())
            }
            mqtt::packet::Property::ResponseTopic(prop) => {
                response_topic = Some(prop.val().to_string())
            }
            mqtt::packet::Property::CorrelationData(prop) => {
                if correlation_data.is_none() {
                    correlation_data = Some(String::from_utf8_lossy(prop.val()).into_owned());
                }
                correlation_data_bytes = Some(prop.val().to_vec());
            }
            mqtt::packet::Property::TopicAlias(prop) => topic_alias = Some(prop.val()),
            mqtt::packet::Property::SubscriptionIdentifier(prop) => {
                subscription_identifiers.push(prop.val())
            }
            mqtt::packet::Property::UserProperty(prop) => {
                user_properties.push((prop.key().to_string(), prop.val().to_string()))
            }
            _ => {}
        }
    }

    MqttMeta {
        topic: packet.topic_name().to_string(),
        qos: packet.qos() as u8,
        retain: packet.retain(),
        duplicate: packet.dup(),
        packet_id: packet.packet_id(),
        content_type,
        payload_format,
        message_expiry_interval_secs,
        response_topic,
        correlation_data,
        correlation_data_bytes,
        topic_alias,
        subscription_identifiers,
        user_properties,
        reason_codes: Vec::new(),
        reason_string: None,
    }
}

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
        mqtt::packet::Packet::V5_0Puback(packet) => Some(RoutedMessage::Result(
            routed_result_from_v5_puback(pending_command_ids, device_id, &packet),
        )),
        mqtt::packet::Packet::V3_1_1Puback(packet) => {
            Some(RoutedMessage::Result(routed_result_from_packet_id(
                pending_command_ids,
                device_id,
                packet.packet_id(),
                DeliveryState::Completed,
                true,
            )))
        }
        mqtt::packet::Packet::V5_0Pubrec(packet) => Some(RoutedMessage::Result(
            routed_result_from_v5_pubrec(pending_command_ids, device_id, &packet),
        )),
        mqtt::packet::Packet::V5_0Pubrel(packet) => Some(RoutedMessage::Result(
            routed_result_from_v5_pubrel(pending_command_ids, device_id, &packet),
        )),
        mqtt::packet::Packet::V3_1_1Pubrec(packet) => {
            Some(RoutedMessage::Result(routed_result_from_packet_id(
                pending_command_ids,
                device_id,
                packet.packet_id(),
                DeliveryState::Dispatched,
                false,
            )))
        }
        mqtt::packet::Packet::V3_1_1Pubrel(packet) => {
            Some(RoutedMessage::Result(routed_result_from_packet_id(
                pending_command_ids,
                device_id,
                packet.packet_id(),
                DeliveryState::Dispatched,
                false,
            )))
        }
        mqtt::packet::Packet::V5_0Pubcomp(packet) => Some(RoutedMessage::Result(
            routed_result_from_v5_pubcomp(pending_command_ids, device_id, &packet),
        )),
        mqtt::packet::Packet::V3_1_1Pubcomp(packet) => {
            Some(RoutedMessage::Result(routed_result_from_packet_id(
                pending_command_ids,
                device_id,
                packet.packet_id(),
                DeliveryState::Completed,
                true,
            )))
        }
        mqtt::packet::Packet::V5_0Suback(packet) => Some(RoutedMessage::Result(
            routed_result_from_v5_suback(pending_command_ids, device_id, &packet),
        )),
        mqtt::packet::Packet::V3_1_1Suback(packet) => {
            Some(RoutedMessage::Result(routed_result_from_packet_id(
                pending_command_ids,
                device_id,
                packet.packet_id(),
                DeliveryState::Completed,
                true,
            )))
        }
        mqtt::packet::Packet::V5_0Unsuback(packet) => Some(RoutedMessage::Result(
            routed_result_from_v5_unsuback(pending_command_ids, device_id, &packet),
        )),
        mqtt::packet::Packet::V3_1_1Unsuback(packet) => {
            Some(RoutedMessage::Result(routed_result_from_packet_id(
                pending_command_ids,
                device_id,
                packet.packet_id(),
                DeliveryState::Completed,
                true,
            )))
        }
        mqtt::packet::Packet::V5_0Disconnect(packet) => Some(RoutedMessage::Result(
            routed_result_from_v5_disconnect(device_id, &packet),
        )),
        mqtt::packet::Packet::V5_0Auth(packet) => Some(RoutedMessage::Result(
            routed_result_from_v5_auth(device_id, &packet),
        )),
        _ => None,
    }
}

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
                        correlation_key = Some(String::from_utf8_lossy(prop.val()).into_owned());
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

pub(crate) fn mqtt_source(device_id: &str) -> EndpointRef {
    EndpointRef {
        device_id: device_id.to_string(),
        protocol: DeviceProtocol::MQTT,
    }
}

pub(crate) fn routed_event_from_v5_publish(
    device_id: &str,
    packet: &mqtt::packet::v5_0::Publish,
) -> RoutedEvent {
    let mut correlation_id = None;
    let mut correlation_data_bytes = None;
    let mut reply_to = None;
    let mut content_type = None;
    let mut payload_format = None;
    let mut message_expiry_interval_secs = None;
    let mut topic_alias = None;
    let mut subscription_identifiers = Vec::new();
    let mut user_properties = Vec::new();
    for prop in packet.props() {
        match prop {
            mqtt::packet::Property::CorrelationData(prop) => {
                correlation_data_bytes = Some(prop.val().to_vec());
                correlation_id = Some(String::from_utf8_lossy(prop.val()).into_owned());
            }
            mqtt::packet::Property::ResponseTopic(prop) => {
                reply_to = Some(Address::Channel(prop.val().to_string()));
            }
            mqtt::packet::Property::ContentType(prop) => {
                content_type = Some(prop.val().to_string());
            }
            mqtt::packet::Property::PayloadFormatIndicator(prop) => {
                payload_format = Some(prop.val().to_string());
            }
            mqtt::packet::Property::MessageExpiryInterval(prop) => {
                message_expiry_interval_secs = Some(prop.val());
            }
            mqtt::packet::Property::TopicAlias(prop) => {
                topic_alias = Some(prop.val());
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
            payload_format,
            message_expiry_interval_secs,
            response_topic,
            correlation_data: correlation_id.clone(),
            correlation_data_bytes,
            topic_alias,
            subscription_identifiers,
            user_properties,
            reason_codes: Vec::new(),
            reason_string: None,
        })),
    }
}

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
            transport: Some(TransportMeta::Mqtt(mqtt_meta_from_v5_publish(
                packet,
                Some(correlation_key),
            ))),
        }));
    }

    Some(RoutedMessage::Event(routed_event_from_v5_publish(
        device_id, packet,
    )))
}

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
            payload_format: None,
            message_expiry_interval_secs: None,
            response_topic: None,
            correlation_data: None,
            correlation_data_bytes: None,
            topic_alias: None,
            subscription_identifiers: Vec::new(),
            user_properties: Vec::new(),
            reason_codes: Vec::new(),
            reason_string: None,
        })),
    }
}

fn routed_result_from_v5_puback(
    pending_command_ids: &mut HashMap<u16, String>,
    device_id: &str,
    packet: &mqtt::packet::v5_0::Puback,
) -> RoutedResult {
    let reason_code = packet.reason_code();
    routed_result_from_packet_id_with_transport(
        pending_command_ids,
        device_id,
        packet.packet_id(),
        reason_code.map_or(DeliveryState::Completed, |code| {
            if code.is_success() {
                DeliveryState::Completed
            } else {
                DeliveryState::Rejected
            }
        }),
        reason_code
            .filter(|code| code.is_failure())
            .map(|code| code.to_string()),
        true,
        Some(TransportMeta::Mqtt(mqtt_result_meta(
            Some(packet.packet_id()),
            reason_code
                .into_iter()
                .map(|code| code.to_string())
                .collect(),
            reason_string_from_props_opt(packet.props()),
        ))),
    )
}

fn routed_result_from_v5_pubrec(
    pending_command_ids: &mut HashMap<u16, String>,
    device_id: &str,
    packet: &mqtt::packet::v5_0::Pubrec,
) -> RoutedResult {
    let reason_code = packet.reason_code();
    routed_result_from_packet_id_with_transport(
        pending_command_ids,
        device_id,
        packet.packet_id(),
        reason_code.map_or(DeliveryState::Dispatched, |code| {
            if code.is_success() {
                DeliveryState::Dispatched
            } else {
                DeliveryState::Rejected
            }
        }),
        reason_code
            .filter(|code| code.is_failure())
            .map(|code| code.to_string()),
        false,
        Some(TransportMeta::Mqtt(mqtt_result_meta(
            Some(packet.packet_id()),
            reason_code
                .into_iter()
                .map(|code| code.to_string())
                .collect(),
            reason_string_from_props_opt(packet.props()),
        ))),
    )
}

fn routed_result_from_v5_pubrel(
    pending_command_ids: &mut HashMap<u16, String>,
    device_id: &str,
    packet: &mqtt::packet::v5_0::Pubrel,
) -> RoutedResult {
    let reason_code = packet.reason_code();
    routed_result_from_packet_id_with_transport(
        pending_command_ids,
        device_id,
        packet.packet_id(),
        reason_code.map_or(DeliveryState::Dispatched, |code| {
            if code.is_success() {
                DeliveryState::Dispatched
            } else {
                DeliveryState::Rejected
            }
        }),
        reason_code
            .filter(|code| code.is_failure())
            .map(|code| code.to_string()),
        false,
        Some(TransportMeta::Mqtt(mqtt_result_meta(
            Some(packet.packet_id()),
            reason_code
                .into_iter()
                .map(|code| code.to_string())
                .collect(),
            reason_string_from_props_opt(packet.props()),
        ))),
    )
}

fn routed_result_from_v5_pubcomp(
    pending_command_ids: &mut HashMap<u16, String>,
    device_id: &str,
    packet: &mqtt::packet::v5_0::Pubcomp,
) -> RoutedResult {
    let reason_code = packet.reason_code();
    routed_result_from_packet_id_with_transport(
        pending_command_ids,
        device_id,
        packet.packet_id(),
        reason_code.map_or(DeliveryState::Completed, |code| {
            if code.is_success() {
                DeliveryState::Completed
            } else {
                DeliveryState::Rejected
            }
        }),
        reason_code
            .filter(|code| code.is_failure())
            .map(|code| code.to_string()),
        true,
        Some(TransportMeta::Mqtt(mqtt_result_meta(
            Some(packet.packet_id()),
            reason_code
                .into_iter()
                .map(|code| code.to_string())
                .collect(),
            reason_string_from_props_opt(packet.props()),
        ))),
    )
}

fn routed_result_from_v5_suback(
    pending_command_ids: &mut HashMap<u16, String>,
    device_id: &str,
    packet: &mqtt::packet::v5_0::Suback,
) -> RoutedResult {
    let reason_codes = packet.reason_codes();
    let failed_codes: Vec<String> = reason_codes
        .iter()
        .filter(|code| code.is_failure())
        .map(ToString::to_string)
        .collect();
    routed_result_from_packet_id_with_transport(
        pending_command_ids,
        device_id,
        packet.packet_id(),
        if failed_codes.is_empty() {
            DeliveryState::Completed
        } else {
            DeliveryState::Rejected
        },
        if failed_codes.is_empty() {
            None
        } else {
            Some(failed_codes.join(","))
        },
        true,
        Some(TransportMeta::Mqtt(mqtt_result_meta(
            Some(packet.packet_id()),
            reason_codes
                .into_iter()
                .map(|code| code.to_string())
                .collect(),
            reason_string_from_props(packet.props()),
        ))),
    )
}

fn routed_result_from_v5_unsuback(
    pending_command_ids: &mut HashMap<u16, String>,
    device_id: &str,
    packet: &mqtt::packet::v5_0::Unsuback,
) -> RoutedResult {
    let reason_codes = packet.reason_codes();
    let failed_codes: Vec<String> = reason_codes
        .iter()
        .filter(|code| code.is_failure())
        .map(ToString::to_string)
        .collect();
    routed_result_from_packet_id_with_transport(
        pending_command_ids,
        device_id,
        packet.packet_id(),
        if failed_codes.is_empty() {
            DeliveryState::Completed
        } else {
            DeliveryState::Rejected
        },
        if failed_codes.is_empty() {
            None
        } else {
            Some(failed_codes.join(","))
        },
        true,
        Some(TransportMeta::Mqtt(mqtt_result_meta(
            Some(packet.packet_id()),
            reason_codes
                .into_iter()
                .map(|code| code.to_string())
                .collect(),
            reason_string_from_props(packet.props()),
        ))),
    )
}

fn routed_result_from_v5_disconnect(
    device_id: &str,
    packet: &mqtt::packet::v5_0::Disconnect,
) -> RoutedResult {
    let reason_code = packet.reason_code();
    let reason_code_text = reason_code.map(|code| code.to_string());
    let state = match reason_code {
        None
        | Some(mqtt::result_code::DisconnectReasonCode::NormalDisconnection)
        | Some(mqtt::result_code::DisconnectReasonCode::DisconnectWithWillMessage) => {
            DeliveryState::Completed
        }
        Some(_) => DeliveryState::Rejected,
    };
    RoutedResult {
        source: mqtt_source(device_id),
        result: CommandResult {
            command_id: "__mqtt_disconnect__".to_string(),
            device_id: device_id.to_string(),
            state: state.clone(),
            payload: None,
            error: if state == DeliveryState::Rejected {
                reason_code_text.clone()
            } else {
                None
            },
            correlation: None,
        },
        transport: Some(TransportMeta::Mqtt(mqtt_result_meta(
            None,
            reason_code_text.into_iter().collect(),
            reason_string_from_props_opt(packet.props()),
        ))),
    }
}

fn routed_result_from_v5_auth(device_id: &str, packet: &mqtt::packet::v5_0::Auth) -> RoutedResult {
    let reason_code = packet.reason_code();
    let reason_code_text = reason_code.map(|code| code.to_string());
    let state = reason_code.map_or(DeliveryState::Accepted, |code| {
        if code.is_success() {
            DeliveryState::Accepted
        } else {
            DeliveryState::Rejected
        }
    });
    RoutedResult {
        source: mqtt_source(device_id),
        result: CommandResult {
            command_id: "__mqtt_auth__".to_string(),
            device_id: device_id.to_string(),
            state: state.clone(),
            payload: None,
            error: if state == DeliveryState::Rejected {
                reason_code_text.clone()
            } else {
                None
            },
            correlation: None,
        },
        transport: Some(TransportMeta::Mqtt(mqtt_result_meta(
            None,
            reason_code_text.into_iter().collect(),
            reason_string_from_props_opt(packet.props()),
        ))),
    }
}

fn mqtt_result_meta(
    packet_id: Option<u16>,
    reason_codes: Vec<String>,
    reason_string: Option<String>,
) -> MqttMeta {
    MqttMeta {
        topic: String::new(),
        qos: 0,
        retain: false,
        duplicate: false,
        packet_id,
        content_type: None,
        payload_format: None,
        message_expiry_interval_secs: None,
        response_topic: None,
        correlation_data: None,
        correlation_data_bytes: None,
        topic_alias: None,
        subscription_identifiers: Vec::new(),
        user_properties: Vec::new(),
        reason_codes,
        reason_string,
    }
}

pub(crate) fn routed_result_from_packet_id(
    pending_command_ids: &mut HashMap<u16, String>,
    device_id: &str,
    packet_id: u16,
    state: DeliveryState,
    remove: bool,
) -> RoutedResult {
    routed_result_from_packet_id_with_transport(
        pending_command_ids,
        device_id,
        packet_id,
        state,
        None,
        remove,
        None,
    )
}

fn routed_result_from_packet_id_with_transport(
    pending_command_ids: &mut HashMap<u16, String>,
    device_id: &str,
    packet_id: u16,
    state: DeliveryState,
    error: Option<String>,
    remove: bool,
    transport: Option<TransportMeta>,
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
            error,
            correlation: None,
        },
        transport,
    }
}
