#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(feature = "std")]
use std::string::String;

#[cfg(not(feature = "std"))]
use alloc::{string::String, vec};

use ferredge_bridge::{BridgeMessage, BridgeOp, BridgePayload, MessagingAction};
use ferredge_core::prelude::*;
use mqtt_protocol_core::mqtt;
use serde_json::to_vec;

use crate::types::{
    MqttCommandConversionError, MqttCommandRef, MqttPacketRequest, MqttPublishRequest,
    MqttSubscriptionRequest, MqttWirePacket,
};

pub(crate) fn encode_packet_request(
    value: MqttCommandRef<'_>,
    message: &BridgeMessage,
) -> Result<MqttPacketRequest, MqttCommandConversionError> {
    let config = value.endpoint_config()?;
    let version = config.preferred_protocol_version();
    let BridgeMessage::Command(command) = message else {
        return Err(MqttCommandConversionError::InvalidBridgeMessage);
    };
    let BridgeOp::Messaging(operation) = &command.operation else {
        return Err(MqttCommandConversionError::InvalidBridgeMessage);
    };

    match operation.action {
        MessagingAction::Publish => build_publish_packet(value.command, message, version),
        MessagingAction::Subscribe => build_subscribe_packet(value.command, message, version),
        MessagingAction::Unsubscribe => build_unsubscribe_packet(value.command, message, version),
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

fn publish_request_from_bridge(
    command: &Command,
    message: &BridgeMessage,
) -> Result<MqttPublishRequest, MqttCommandConversionError> {
    let BridgeMessage::Command(bridge) = message else {
        return Err(MqttCommandConversionError::InvalidBridgeMessage);
    };
    let Intent::Send {
        channel, options, ..
    } = &command.intent
    else {
        return Err(MqttCommandConversionError::UnsupportedIntent);
    };
    let mqtt = match &options.protocol {
        Some(BrokerMessageProtocolOptions::Mqtt(mqtt)) => mqtt.clone(),
        None => MqttMessageOptions::default(),
    };
    Ok(MqttPublishRequest {
        command_id: bridge.id.clone(),
        channel: channel.clone(),
        payload: mqtt_payload_bytes(
            bridge
                .payload
                .as_ref()
                .ok_or(MqttCommandConversionError::InvalidBridgeMessage)?,
        )?,
        delivery: options.delivery,
        retain: mqtt.retain,
        payload_format: mqtt.payload_format,
        content_type: mqtt
            .content_type
            .or_else(|| bridge.meta.content_type.clone()),
        message_expiry_interval_secs: mqtt.message_expiry_interval_secs,
        topic_alias: mqtt.topic_alias,
        headers: options.headers.clone(),
        user_properties: mqtt.user_properties,
        reply_to: options.reply_to.clone(),
        response_topic: mqtt.response_topic,
        correlation_id: options
            .correlation_id
            .clone()
            .or_else(|| bridge.meta.correlation_id.clone()),
        correlation_data: mqtt.correlation_data,
    })
}

fn subscription_request_from_bridge(
    command: &Command,
    message: &BridgeMessage,
) -> Result<MqttSubscriptionRequest, MqttCommandConversionError> {
    let BridgeMessage::Command(bridge) = message else {
        return Err(MqttCommandConversionError::InvalidBridgeMessage);
    };

    match &command.intent {
        Intent::Subscribe { channel, options } => {
            let mqtt = match &options.protocol {
                Some(BrokerSubscriptionProtocolOptions::Mqtt(mqtt)) => mqtt.clone(),
                None => MqttSubscriptionOptions::default(),
            };
            Ok(MqttSubscriptionRequest {
                command_id: bridge.id.clone(),
                channel: channel.clone(),
                delivery: options.delivery,
                durable_name: options.durable_name.clone(),
                shared_group: options.shared_group.clone(),
                no_local: mqtt.no_local,
                retain_as_published: mqtt.retain_as_published,
                retain_handling: mqtt.retain_handling,
                subscription_identifier: mqtt.subscription_identifier,
                user_properties: mqtt.user_properties,
            })
        }
        Intent::Unsubscribe { channel } => Ok(MqttSubscriptionRequest {
            command_id: bridge.id.clone(),
            channel: channel.to_owned(),
            delivery: None,
            durable_name: None,
            shared_group: None,
            no_local: false,
            retain_as_published: false,
            retain_handling: None,
            subscription_identifier: None,
            user_properties: Vec::new(),
        }),
        _ => Err(MqttCommandConversionError::UnsupportedIntent),
    }
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

pub fn build_publish_packet(
    command: &Command,
    message: &BridgeMessage,
    version: MqttProtocolVersion,
) -> Result<MqttPacketRequest, MqttCommandConversionError> {
    let publish = publish_request_from_bridge(command, message)?;
    let topic = mqtt_topic_from_address(&publish.channel)?;
    let qos = qos_from_delivery(publish.delivery);
    let packet_id = packet_id_for_qos(qos);

    match version {
        MqttProtocolVersion::V5_0 => {
            let props = build_v5_publish_props(&publish)?;
            let mut builder = mqtt::packet::v5_0::Publish::builder()
                .topic_name(topic.as_str())?
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
                    command_id: publish.command_id,
                    packet: MqttWirePacket::V5Publish(packet),
                })
                .map_err(Into::into)
        }
        MqttProtocolVersion::V3_1_1 => {
            validate_v3_publish_support(&publish)?;
            let mut builder = mqtt::packet::v3_1_1::Publish::builder()
                .topic_name(topic.as_str())?
                .qos(qos)
                .retain(publish.retain)
                .payload(publish.payload);
            if let Some(packet_id) = packet_id {
                builder = builder.packet_id(packet_id);
            }
            builder
                .build()
                .map(|packet| MqttPacketRequest {
                    command_id: publish.command_id,
                    packet: MqttWirePacket::V3Publish(packet),
                })
                .map_err(Into::into)
        }
    }
}

pub fn build_subscribe_packet(
    command: &Command,
    message: &BridgeMessage,
    version: MqttProtocolVersion,
) -> Result<MqttPacketRequest, MqttCommandConversionError> {
    let subscription = subscription_request_from_bridge(command, message)?;
    let topic = mqtt_topic_from_subscription(&subscription)?;
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
                    command_id: subscription.command_id.clone(),
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
                    command_id: subscription.command_id.clone(),
                    packet: MqttWirePacket::V3Subscribe(packet),
                })
                .map_err(Into::into)
        }
    }
}

