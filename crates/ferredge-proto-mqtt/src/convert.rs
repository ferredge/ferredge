#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(feature = "std")]
use std::{
    string::{String, ToString},
    vec::Vec,
};

#[cfg(not(feature = "std"))]
use alloc::{
    string::{String, ToString},
    vec,
    vec::Vec,
};

use ferredge_bridge::{
    BridgeCodec, BridgeHeaders, BridgeMessage, BridgeOp, BridgePayload, BridgeTransportMeta,
    MessagingAction,
};
use ferredge_core::prelude::*;
use mqtt_protocol_core::mqtt;
use serde_json::to_vec;

use crate::types::{MqttCommandConversionError, MqttCommandRef, MqttPacketRequest, MqttWirePacket};

/// Bridge codec that turns a planned bridge message into a version-specific MQTT packet request.
pub struct MqttBridgeCodec<'a> {
    value: MqttCommandRef<'a>,
}

impl<'a> MqttBridgeCodec<'a> {
    /// Creates a codec bound to one MQTT device definition.
    pub fn new(device: &'a Device<crate::types::MqttResourceAttributes>) -> Self {
        Self {
            value: MqttCommandRef { device },
        }
    }
}

impl BridgeCodec<MqttPacketRequest> for MqttBridgeCodec<'_> {
    type Error = MqttCommandConversionError;

    fn encode(&self, message: &BridgeMessage<'_>) -> Result<MqttPacketRequest, Self::Error> {
        let config = self.value.endpoint_config()?;
        let version = config.preferred_protocol_version();
        let BridgeMessage::Command(command) = message else {
            return Err(MqttCommandConversionError::InvalidBridgeMessage);
        };
        let BridgeOp::Messaging(operation) = &command.operation else {
            return Err(MqttCommandConversionError::InvalidBridgeMessage);
        };

        match operation.action {
            MessagingAction::Publish => build_publish_packet(message, version),
            MessagingAction::Subscribe => build_subscribe_packet(message, version),
            MessagingAction::Unsubscribe => build_unsubscribe_packet(message, version),
        }
    }

    fn decode(&self, _native: MqttPacketRequest) -> Result<BridgeMessage<'static>, Self::Error> {
        Err(MqttCommandConversionError::InvalidBridgeMessage)
    }
}

fn mqtt_payload_bytes(payload: &BridgePayload) -> Result<Vec<u8>, MqttCommandConversionError> {
    match payload {
        BridgePayload::Binary(bytes) => Ok(bytes.clone()),
        BridgePayload::Text(value) => Ok(value.clone().into_bytes()),
        BridgePayload::Empty => Ok(Vec::new()),
        other => to_vec(&PayloadValue::from(other.clone()))
            .map_err(MqttCommandConversionError::InvalidPayload),
    }
}

struct PublishBridgeView<'a> {
    command_id: &'a str,
    topic: &'a str,
    payload: Vec<u8>,
    delivery: Option<DeliveryGuarantee>,
    retain: bool,
    payload_format: Option<MqttPayloadFormat>,
    content_type: Option<&'a str>,
    message_expiry_interval_secs: Option<u32>,
    topic_alias: Option<u16>,
    reply_to: Option<String>,
    response_topic: Option<&'a str>,
    correlation_id: Option<String>,
    correlation_data: Option<Vec<u8>>,
    projected_headers: ProjectedHeaderRefs<'a>,
}

struct SubscribeBridgeView<'a> {
    command_id: &'a str,
    topic: &'a str,
    delivery: Option<DeliveryGuarantee>,
    durable_name: Option<&'a str>,
    shared_group: Option<&'a str>,
    no_local: bool,
    retain_as_published: bool,
    retain_handling: Option<MqttRetainHandling>,
    subscription_identifier: Option<u32>,
    projected_user_properties: Vec<(&'a str, &'a str)>,
}

