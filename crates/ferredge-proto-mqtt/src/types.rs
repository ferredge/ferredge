#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(feature = "std")]
use std::{string::String, vec::Vec};

#[cfg(not(feature = "std"))]
use alloc::{string::String, vec::Vec};

use ferredge_core::prelude::*;
use mqtt_protocol_core::mqtt;
use serde::{Deserialize, Serialize};

/// MQTT-specific resource metadata placeholder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MqttResourceAttributes;

impl DeviceResourceAttributes for MqttResourceAttributes {}

/// Native MQTT publish request understood by adapter conversion layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MqttPublishRequest {
    /// Original routed command identifier.
    pub command_id: String,
    /// Broker channel derived from routed command.
    pub channel: BrokerAddress,
    /// Raw message payload.
    pub payload: Vec<u8>,
    /// Delivery guarantee requested by routed command.
    pub delivery: Option<DeliveryGuarantee>,
    /// Optional message headers for future v5 property mapping.
    pub headers: Vec<(String, String)>,
    /// Optional logical reply channel.
    pub reply_to: Option<String>,
    /// Optional application-level correlation identifier.
    pub correlation_id: Option<String>,
}

/// Native MQTT subscription request understood by adapter conversion layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MqttSubscriptionRequest {
    /// Original routed command identifier.
    pub command_id: String,
    /// Broker channel to subscribe to.
    pub channel: BrokerAddress,
    /// Delivery guarantee requested by routed command.
    pub delivery: Option<DeliveryGuarantee>,
    /// Optional durable name for brokers that support it.
    pub durable_name: Option<String>,
    /// Optional shared consumer group for brokers that support it.
    pub shared_group: Option<String>,
}

/// Version-aware MQTT native request produced from routed commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MqttWirePacket {
    /// MQTT publish packet serialized through MQTT 5.0 rules.
    V5Publish(mqtt::packet::v5_0::Publish),
    /// MQTT publish packet serialized through MQTT 3.1.1 rules.
    V3Publish(mqtt::packet::v3_1_1::Publish),
    /// MQTT subscribe packet serialized through MQTT 5.0 rules.
    V5Subscribe(mqtt::packet::v5_0::Subscribe),
    /// MQTT subscribe packet serialized through MQTT 3.1.1 rules.
    V3Subscribe(mqtt::packet::v3_1_1::Subscribe),
    /// MQTT unsubscribe packet serialized through MQTT 5.0 rules.
    V5Unsubscribe(mqtt::packet::v5_0::Unsubscribe),
    /// MQTT unsubscribe packet serialized through MQTT 3.1.1 rules.
    V3Unsubscribe(mqtt::packet::v3_1_1::Unsubscribe),
}

/// Version-aware MQTT request plus original command identity for correlation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MqttPacketRequest {
    /// Original routed command identifier.
    pub command_id: String,
    /// Concrete MQTT packet to send.
    pub packet: MqttWirePacket,
}

/// MQTT conversion errors raised while projecting routed commands into MQTT packets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MqttCommandConversionError {
    /// Routed command intent does not map to MQTT pub/sub semantics.
    UnsupportedIntent,
    /// Requested broker channel kind cannot be mapped to MQTT topic semantics.
    UnsupportedChannelKind,
    /// Routed command omitted packet details required for selected MQTT operation.
    InvalidCommand(String),
    /// Underlying `mqtt_protocol_core` packet builder rejected the request.
    PacketBuild(String),
}

impl core::fmt::Display for MqttCommandConversionError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnsupportedIntent => write!(f, "unsupported intent for MQTT driver"),
            Self::UnsupportedChannelKind => {
                write!(f, "unsupported broker channel kind for MQTT topic mapping")
            }
            Self::InvalidCommand(message) => write!(f, "{message}"),
            Self::PacketBuild(message) => write!(f, "{message}"),
        }
    }
}

/// Borrowed view carrying enough context to convert routed command into MQTT packets.
#[derive(Debug, Clone, Copy)]
pub struct MqttCommandRef<'a> {
    /// Device-side broker configuration and metadata.
    pub device: &'a Device<MqttResourceAttributes>,
    /// Routed command to convert.
    pub command: &'a Command,
}

impl MqttCommandRef<'_> {
    /// Returns MQTT endpoint config from bound device.
    pub fn endpoint_config(&self) -> Result<&MqttEndpointConfig, MqttCommandConversionError> {
        match &self.device.endpoint {
            DeviceEndpoint::Mqtt(config) => Ok(config),
            _ => Err(MqttCommandConversionError::InvalidCommand(
                "device endpoint is not MQTT".to_string(),
            )),
        }
    }
}
