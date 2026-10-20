#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(feature = "std")]
use std::{
    borrow::Cow,
    string::{String, ToString},
    vec::Vec,
};

#[cfg(not(feature = "std"))]
use alloc::{
    borrow::Cow,
    format,
    string::{String, ToString},
    vec,
    vec::Vec,
};

use ferredge_bridge::{
    BridgeMessage, BridgeOp, BridgeOutbound, MessagingAction, MessagingOp, NativeOutbound,
    ProtocolEncoder,
};
use ferredge_core::prelude::*;
use mqtt_protocol_core::mqtt;
use serde_json::to_vec;

use crate::endpoint::MqttProtocolVersion;
use crate::meta::{MqttMeta, MqttPayloadFormat, MqttRetainHandling};
use crate::types::{
    MqttCommandConversionError, MqttCommandRef, MqttNativePlan, MqttPacketRequest, MqttWirePacket,
};

/// Bridge codec that turns a planned bridge message into a version-specific MQTT packet request.
pub struct MqttBridgeCodec<'a> {
    value: MqttCommandRef<'a>,
}

impl<'a> MqttBridgeCodec<'a> {
    /// Creates a codec bound to one MQTT device definition.
    pub fn new(device: &'a crate::endpoint::MqttDevice) -> Self {
        Self {
            value: MqttCommandRef { device },
        }
    }
}

impl<'p> ProtocolEncoder<BridgeOutbound, BridgeMessage<'p>, MqttPacketRequest>
    for MqttBridgeCodec<'_>
{
    type Error = MqttCommandConversionError;

    fn encode(&self, message: BridgeMessage<'p>) -> Result<MqttPacketRequest, Self::Error> {
        let version = self.value.device.endpoint.preferred_protocol_version();
        build_packet_request(packet_semantics_from_bridge(&message)?, version)
    }
}

impl<'p> ProtocolEncoder<NativeOutbound, MqttNativePlan<'p>, MqttPacketRequest>
    for MqttBridgeCodec<'_>
{
    type Error = MqttCommandConversionError;

    fn encode(&self, plan: MqttNativePlan<'p>) -> Result<MqttPacketRequest, Self::Error> {
        let version = self.value.device.endpoint.preferred_protocol_version();
        build_packet_request(packet_semantics_from_native_plan(&plan)?, version)
    }
}

fn mqtt_payload_bytes(payload: &PayloadValue) -> Result<Vec<u8>, MqttCommandConversionError> {
    match payload {
        PayloadValue::Bytes(bytes) => Ok(bytes.to_vec()),
        PayloadValue::String(value) => Ok(value.as_bytes().to_vec()),
        PayloadValue::Null => Ok(Vec::new()),
        other => to_vec(other).map_err(MqttCommandConversionError::InvalidPayload),
    }
}

struct PublishView<'a> {
    command_id: &'a str,
    topic: &'a str,
    payload: Vec<u8>,
    delivery: Option<DeliveryGuarantee>,
    retain: bool,
    payload_format: Option<MqttPayloadFormat>,
    content_type: Option<Cow<'a, str>>,
    message_expiry_interval_secs: Option<u32>,
    topic_alias: Option<u16>,
    reply_to: Option<String>,
    response_topic: Option<Cow<'a, str>>,
    correlation_id: Option<String>,
    correlation_data: Option<Vec<u8>>,
    projected_user_properties: Vec<(&'a str, &'a str)>,
}

struct SubscribeView<'a> {
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

enum PacketSemantics<'a> {
    Publish(PublishView<'a>),
    Subscribe(SubscribeView<'a>),
    Unsubscribe(SubscribeView<'a>),
}

/// Decodes and fully validates the bridge capture through the typed MQTT view, exactly like the native path.
fn mqtt_capture_view<'a>(
    transport: Option<&'a TransportMeta<'a>>,
) -> Result<MqttMeta<'a>, MqttCommandConversionError> {
    match transport {
        Some(meta) => MqttMeta::try_from_transport_meta(meta.as_borrowed()),
        None => Ok(MqttMeta::default()),
    }
}