struct ProjectedHeaderRefs<'a> {
    user_properties: Vec<(&'a str, &'a str)>,
    content_type: Option<&'a str>,
    http_method: Option<&'a str>,
    http_path: Option<&'a str>,
    http_status_code: Option<u16>,
}

fn publish_view_from_bridge<'a>(
    message: &'a BridgeMessage<'a>,
) -> Result<PublishBridgeView<'a>, MqttCommandConversionError> {
    let BridgeMessage::Command(bridge) = message else {
        return Err(MqttCommandConversionError::InvalidBridgeMessage);
    };
    let topic = match &bridge.route {
        ferredge_bridge::BridgeRoute::Messaging { topic } => topic.as_ref(),
        _ => return Err(MqttCommandConversionError::InvalidBridgeMessage),
    };
    let mqtt = match &bridge.transport {
        Some(BridgeTransportMeta::Mqtt(mqtt)) => Some(mqtt),
        Some(BridgeTransportMeta::Http(_)) | None => None,
    };
    let projected = project_publish_metadata(bridge.transport.as_ref(), bridge.headers.as_ref());
    Ok(PublishBridgeView {
        command_id: bridge.id.as_str(),
        topic,
        payload: mqtt_payload_bytes(
            bridge
                .payload
                .as_ref()
                .ok_or(MqttCommandConversionError::InvalidBridgeMessage)?,
        )?,
        delivery: mqtt.and_then(|mqtt| delivery_from_qos(mqtt.qos)),
        retain: mqtt.is_some_and(|mqtt| mqtt.retain),
        payload_format: mqtt
            .and_then(|mqtt| mqtt_payload_format_from_bridge(mqtt.payload_format.as_deref())),
        content_type: mqtt
            .and_then(|mqtt| mqtt.content_type.as_deref())
            .or(projected.content_type),
        message_expiry_interval_secs: mqtt.and_then(|mqtt| mqtt.message_expiry_interval_secs),
        topic_alias: mqtt.and_then(|mqtt| mqtt.topic_alias),
        reply_to: bridge
            .correlation
            .as_ref()
            .and_then(|correlation| correlation.reply_to.as_ref())
            .and_then(channel_reply_to),
        response_topic: mqtt.and_then(|mqtt| mqtt.response_topic.as_deref()),
        correlation_id: bridge
            .correlation
            .as_ref()
            .map(|correlation| correlation.request_id.to_string())
            .or_else(|| {
                mqtt.and_then(|mqtt| mqtt.correlation_data.as_ref().map(ToString::to_string))
            }),
        correlation_data: mqtt.and_then(|mqtt| mqtt.correlation_data_bytes.clone()),
        projected_headers: projected,
    })
}

fn subscription_view_from_bridge<'a>(
    message: &'a BridgeMessage<'a>,
) -> Result<SubscribeBridgeView<'a>, MqttCommandConversionError> {
    let BridgeMessage::Command(bridge) = message else {
        return Err(MqttCommandConversionError::InvalidBridgeMessage);
    };
    let channel = match &bridge.route {
        ferredge_bridge::BridgeRoute::Messaging { topic } => topic.as_ref(),
        _ => return Err(MqttCommandConversionError::InvalidBridgeMessage),
    };
    let mqtt = match &bridge.transport {
        Some(BridgeTransportMeta::Mqtt(mqtt)) => Some(mqtt),
        Some(BridgeTransportMeta::Http(_)) | None => None,
    };
    let projected =
        project_subscription_metadata(bridge.transport.as_ref(), bridge.headers.as_ref());
    Ok(SubscribeBridgeView {
        command_id: bridge.id.as_str(),
        topic: channel,
        delivery: mqtt.and_then(|mqtt| delivery_from_qos(mqtt.qos)),
        durable_name: mqtt.and_then(|mqtt| mqtt.durable_name.as_deref()),
        shared_group: mqtt.and_then(|mqtt| mqtt.shared_group.as_deref()),
        no_local: mqtt.is_some_and(|mqtt| mqtt.no_local),
        retain_as_published: mqtt.is_some_and(|mqtt| mqtt.retain_as_published),
        retain_handling: mqtt.and_then(|mqtt| mqtt.retain_handling),
        subscription_identifier: mqtt
            .and_then(|mqtt| mqtt.subscription_identifiers.first().copied()),
        projected_user_properties: projected.user_properties,
    })
}

