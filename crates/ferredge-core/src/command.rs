extern crate alloc;

use alloc::{string::String, vec::Vec};
use core::fmt;

use crate::device::DeviceId;
use serde::{Deserialize, Serialize};

/// Unique identifier for command lifecycle and correlation.
pub type CommandId = String;

/// High-level lifecycle state for command delivery and completion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeliveryState {
    /// Transport accepted command for further processing.
    Accepted,
    /// Router or transport dispatched command to target.
    Dispatched,
    /// Command completed successfully and may carry payload.
    Completed,
    /// Command failed and should carry error details.
    Rejected,
    /// Command expired before completion.
    TimedOut,
}

/// Correlates async replies or completion events with original request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Correlation {
    /// Identifier of original command or request.
    pub request_id: CommandId,
    /// Optional logical reply destination such as topic or resource.
    pub reply_to: Option<Address>,
}

/// Protocol-neutral logical address used by routed messages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Address {
    /// Resource-oriented address such as HTTP path or Modbus register alias.
    Resource(String),
    /// Channel-oriented address such as MQTT or NATS topic.
    Channel(String),
}

/// Common delivery guarantees shared by broker-style protocols.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeliveryGuarantee {
    /// Delivery may be dropped without retry.
    BestEffort,
    /// Delivery must be retried until receiver gets at least one copy.
    AtLeastOnce,
    /// Delivery must arrive exactly once when transport supports it.
    ExactlyOnce,
}

/// MQTT payload format indicator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MqttPayloadFormat {
    /// Payload is arbitrary binary bytes.
    Bytes,
    /// Payload should be interpreted as UTF-8 text.
    Utf8,
}

/// MQTT retained-message handling policy for subscriptions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MqttRetainHandling {
    /// Send retained messages when subscription is established.
    SendRetained,
    /// Send retained messages only when subscription does not already exist.
    SendRetainedIfNotExists,
    /// Do not send retained messages on subscribe.
    DoNotSendRetained,
}

/// Protocol-neutral broker send options preserved in routed command layer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct BrokerMessageOptions {
    /// Optional delivery guarantee requested by caller.
    pub delivery: Option<DeliveryGuarantee>,
    /// Optional message headers or attributes.
    pub headers: Vec<(String, String)>,
    /// Optional logical reply channel.
    pub reply_to: Option<String>,
    /// Optional application-level correlation identifier.
    pub correlation_id: Option<String>,
    /// Optional protocol-specific broker extension.
    pub protocol: Option<BrokerMessageProtocolOptions>,
}

/// Protocol-neutral broker subscription options preserved in routed command layer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct BrokerSubscriptionOptions {
    /// Optional delivery guarantee preference for consumed messages.
    pub delivery: Option<DeliveryGuarantee>,
    /// Optional durable consumer or subscription name.
    pub durable_name: Option<String>,
    /// Optional shared consumer group identifier.
    pub shared_group: Option<String>,
    /// Optional protocol-specific subscription extension.
    pub protocol: Option<BrokerSubscriptionProtocolOptions>,
}

/// Tagged protocol-specific broker send options.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BrokerMessageProtocolOptions {
    /// MQTT-specific publish options.
    Mqtt(MqttMessageOptions),
}

/// MQTT-specific broker send options.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MqttMessageOptions {
    /// Whether broker should retain message for future subscribers.
    pub retain: bool,
    /// Optional MQTT v5 payload format indicator.
    pub payload_format: Option<MqttPayloadFormat>,
    /// Optional MQTT v5 content type.
    pub content_type: Option<String>,
    /// Optional MQTT v5 message expiry interval in seconds.
    pub message_expiry_interval_secs: Option<u32>,
    /// Optional MQTT v5 topic alias.
    pub topic_alias: Option<u16>,
    /// Optional MQTT v5 user properties.
    pub user_properties: Vec<(String, String)>,
    /// Optional MQTT v5 response topic. If omitted, generic `reply_to` is used when possible.
    pub response_topic: Option<String>,
    /// Optional raw MQTT v5 correlation data.
    pub correlation_data: Option<Vec<u8>>,
}

/// Tagged protocol-specific broker subscription options.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BrokerSubscriptionProtocolOptions {
    /// MQTT-specific subscribe options.
    Mqtt(MqttSubscriptionOptions),
}

