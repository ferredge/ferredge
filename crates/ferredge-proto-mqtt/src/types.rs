extern crate alloc;

use alloc::borrow::Cow;

#[cfg(not(feature = "std"))]
use alloc::{
    string::{String, ToString},
    vec::Vec,
};

use ferredge_bridge::BridgePlannerError;
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
    /// Whether broker should retain message.
    pub retain: bool,
    /// Optional MQTT v5 payload format indicator.
    pub payload_format: Option<MqttPayloadFormat>,
    /// Optional MQTT v5 content type.
    pub content_type: Option<String>,
    /// Optional MQTT v5 message expiry interval in seconds.
    pub message_expiry_interval_secs: Option<u32>,
    /// Optional MQTT v5 topic alias.
    pub topic_alias: Option<u16>,
    /// Optional message headers for future v5 property mapping.
    pub headers: Vec<(String, String)>,
    /// Optional MQTT v5 user properties.
    pub user_properties: Vec<(String, String)>,
    /// Optional logical reply channel.
    pub reply_to: Option<String>,
    /// Optional MQTT v5 response topic.
    pub response_topic: Option<String>,
    /// Optional application-level correlation identifier.
    pub correlation_id: Option<String>,
    /// Optional raw MQTT v5 correlation data.
    pub correlation_data: Option<Vec<u8>>,
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
    /// Prevent broker from sending local publishes back to this client.
    pub no_local: bool,
    /// Preserve original retain flag from broker publish.
    pub retain_as_published: bool,
    /// Retained publish handling policy at subscribe time.
    pub retain_handling: Option<MqttRetainHandling>,
    /// Optional MQTT v5 subscription identifier.
    pub subscription_identifier: Option<u32>,
    /// Optional MQTT v5 user properties.
    pub user_properties: Vec<(String, String)>,
}

/// Borrowed native MQTT semantic plan shared by direct and bridge-driven outbound mapping.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub enum MqttNativePlan<'a> {
    Publish {
        command_id: Cow<'a, str>,
        topic: Cow<'a, str>,
        payload: ferredge_bridge::BridgePayload<'a>,
        delivery: Option<DeliveryGuarantee>,
        retain: bool,
        payload_format: Option<MqttPayloadFormat>,
        content_type: Option<Cow<'a, str>>,
        message_expiry_interval_secs: Option<u32>,
        topic_alias: Option<u16>,
        headers: Vec<(Cow<'a, str>, Cow<'a, str>)>,
        user_properties: Vec<(Cow<'a, str>, Cow<'a, str>)>,
        reply_to: Option<Address<'a>>,
        response_topic: Option<Cow<'a, str>>,
        correlation_id: Option<Cow<'a, str>>,
        correlation_data: Option<Vec<u8>>,
    },
    Subscribe {
        command_id: Cow<'a, str>,
        topic: Cow<'a, str>,
        delivery: Option<DeliveryGuarantee>,
        durable_name: Option<Cow<'a, str>>,
        shared_group: Option<Cow<'a, str>>,
        no_local: bool,
        retain_as_published: bool,
        retain_handling: Option<MqttRetainHandling>,
        subscription_identifier: Option<u32>,
        user_properties: Vec<(Cow<'a, str>, Cow<'a, str>)>,
    },
    Unsubscribe {
        command_id: Cow<'a, str>,
        topic: Cow<'a, str>,
    },
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
#[derive(Debug, thiserror::Error)]
pub enum MqttCommandConversionError {
    /// Routed command intent does not map to MQTT pub/sub semantics.
    #[error("unsupported intent for MQTT driver")]
    UnsupportedIntent,
    /// Requested broker channel kind cannot be mapped to MQTT topic semantics.
    #[error("unsupported broker channel kind for MQTT topic mapping: {0:?}")]
    UnsupportedChannelKind(BrokerChannelKind),
    /// Bound device endpoint is not configured for MQTT.
    #[error("device endpoint is not MQTT")]
    NonMqttEndpoint,
    /// MQTT 3.1.1 cannot represent selected publish options.
    #[error("MQTT 3.1.1 publish does not support MQTT v5 properties")]
    MqttV5PublishOptionsOnV3,
    /// MQTT 3.1.1 cannot represent selected subscription options.
    #[error("MQTT 3.1.1 subscribe does not support requested MQTT v5 subscription options")]
    MqttV5SubscriptionOptionsOnV3,
    /// Routed typed payload cannot be encoded as an MQTT publish body.
    #[error("failed to serialize MQTT payload as JSON: {0}")]
    InvalidPayload(#[from] serde_json::Error),
    /// Underlying `mqtt_protocol_core` packet builder rejected the request.
    #[error("failed to build MQTT packet: {0}")]
    PacketBuild(#[from] mqtt::result_code::MqttError),
    #[error("invalid bridge request: {0}")]
    Bridge(#[from] BridgePlannerError),
    #[error("bridge message does not describe an MQTT packet request")]
    InvalidBridgeMessage,
}

/// Borrowed view carrying enough context to convert routed command into MQTT packets.
#[derive(Debug, Clone, Copy)]
pub(crate) struct MqttCommandRef<'a> {
    /// Device-side broker configuration and metadata.
    pub device: &'a Device<MqttResourceAttributes>,
}

impl MqttCommandRef<'_> {
    /// Returns MQTT endpoint config from bound device.
    pub fn endpoint_config(&self) -> Result<&MqttEndpointConfig, MqttCommandConversionError> {
        match &self.device.endpoint {
            DeviceEndpoint::Mqtt(config) => Ok(config),
            _ => Err(MqttCommandConversionError::NonMqttEndpoint),
        }
    }
}

/// Stage of MQTT v5 enhanced authentication exchange.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MqttAuthStage {
    /// Initial connect-time enhanced authentication.
    Connect,
    /// Re-authentication on already connected session.
    Reauthenticate,
}

