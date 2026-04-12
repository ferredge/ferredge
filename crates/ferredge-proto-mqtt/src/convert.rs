#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(feature = "std")]
use std::string::{String, ToString};

#[cfg(not(feature = "std"))]
use alloc::{
    string::{String, ToString},
    vec,
};

use ferredge_core::prelude::*;
use mqtt_protocol_core::mqtt;

use crate::types::{
    MqttCommandConversionError, MqttCommandRef, MqttPacketRequest, MqttPublishRequest,
    MqttSubscriptionRequest, MqttWirePacket,
};

impl TryFrom<&Command> for MqttPublishRequest {
    type Error = MqttCommandConversionError;

    fn try_from(command: &Command) -> Result<Self, Self::Error> {
        match &command.intent {
            Intent::Send {
                channel,
                payload,
                options,
            } => Ok(Self {
                command_id: command.id.clone(),
                channel: channel.clone(),
                payload: payload.clone(),
                delivery: options.delivery,
                headers: options.headers.clone(),
                reply_to: options.reply_to.clone(),
                correlation_id: options.correlation_id.clone(),
            }),
            _ => Err(MqttCommandConversionError::UnsupportedIntent),
        }
    }
}

impl TryFrom<&Command> for MqttSubscriptionRequest {
    type Error = MqttCommandConversionError;

    fn try_from(command: &Command) -> Result<Self, Self::Error> {
        match &command.intent {
            Intent::Subscribe { channel, options } => Ok(Self {
                command_id: command.id.clone(),
                channel: channel.clone(),
                delivery: options.delivery,
                durable_name: options.durable_name.clone(),
                shared_group: options.shared_group.clone(),
            }),
            Intent::Unsubscribe { channel } => Ok(Self {
                command_id: command.id.clone(),
                channel: channel.clone(),
                delivery: None,
                durable_name: None,
                shared_group: None,
            }),
            _ => Err(MqttCommandConversionError::UnsupportedIntent),
        }
    }
}

impl TryFrom<MqttCommandRef<'_>> for MqttPacketRequest {
    type Error = MqttCommandConversionError;

    fn try_from(value: MqttCommandRef<'_>) -> Result<Self, Self::Error> {
        let config = value.endpoint_config()?;
        let version = config.preferred_protocol_version();

        match &value.command.intent {
            Intent::Send { .. } => build_publish_packet(value.command, version),
            Intent::Subscribe { .. } => build_subscribe_packet(value.command, version),
            Intent::Unsubscribe { .. } => build_unsubscribe_packet(value.command, version),
            _ => Err(MqttCommandConversionError::UnsupportedIntent),
        }
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
    version: MqttProtocolVersion,
) -> Result<MqttPacketRequest, MqttCommandConversionError> {
    let publish = MqttPublishRequest::try_from(command)?;
    let topic = mqtt_topic_from_address(&publish.channel)?;
    let qos = qos_from_delivery(publish.delivery);
    let packet_id = packet_id_for_qos(qos);

    match version {
        MqttProtocolVersion::V5_0 => {
            let props = build_v5_publish_props(&publish)?;
            let mut builder = mqtt::packet::v5_0::Publish::builder()
                .topic_name(topic.as_str())
                .map_err(|e| MqttCommandConversionError::PacketBuild(e.to_string()))?
                .qos(qos)
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
                .map_err(|e| MqttCommandConversionError::PacketBuild(e.to_string()))
        }
        MqttProtocolVersion::V3_1_1 => {
            let mut builder = mqtt::packet::v3_1_1::Publish::builder()
                .topic_name(topic.as_str())
                .map_err(|e| MqttCommandConversionError::PacketBuild(e.to_string()))?
                .qos(qos)
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
                .map_err(|e| MqttCommandConversionError::PacketBuild(e.to_string()))
        }
    }
}