fn publish_view_from_bridge<'a>(
    message: &'a BridgeMessage<'a>,
) -> Result<PublishView<'a>, MqttCommandConversionError> {
    let BridgeMessage::Command(bridge) = message else {
        return Err(MqttCommandConversionError::InvalidBridgeMessage);
    };
    let BridgeOp::Messaging(MessagingOp { delivery, .. }) = &bridge.operation else {
        return Err(MqttCommandConversionError::InvalidBridgeMessage);
    };
    let topic = match &bridge.route {
        Route::Messaging { topic } => topic.as_ref(),
        _ => return Err(MqttCommandConversionError::InvalidBridgeMessage),
    };
    let mqtt = mqtt_capture_view(bridge.transport.as_ref())?;
    Ok(PublishView {
        command_id: bridge.id.as_ref(),
        topic,
        payload: mqtt_payload_bytes(
            bridge
                .payload
                .as_ref()
                .ok_or(MqttCommandConversionError::InvalidBridgeMessage)?,
        )?,
        delivery: *delivery,
        retain: mqtt.retain,
        payload_format: mqtt.payload_format,
        content_type: mqtt.content_type,
        message_expiry_interval_secs: mqtt.message_expiry_interval_secs,
        topic_alias: mqtt.topic_alias,
        reply_to: bridge
            .correlation
            .as_ref()
            .and_then(|correlation| correlation.reply_to.as_ref())
            .and_then(channel_reply_to),
        response_topic: mqtt.response_topic,
        correlation_id: bridge
            .correlation
            .as_ref()
            .map(|correlation| correlation.request_id.to_string()),
        correlation_data: mqtt.correlation_data.map(Cow::into_owned),
        projected_user_properties: header_user_properties(bridge.headers.as_ref()),
    })
}

fn subscription_view_from_bridge<'a>(
    message: &'a BridgeMessage<'a>,
) -> Result<SubscribeView<'a>, MqttCommandConversionError> {
    let BridgeMessage::Command(bridge) = message else {
        return Err(MqttCommandConversionError::InvalidBridgeMessage);
    };
    let BridgeOp::Messaging(operation) = &bridge.operation else {
        return Err(MqttCommandConversionError::InvalidBridgeMessage);
    };
    let channel = match &bridge.route {
        Route::Messaging { topic } => topic.as_ref(),
        _ => return Err(MqttCommandConversionError::InvalidBridgeMessage),
    };
    let mqtt = mqtt_capture_view(bridge.transport.as_ref())?;
    Ok(SubscribeView {
        command_id: bridge.id.as_ref(),
        topic: channel,
        delivery: operation.delivery,
        durable_name: operation.durable_name.as_deref(),
        shared_group: operation.shared_group.as_deref(),
        no_local: mqtt.no_local,
        retain_as_published: mqtt.retain_as_published,
        retain_handling: mqtt.retain_handling,
        subscription_identifier: mqtt.subscription_identifiers.first().copied(),
        projected_user_properties: header_user_properties(bridge.headers.as_ref()),
    })
}

fn publish_view_from_native_plan<'a>(
    plan: &'a MqttNativePlan<'a>,
) -> Result<PublishView<'a>, MqttCommandConversionError> {
    let MqttNativePlan::Publish {
        command_id,
        topic,
        payload,
        delivery,
        retain,
        payload_format,
        content_type,
        message_expiry_interval_secs,
        topic_alias,
        user_properties,
        reply_to,
        response_topic,
        correlation_id,
        correlation_data,
    } = plan
    else {
        unreachable!("publish view requires publish plan")
    };
    Ok(PublishView {
        command_id,
        topic,
        payload: mqtt_payload_bytes(payload)?,
        delivery: *delivery,
        retain: *retain,
        payload_format: *payload_format,
        content_type: content_type.as_deref().map(Cow::Borrowed),
        message_expiry_interval_secs: *message_expiry_interval_secs,
        topic_alias: *topic_alias,
        reply_to: reply_to.as_ref().and_then(channel_reply_to),
        response_topic: response_topic.as_deref().map(Cow::Borrowed),
        correlation_id: correlation_id.as_deref().map(ToString::to_string),
        correlation_data: correlation_data.as_deref().map(<[u8]>::to_vec),
        projected_user_properties: user_properties
            .iter()
            .map(|(key, value)| (key.as_ref(), value.as_ref()))
            .collect(),
    })
}

fn subscription_view_from_native_plan<'a>(plan: &'a MqttNativePlan<'a>) -> SubscribeView<'a> {
    match plan {
        MqttNativePlan::Subscribe {
            command_id,
            topic,
            delivery,
            durable_name,
            shared_group,
            no_local,
            retain_as_published,
            retain_handling,
            subscription_identifier,
            user_properties,
        } => SubscribeView {
            command_id,
            topic,
            delivery: *delivery,
            durable_name: durable_name.as_deref(),
            shared_group: shared_group.as_deref(),
            no_local: *no_local,
            retain_as_published: *retain_as_published,
            retain_handling: *retain_handling,
            subscription_identifier: *subscription_identifier,
            projected_user_properties: user_properties
                .iter()
                .map(|(key, value)| (key.as_ref(), value.as_ref()))
                .collect(),
        },
        MqttNativePlan::Unsubscribe { command_id, topic } => SubscribeView {
            command_id,
            topic,
            delivery: None,
            durable_name: None,
            shared_group: None,
            no_local: false,
            retain_as_published: false,
            retain_handling: None,
            subscription_identifier: None,
            projected_user_properties: Vec::new(),
        },
        MqttNativePlan::Publish { .. } => {
            unreachable!("subscription view requires non-publish plan")
        }
    }
}