struct ProjectedSubscriptionMetadata<'a> {
    user_properties: Vec<(&'a str, &'a str)>,
}

fn project_publish_metadata<'a>(
    transport: Option<&'a BridgeTransportMeta<'a>>,
    headers: Option<&'a BridgeHeaders<'a>>,
) -> ProjectedHeaderRefs<'a> {
    let mut projected = ProjectedHeaderRefs {
        user_properties: Vec::new(),
        content_type: None,
        http_method: None,
        http_path: None,
        http_status_code: None,
    };
    if let Some(transport) = transport {
        match transport {
            BridgeTransportMeta::Mqtt(_) => {}
            BridgeTransportMeta::Http(meta) => {
                projected.content_type = meta.content_type.as_deref();
                projected.http_method = meta.method.as_deref();
                projected.http_path = meta.path.as_deref();
                projected.http_status_code = meta.status_code;
            }
        }
    }
    if let Some(headers) = headers {
        match headers {
            BridgeHeaders::Mqtt(headers) => projected.user_properties.extend(
                headers
                    .iter()
                    .map(|header| (header.key.as_ref(), header.value.as_ref())),
            ),
            BridgeHeaders::Http(headers) => projected.user_properties.extend(
                headers
                    .iter()
                    .map(|header| (header.key.as_ref(), header.value.as_ref())),
            ),
        }
    }
    projected
}

fn project_subscription_metadata<'a>(
    transport: Option<&'a BridgeTransportMeta<'a>>,
    headers: Option<&'a BridgeHeaders<'a>>,
) -> ProjectedSubscriptionMetadata<'a> {
    let mut projected = ProjectedSubscriptionMetadata {
        user_properties: Vec::new(),
    };
    if let Some(BridgeTransportMeta::Http(meta)) = transport {
        if let Some(method) = &meta.method {
            projected
                .user_properties
                .push(("ferredge-http-method", method.as_ref()));
        }
        if let Some(path) = &meta.path {
            projected
                .user_properties
                .push(("ferredge-http-path", path.as_ref()));
        }
        if let Some(status_code) = meta.status_code {
            let _ = status_code;
        }
    }
    if let Some(headers) = headers {
        match headers {
            BridgeHeaders::Mqtt(headers) => projected.user_properties.extend(
                headers
                    .iter()
                    .map(|header| (header.key.as_ref(), header.value.as_ref())),
            ),
            BridgeHeaders::Http(headers) => projected.user_properties.extend(
                headers
                    .iter()
                    .map(|header| (header.key.as_ref(), header.value.as_ref())),
            ),
        }
    }
    projected
}

pub fn qos_from_delivery(delivery: Option<DeliveryGuarantee>) -> mqtt::packet::Qos {
    match delivery {
        Some(DeliveryGuarantee::ExactlyOnce) => mqtt::packet::Qos::ExactlyOnce,
        Some(DeliveryGuarantee::AtLeastOnce) => mqtt::packet::Qos::AtLeastOnce,
        _ => mqtt::packet::Qos::AtMostOnce,
    }
}

pub fn packet_id_for_qos(qos: mqtt::packet::Qos) -> Option<u16> {
    match qos {
        mqtt::packet::Qos::AtMostOnce => None,
        mqtt::packet::Qos::AtLeastOnce | mqtt::packet::Qos::ExactlyOnce => Some(1u16),
    }
}