pub fn build_subscribe_packet(
    command: &Command,
    version: MqttProtocolVersion,
) -> Result<MqttPacketRequest, MqttCommandConversionError> {
    let subscription = MqttSubscriptionRequest::try_from(command)?;
    let topic = mqtt_topic_from_address(&subscription.channel)?;
    let entry = mqtt::packet::SubEntry::new(
        topic.as_str(),
        mqtt::packet::SubOpts::new().set_qos(qos_from_delivery(subscription.delivery)),
    )
    .map_err(|e| MqttCommandConversionError::PacketBuild(e.to_string()))?;

    match version {
        MqttProtocolVersion::V5_0 => mqtt::packet::v5_0::Subscribe::builder()
            .packet_id(1u16)
            .entries(vec![entry])
            .build()
            .map(|packet| MqttPacketRequest {
                command_id: subscription.command_id.clone(),
                packet: MqttWirePacket::V5Subscribe(packet),
            })
            .map_err(|e| MqttCommandConversionError::PacketBuild(e.to_string())),
        MqttProtocolVersion::V3_1_1 => mqtt::packet::v3_1_1::Subscribe::builder()
            .packet_id(1u16)
            .entries(vec![entry])
            .build()
            .map(|packet| MqttPacketRequest {
                command_id: subscription.command_id.clone(),
                packet: MqttWirePacket::V3Subscribe(packet),
            })
            .map_err(|e| MqttCommandConversionError::PacketBuild(e.to_string())),
    }
}

pub fn build_unsubscribe_packet(
    command: &Command,
    version: MqttProtocolVersion,
) -> Result<MqttPacketRequest, MqttCommandConversionError> {
    let subscription = MqttSubscriptionRequest::try_from(command)?;
    let topic = mqtt_topic_from_address(&subscription.channel)?;

    match version {
        MqttProtocolVersion::V5_0 => mqtt::packet::v5_0::Unsubscribe::builder()
            .packet_id(1u16)
            .entries(vec![topic.as_str()])
            .map_err(|e| MqttCommandConversionError::PacketBuild(e.to_string()))?
            .build()
            .map(|packet| MqttPacketRequest {
                command_id: subscription.command_id.clone(),
                packet: MqttWirePacket::V5Unsubscribe(packet),
            })
            .map_err(|e| MqttCommandConversionError::PacketBuild(e.to_string())),
        MqttProtocolVersion::V3_1_1 => mqtt::packet::v3_1_1::Unsubscribe::builder()
            .packet_id(1u16)
            .entries(vec![topic.as_str()])
            .map_err(|e| MqttCommandConversionError::PacketBuild(e.to_string()))?
            .build()
            .map(|packet| MqttPacketRequest {
                command_id: subscription.command_id,
                packet: MqttWirePacket::V3Unsubscribe(packet),
            })
            .map_err(|e| MqttCommandConversionError::PacketBuild(e.to_string())),
    }
}

pub fn build_v5_publish_props(
    publish: &MqttPublishRequest,
) -> Result<mqtt::packet::Properties, MqttCommandConversionError> {
    let mut props = mqtt::packet::Properties::new();

    if let Some(reply_to) = &publish.reply_to {
        props.push(mqtt::packet::Property::ResponseTopic(
            mqtt::packet::ResponseTopic::new(reply_to.as_str())
                .map_err(|e| MqttCommandConversionError::PacketBuild(e.to_string()))?,
        ));
    }
    if let Some(correlation_id) = &publish.correlation_id {
        props.push(mqtt::packet::Property::CorrelationData(
            mqtt::packet::CorrelationData::new(correlation_id.as_bytes().to_vec())
                .map_err(|e| MqttCommandConversionError::PacketBuild(e.to_string()))?,
        ));
    }

    for (key, value) in &publish.headers {
        if key.eq_ignore_ascii_case("content-type") {
            props.push(mqtt::packet::Property::ContentType(
                mqtt::packet::ContentType::new(value.as_str())
                    .map_err(|e| MqttCommandConversionError::PacketBuild(e.to_string()))?,
            ));
        } else {
            props.push(mqtt::packet::Property::UserProperty(
                mqtt::packet::UserProperty::new(key.as_str(), value.as_str())
                    .map_err(|e| MqttCommandConversionError::PacketBuild(e.to_string()))?,
            ));
        }
    }

    Ok(props)
}

pub fn mqtt_topic_from_address(
    value: &BrokerAddress,
) -> Result<String, MqttCommandConversionError> {
    match value.kind {
        None | Some(BrokerChannelKind::Topic) | Some(BrokerChannelKind::Subject) => {
            Ok(value.name.clone())
        }
        Some(BrokerChannelKind::Queue) | Some(BrokerChannelKind::Stream) => {
            Err(MqttCommandConversionError::UnsupportedChannelKind)
        }
    }
}