pub fn build_unsubscribe_packet(
    command: &Command,
    message: &BridgeMessage,
    version: MqttProtocolVersion,
) -> Result<MqttPacketRequest, MqttCommandConversionError> {
    let subscription = subscription_request_from_bridge(command, message)?;
    let topic = mqtt_topic_from_subscription(&subscription)?;

    match version {
        MqttProtocolVersion::V5_0 => {
            let props = build_v5_unsubscribe_props(&subscription)?;
            mqtt::packet::v5_0::Unsubscribe::builder()
                .packet_id(1u16)
                .entries(vec![topic.as_str()])?
                .props(props)
                .build()
                .map(|packet| MqttPacketRequest {
                    command_id: subscription.command_id.clone(),
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
                    command_id: subscription.command_id,
                    packet: MqttWirePacket::V3Unsubscribe(packet),
                })
                .map_err(Into::into)
        }
    }
}

pub fn build_v5_publish_props(
    publish: &MqttPublishRequest,
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
        .as_ref()
        .or(publish.reply_to.as_ref())
    {
        props.push(mqtt::packet::Property::ResponseTopic(
            mqtt::packet::ResponseTopic::new(reply_to.as_str())?,
        ));
    }
    if let Some(content_type) = &publish.content_type {
        props.push(mqtt::packet::Property::ContentType(
            mqtt::packet::ContentType::new(content_type.as_str())?,
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

    for (key, value) in &publish.headers {
        if !key.eq_ignore_ascii_case("content-type") {
            props.push(mqtt::packet::Property::UserProperty(
                mqtt::packet::UserProperty::new(key.as_str(), value.as_str())?,
            ));
        }
    }
    for (key, value) in &publish.user_properties {
        props.push(mqtt::packet::Property::UserProperty(
            mqtt::packet::UserProperty::new(key.as_str(), value.as_str())?,
        ));
    }

    Ok(props)
}

pub fn build_v5_subscribe_props(
    subscription: &MqttSubscriptionRequest,
) -> Result<mqtt::packet::Properties, MqttCommandConversionError> {
    let mut props = mqtt::packet::Properties::new();

    if let Some(subscription_identifier) = subscription.subscription_identifier {
        props.push(mqtt::packet::Property::SubscriptionIdentifier(
            mqtt::packet::SubscriptionIdentifier::new(subscription_identifier)?,
        ));
    }
    for (key, value) in &subscription.user_properties {
        props.push(mqtt::packet::Property::UserProperty(
            mqtt::packet::UserProperty::new(key.as_str(), value.as_str())?,
        ));
    }

    Ok(props)
}

pub fn build_v5_unsubscribe_props(
    subscription: &MqttSubscriptionRequest,
) -> Result<mqtt::packet::Properties, MqttCommandConversionError> {
    let mut props = mqtt::packet::Properties::new();
    props.push(mqtt::packet::Property::UserProperty(
        mqtt::packet::UserProperty::new("ferredge-command-id", subscription.command_id.as_str())?,
    ));
    Ok(props)
}

pub fn mqtt_topic_from_address(
    value: &BrokerAddress,
) -> Result<String, MqttCommandConversionError> {
    match value.kind {
        None | Some(BrokerChannelKind::Topic) | Some(BrokerChannelKind::Subject) => {
            Ok(value.name.clone())
        }
        Some(kind @ (BrokerChannelKind::Queue | BrokerChannelKind::Stream)) => {
            Err(MqttCommandConversionError::UnsupportedChannelKind(kind))
        }
    }
}

fn mqtt_topic_from_subscription(
    subscription: &MqttSubscriptionRequest,
) -> Result<String, MqttCommandConversionError> {
    let topic = mqtt_topic_from_address(&subscription.channel)?;
    if let Some(shared_group) = &subscription.shared_group {
        Ok(format!("$share/{shared_group}/{topic}"))
    } else {
        Ok(topic)
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
    publish: &MqttPublishRequest,
) -> Result<(), MqttCommandConversionError> {
    if publish.payload_format.is_some()
        || publish.content_type.is_some()
        || publish.message_expiry_interval_secs.is_some()
        || publish.topic_alias.is_some()
        || publish.response_topic.is_some()
        || publish.correlation_data.is_some()
        || !publish.user_properties.is_empty()
        || !publish.headers.is_empty()
        || publish.reply_to.is_some()
        || publish.correlation_id.is_some()
    {
        return Err(MqttCommandConversionError::MqttV5PublishOptionsOnV3);
    }
    Ok(())
}

fn validate_v3_subscribe_support(
    subscription: &MqttSubscriptionRequest,
) -> Result<(), MqttCommandConversionError> {
    if subscription.no_local
        || subscription.retain_as_published
        || subscription.retain_handling.is_some()
        || subscription.subscription_identifier.is_some()
        || !subscription.user_properties.is_empty()
        || subscription.durable_name.is_some()
    {
        return Err(MqttCommandConversionError::MqttV5SubscriptionOptionsOnV3);
    }
    Ok(())
}