fn delivery_from_qos(qos: Option<u8>) -> Option<DeliveryGuarantee> {
    match qos {
        Some(2) => Some(DeliveryGuarantee::ExactlyOnce),
        Some(1) => Some(DeliveryGuarantee::AtLeastOnce),
        Some(0) => Some(DeliveryGuarantee::BestEffort),
        _ => None,
    }
}

fn mqtt_payload_format_from_bridge(payload_format: Option<&str>) -> Option<MqttPayloadFormat> {
    match payload_format {
        Some("utf8") => Some(MqttPayloadFormat::Utf8),
        Some("bytes") => Some(MqttPayloadFormat::Bytes),
        _ => None,
    }
}

fn channel_reply_to(address: &Address) -> Option<String> {
    match address {
        Address::Channel(value) => Some(value.clone()),
        Address::Resource(_) => None,
    }
}

pub fn build_publish_packet(
    message: &BridgeMessage<'_>,
    version: MqttProtocolVersion,
) -> Result<MqttPacketRequest, MqttCommandConversionError> {
    let publish = publish_view_from_bridge(message)?;
    let qos = qos_from_delivery(publish.delivery);
    let packet_id = packet_id_for_qos(qos);

    match version {
        MqttProtocolVersion::V5_0 => {
            let props = build_v5_publish_props(&publish)?;
            let mut builder = mqtt::packet::v5_0::Publish::builder()
                .topic_name(publish.topic)?
                .qos(qos)
                .retain(publish.retain)
                .payload(publish.payload)
                .props(props);
            if let Some(packet_id) = packet_id {
                builder = builder.packet_id(packet_id);
            }
            builder
                .build()
                .map(|packet| MqttPacketRequest {
                    command_id: publish.command_id.to_string(),
                    packet: MqttWirePacket::V5Publish(packet),
                })
                .map_err(Into::into)
        }
        MqttProtocolVersion::V3_1_1 => {
            validate_v3_publish_support(&publish)?;
            let mut builder = mqtt::packet::v3_1_1::Publish::builder()
                .topic_name(publish.topic)?
                .qos(qos)
                .retain(publish.retain)
                .payload(publish.payload);
            if let Some(packet_id) = packet_id {
                builder = builder.packet_id(packet_id);
            }
            builder
                .build()
                .map(|packet| MqttPacketRequest {
                    command_id: publish.command_id.to_string(),
                    packet: MqttWirePacket::V3Publish(packet),
                })
                .map_err(Into::into)
        }
    }
}

pub fn build_subscribe_packet(
    message: &BridgeMessage<'_>,
    version: MqttProtocolVersion,
) -> Result<MqttPacketRequest, MqttCommandConversionError> {
    let subscription = subscription_view_from_bridge(message)?;
    let topic = mqtt_topic_from_subscription_parts(subscription.topic, subscription.shared_group);
    let mut sub_opts =
        mqtt::packet::SubOpts::new().set_qos(qos_from_delivery(subscription.delivery));
    sub_opts = sub_opts.set_nl(subscription.no_local);
    sub_opts = sub_opts.set_rap(subscription.retain_as_published);
    if let Some(retain_handling) = subscription.retain_handling {
        sub_opts = sub_opts.set_rh(retain_handling_from_core(retain_handling));
    }
    let entry = mqtt::packet::SubEntry::new(topic.as_str(), sub_opts)?;

    match version {
        MqttProtocolVersion::V5_0 => {
            let props = build_v5_subscribe_props(&subscription)?;
            mqtt::packet::v5_0::Subscribe::builder()
                .packet_id(1u16)
                .entries(vec![entry])
                .props(props)
                .build()
                .map(|packet| MqttPacketRequest {
                    command_id: subscription.command_id.to_string(),
                    packet: MqttWirePacket::V5Subscribe(packet),
                })
                .map_err(Into::into)
        }
        MqttProtocolVersion::V3_1_1 => {
            validate_v3_subscribe_support(&subscription)?;
            mqtt::packet::v3_1_1::Subscribe::builder()
                .packet_id(1u16)
                .entries(vec![entry])
                .build()
                .map(|packet| MqttPacketRequest {
                    command_id: subscription.command_id.to_string(),
                    packet: MqttWirePacket::V3Subscribe(packet),
                })
                .map_err(Into::into)
        }
    }
}