/// MQTT v5 enhanced authentication reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MqttAuthFlowReason {
    /// Authentication exchange completed successfully.
    Success,
    /// Broker or client requests next auth step.
    ContinueAuthentication,
    /// Re-authentication initiated on live connection.
    ReAuthenticate,
}

/// Inbound MQTT v5 auth challenge presented to auth handler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MqttAuthChallenge {
    /// Whether auth belongs to initial connect or later re-authentication.
    pub stage: MqttAuthStage,
    /// Packet reason for current auth exchange.
    pub reason: MqttAuthFlowReason,
    /// Authentication method selected for exchange.
    pub authentication_method: Option<String>,
    /// Authentication data bytes from broker.
    pub authentication_data: Option<Vec<u8>>,
    /// Optional human-readable reason string from broker.
    pub reason_string: Option<String>,
    /// Optional user properties from broker auth packet.
    pub user_properties: Vec<(String, String)>,
}

/// Outbound MQTT v5 auth response returned by auth handler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MqttAuthResponse {
    /// Reason code to send in outgoing AUTH packet.
    pub reason: MqttAuthFlowReason,
    /// Authentication method to include. If omitted, current session method is reused.
    pub authentication_method: Option<String>,
    /// Authentication data bytes to send.
    pub authentication_data: Option<Vec<u8>>,
    /// Optional human-readable reason string.
    pub reason_string: Option<String>,
    /// Optional user properties.
    pub user_properties: Vec<(String, String)>,
}

/// Provider for MQTT v5 enhanced authentication steps.
pub trait MqttAuthProvider: Send + Sync {
    /// Builds next auth response for broker challenge.
    fn respond(&self, challenge: MqttAuthChallenge) -> Result<Option<MqttAuthResponse>, String>;
}

impl<F> MqttAuthProvider for F
where
    F: Fn(MqttAuthChallenge) -> Result<Option<MqttAuthResponse>, String> + Send + Sync,
{
    fn respond(&self, challenge: MqttAuthChallenge) -> Result<Option<MqttAuthResponse>, String> {
        self(challenge)
    }
}