fn packet_semantics_from_bridge<'a>(
    message: &'a BridgeMessage<'a>,
) -> Result<PacketSemantics<'a>, MqttCommandConversionError> {
    let BridgeMessage::Command(command) = message else {
        return Err(MqttCommandConversionError::InvalidBridgeMessage);
    };
    let BridgeOp::Messaging(operation) = &command.operation else {
        return Err(MqttCommandConversionError::InvalidBridgeMessage);
    };
    match operation.action {
        MessagingAction::Publish => {
            Ok(PacketSemantics::Publish(publish_view_from_bridge(message)?))
        }
        MessagingAction::Subscribe => Ok(PacketSemantics::Subscribe(
            subscription_view_from_bridge(message)?,
        )),
        MessagingAction::Unsubscribe => Ok(PacketSemantics::Unsubscribe(
            subscription_view_from_bridge(message)?,
        )),
        // Forward-compat: actions added later are not supported by this codec.
        _ => Err(MqttCommandConversionError::InvalidBridgeMessage),
    }
}

fn packet_semantics_from_native_plan<'a>(
    plan: &'a MqttNativePlan<'a>,
) -> Result<PacketSemantics<'a>, MqttCommandConversionError> {
    match plan {
        MqttNativePlan::Publish { .. } => Ok(PacketSemantics::Publish(
            publish_view_from_native_plan(plan)?,
        )),
        MqttNativePlan::Subscribe { .. } => Ok(PacketSemantics::Subscribe(
            subscription_view_from_native_plan(plan),
        )),
        MqttNativePlan::Unsubscribe { .. } => Ok(PacketSemantics::Unsubscribe(
            subscription_view_from_native_plan(plan),
        )),
    }
}

fn header_user_properties<'a>(headers: Option<&'a Headers<'a>>) -> Vec<(&'a str, &'a str)> {
    headers
        .map(|headers| {
            headers
                .iter()
                .map(|header| (header.key.as_ref(), header.value.as_ref()))
                .collect()
        })
        .unwrap_or_default()
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

fn channel_reply_to(address: &Address) -> Option<String> {
    match address {
        Address::Channel(value) => Some(value.to_string()),
        _ => None,
    }
}

fn build_publish_packet(
    publish: PublishView<'_>,
    version: MqttProtocolVersion,
) -> Result<MqttPacketRequest, MqttCommandConversionError> {
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

fn build_subscribe_packet(
    subscription: SubscribeView<'_>,
    version: MqttProtocolVersion,
) -> Result<MqttPacketRequest, MqttCommandConversionError> {
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

fn build_unsubscribe_packet(
    subscription: SubscribeView<'_>,
    version: MqttProtocolVersion,
) -> Result<MqttPacketRequest, MqttCommandConversionError> {
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

fn build_packet_request(
    semantics: PacketSemantics<'_>,
    version: MqttProtocolVersion,
) -> Result<MqttPacketRequest, MqttCommandConversionError> {
    match semantics {
        PacketSemantics::Publish(publish) => build_publish_packet(publish, version),
        PacketSemantics::Subscribe(subscription) => build_subscribe_packet(subscription, version),
        PacketSemantics::Unsubscribe(subscription) => {
            build_unsubscribe_packet(subscription, version)
        }
    }
}

fn build_v5_publish_props(
    publish: &PublishView<'_>,
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

    if let Some(reply_to) = publish
        .response_topic
        .as_deref()
        .or(publish.reply_to.as_deref())
    {
        props.push(mqtt::packet::Property::ResponseTopic(
            mqtt::packet::ResponseTopic::new(reply_to)?,
        ));
    }
    if let Some(content_type) = publish.content_type.as_deref() {
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

    for &(key, value) in &publish.projected_user_properties {
        props.push(mqtt::packet::Property::UserProperty(
            mqtt::packet::UserProperty::new(key, value)?,
        ));
    }

    Ok(props)
}

fn build_v5_subscribe_props(
    subscription: &SubscribeView<'_>,
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
    subscription: &SubscribeView<'_>,
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
    publish: &PublishView<'_>,
) -> Result<(), MqttCommandConversionError> {
    if publish.payload_format.is_some()
        || publish.content_type.is_some()
        || publish.message_expiry_interval_secs.is_some()
        || publish.topic_alias.is_some()
        || publish.response_topic.is_some()
        || publish.correlation_data.is_some()
        || !publish.projected_user_properties.is_empty()
        || publish.reply_to.is_some()
        || publish.correlation_id.is_some()
    {
        return Err(MqttCommandConversionError::MqttV5PublishOptionsOnV3);
    }
    Ok(())
}

fn validate_v3_subscribe_support(
    subscription: &SubscribeView<'_>,
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