pub fn build_unsubscribe_packet(
    message: &BridgeMessage<'_>,
    version: MqttProtocolVersion,
) -> Result<MqttPacketRequest, MqttCommandConversionError> {
    let subscription = subscription_view_from_bridge(message)?;
    let topic = mqtt_topic_from_subscription_parts(subscription.topic, subscription.shared_group);

    match version {
        MqttProtocolVersion::V5_0 => {
            let props = build_v5_unsubscribe_props(&subscription)?;
            mqtt::packet::v5_0::Unsubscribe::builder()
                .packet_id(1u16)
                .entries(vec![topic.as_str()])?
                .props(props)
                .build()
                .map(|packet| MqttPacketRequest {
                    command_id: subscription.command_id.to_string(),
                    packet: MqttWirePacket::V5Unsubscribe(packet),
                })
                .map_err(Into::into)
        }
        MqttProtocolVersion::V3_1_1 => {
            validate_v3_subscribe_support(&subscription)?;
            mqtt::packet::v3_1_1::Unsubscribe::builder()
                .packet_id(1u16)
                .entries(vec![topic.as_str()])?
                .build()
                .map(|packet| MqttPacketRequest {
                    command_id: subscription.command_id.to_string(),
                    packet: MqttWirePacket::V3Unsubscribe(packet),
                })
                .map_err(Into::into)
        }
    }
}

fn build_v5_publish_props(
    publish: &PublishBridgeView<'_>,
) -> Result<mqtt::packet::Properties, MqttCommandConversionError> {
    let mut props = mqtt::packet::Properties::new();

    if let Some(payload_format) = publish.payload_format {
        props.push(mqtt::packet::Property::PayloadFormatIndicator(
            mqtt::packet::PayloadFormatIndicator::new(match payload_format {
                MqttPayloadFormat::Bytes => mqtt::packet::PayloadFormat::Binary,
                MqttPayloadFormat::Utf8 => mqtt::packet::PayloadFormat::String,
            })?,
        ));
    }
    if let Some(expiry) = publish.message_expiry_interval_secs {
        props.push(mqtt::packet::Property::MessageExpiryInterval(
            mqtt::packet::MessageExpiryInterval::new(expiry)?,
        ));
    }
    if let Some(topic_alias) = publish.topic_alias {
        props.push(mqtt::packet::Property::TopicAlias(
            mqtt::packet::TopicAlias::new(topic_alias)?,
        ));
    }

    if let Some(reply_to) = publish.response_topic.or(publish.reply_to.as_deref()) {
        props.push(mqtt::packet::Property::ResponseTopic(
            mqtt::packet::ResponseTopic::new(reply_to)?,
        ));
    }
    if let Some(content_type) = publish.content_type {
        props.push(mqtt::packet::Property::ContentType(
            mqtt::packet::ContentType::new(content_type)?,
        ));
    }

    let correlation_data = publish.correlation_data.clone().or_else(|| {
        publish
            .correlation_id
            .as_ref()
            .map(|value| value.as_bytes().to_vec())
            .or_else(|| {
                if publish.reply_to.is_some() || publish.response_topic.is_some() {
                    Some(publish.command_id.as_bytes().to_vec())
                } else {
                    None
                }
            })
    });
    if let Some(correlation_data) = correlation_data {
        props.push(mqtt::packet::Property::CorrelationData(
            mqtt::packet::CorrelationData::new(correlation_data)?,
        ));
    }

    if let Some(method) = publish.projected_headers.http_method {
        props.push(mqtt::packet::Property::UserProperty(
            mqtt::packet::UserProperty::new("ferredge-http-method", method)?,
        ));
    }
    if let Some(path) = publish.projected_headers.http_path {
        props.push(mqtt::packet::Property::UserProperty(
            mqtt::packet::UserProperty::new("ferredge-http-path", path)?,
        ));
    }
    if let Some(status_code) = publish.projected_headers.http_status_code {
        let status_code = status_code.to_string();
        props.push(mqtt::packet::Property::UserProperty(
            mqtt::packet::UserProperty::new("ferredge-http-status-code", status_code.as_str())?,
        ));
    }
    for &(key, value) in &publish.projected_headers.user_properties {
        props.push(mqtt::packet::Property::UserProperty(
            mqtt::packet::UserProperty::new(key, value)?,
        ));
    }

    Ok(props)
}