/// MQTT-specific broker subscription options.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MqttSubscriptionOptions {
    /// Prevent broker from sending messages published by same client.
    pub no_local: bool,
    /// Preserve original RETAIN flag on forwarded messages.
    pub retain_as_published: bool,
    /// Retained-message delivery policy when subscription starts.
    pub retain_handling: Option<MqttRetainHandling>,
    /// Optional MQTT v5 subscription identifier.
    pub subscription_identifier: Option<u32>,
    /// Optional MQTT v5 user properties.
    pub user_properties: Vec<(String, String)>,
}

/// Generic broker address used across topic, queue, subject, and stream transports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrokerAddress {
    /// Broker-specific destination or subscription name.
    pub name: String,
    /// Optional semantic classification of broker address.
    pub kind: Option<BrokerChannelKind>,
}

/// Common broker channel categories used by transport-neutral routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BrokerChannelKind {
    /// Topic-like fan-out destination.
    Topic,
    /// Queue-like work distribution destination.
    Queue,
    /// Subject-like routed destination.
    Subject,
    /// Stream or log-like destination.
    Stream,
}

/// Protocol-neutral typed payload carried across commands, results, and routing.
/// 
/// **Note:** f64 doesn't support `Eq` trait due to `NaN` semantics, so `PayloadValue` either.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PayloadValue {
    /// Explicit absence of payload content.
    Null,
    /// Boolean scalar.
    Bool(bool),
    /// Signed integer scalar.
    I64(i64),
    /// Unsigned integer scalar.
    U64(u64),
    /// Floating-point scalar.
    F64(f64),
    /// UTF-8 text payload.
    String(String),
    /// Opaque binary payload.
    Bytes(Vec<u8>),
    /// Ordered heterogeneous sequence payload.
    List(Vec<PayloadValue>),
    /// Ordered object payload.
    Map(Vec<(String, PayloadValue)>),
}

impl From<Vec<u8>> for PayloadValue {
    fn from(value: Vec<u8>) -> Self {
        Self::Bytes(value)
    }
}

impl From<&[u8]> for PayloadValue {
    fn from(value: &[u8]) -> Self {
        Self::Bytes(value.to_vec())
    }
}

impl From<String> for PayloadValue {
    fn from(value: String) -> Self {
        Self::String(value)
    }
}

impl From<&str> for PayloadValue {
    fn from(value: &str) -> Self {
        Self::String(value.to_string())
    }
}

impl fmt::Display for PayloadValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Null => write!(f, "null"),
            Self::Bool(value) => write!(f, "{value}"),
            Self::I64(value) => write!(f, "{value}"),
            Self::U64(value) => write!(f, "{value}"),
            Self::F64(value) => write!(f, "{value}"),
            Self::String(value) => write!(f, "{value}"),
            Self::Bytes(_) => write!(f, "<bytes>"),
            Self::List(_) => write!(f, "<list>"),
            Self::Map(_) => write!(f, "<map>"),
        }
    }
}

/// Protocol-neutral operation intent routed between adapters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Intent {
    /// Reads current state from named resource.
    Read { resource: String },
    /// Writes typed payload to named resource.
    Write {
        resource: String,
        payload: PayloadValue,
    },
    /// Invokes operation with optional argument payload.
    Invoke {
        operation: String,
        args: Option<PayloadValue>,
    },
    /// Sends typed payload to broker-oriented channel such as topic, subject, queue, or stream.
    Send {
        channel: BrokerAddress,
        payload: PayloadValue,
        options: BrokerMessageOptions,
    },
    /// Subscribes to broker-oriented channel such as topic, subject, queue, or stream.
    Subscribe {
        channel: BrokerAddress,
        options: BrokerSubscriptionOptions,
    },
    /// Removes prior subscription for channel.
    Unsubscribe { channel: BrokerAddress },
}

/// Represents a protocol-neutral command sent to a device or the core.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Command {
    /// Stable command identifier.
    pub id: CommandId,
    /// Optional origin device when command was emitted by another device.
    pub source_device_id: Option<DeviceId>,
    /// Destination device identifier.
    pub target_device_id: DeviceId,
    /// Requested protocol-neutral operation.
    pub intent: Intent,
    /// Optional async reply or completion correlation metadata.
    pub correlation: Option<Correlation>,
}

/// Represents the result of command execution or delivery.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandResult {
    /// Identifier of command this result belongs to.
    pub command_id: CommandId,
    /// Device that produced this result.
    pub device_id: DeviceId,
    /// Delivery or completion state.
    pub state: DeliveryState,
    /// Optional payload returned by successful completion.
    pub payload: Option<PayloadValue>,
    /// Optional human-readable error description.
    pub error: Option<String>,
    /// Optional correlation metadata preserved from originating command.
    pub correlation: Option<Correlation>,
}