fn build_v5_subscribe_props(
    subscription: &SubscribeBridgeView<'_>,
) -> Result<mqtt::packet::Properties, MqttCommandConversionError> {
    let mut props = mqtt::packet::Properties::new();

    if let Some(subscription_identifier) = subscription.subscription_identifier {
        props.push(mqtt::packet::Property::SubscriptionIdentifier(
            mqtt::packet::SubscriptionIdentifier::new(subscription_identifier)?,
        ));
    }
    for &(key, value) in &subscription.projected_user_properties {
        props.push(mqtt::packet::Property::UserProperty(
            mqtt::packet::UserProperty::new(key, value)?,
        ));
    }

    Ok(props)
}

fn build_v5_unsubscribe_props(
    subscription: &SubscribeBridgeView<'_>,
) -> Result<mqtt::packet::Properties, MqttCommandConversionError> {
    let mut props = mqtt::packet::Properties::new();
    props.push(mqtt::packet::Property::UserProperty(
        mqtt::packet::UserProperty::new("ferredge-command-id", subscription.command_id)?,
    ));
    Ok(props)
}

fn mqtt_topic_from_subscription_parts(topic: &str, shared_group: Option<&str>) -> String {
    if let Some(shared_group) = shared_group {
        format!("$share/{shared_group}/{topic}")
    } else {
        topic.to_string()
    }
}

fn retain_handling_from_core(value: MqttRetainHandling) -> mqtt::packet::RetainHandling {
    match value {
        MqttRetainHandling::SendRetained => mqtt::packet::RetainHandling::SendRetained,
        MqttRetainHandling::SendRetainedIfNotExists => {
            mqtt::packet::RetainHandling::SendRetainedIfNotExists
        }
        MqttRetainHandling::DoNotSendRetained => mqtt::packet::RetainHandling::DoNotSendRetained,
    }
}

fn validate_v3_publish_support(
    publish: &PublishBridgeView<'_>,
) -> Result<(), MqttCommandConversionError> {
    if publish.payload_format.is_some()
        || publish.content_type.is_some()
        || publish.message_expiry_interval_secs.is_some()
        || publish.topic_alias.is_some()
        || publish.response_topic.is_some()
        || publish.correlation_data.is_some()
        || !publish.projected_headers.user_properties.is_empty()
        || publish.projected_headers.http_method.is_some()
        || publish.projected_headers.http_path.is_some()
        || publish.projected_headers.http_status_code.is_some()
        || publish.reply_to.is_some()
        || publish.correlation_id.is_some()
    {
        return Err(MqttCommandConversionError::MqttV5PublishOptionsOnV3);
    }
    Ok(())
}

fn validate_v3_subscribe_support(
    subscription: &SubscribeBridgeView<'_>,
) -> Result<(), MqttCommandConversionError> {
    if subscription.no_local
        || subscription.retain_as_published
        || subscription.retain_handling.is_some()
        || subscription.subscription_identifier.is_some()
        || !subscription.projected_user_properties.is_empty()
        || subscription.durable_name.is_some()
        || subscription.shared_group.is_some()
    {
        return Err(MqttCommandConversionError::MqttV5SubscriptionOptionsOnV3);
    }
    Ok(())
}
