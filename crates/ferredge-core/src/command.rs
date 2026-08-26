extern crate alloc;

use alloc::{borrow::Cow, string::String, vec::Vec};
use core::fmt;

use crate::device::DeviceId;
use crate::headers::Headers;
use crate::meta::TransportMeta;
use crate::owned::IntoOwned;
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
pub struct Correlation<'a> {
    /// Identifier of original command or request.
    pub request_id: Cow<'a, str>,
    /// Optional logical reply destination such as topic or resource.
    pub reply_to: Option<Address<'a>>,
}

impl Correlation<'_> {
    /// Re-borrows this correlation without forcing ownership changes.
    pub fn as_borrowed(&self) -> Correlation<'_> {
        Correlation {
            request_id: Cow::Borrowed(self.request_id.as_ref()),
            reply_to: self.reply_to.as_ref().map(Address::as_borrowed),
        }
    }

    /// Materializes an owned correlation for async or persistent boundaries.
    pub fn into_owned(self) -> Correlation<'static> {
        IntoOwned::into_owned(self)
    }
}

impl IntoOwned for Correlation<'_> {
    type Owned = Correlation<'static>;
    fn into_owned(self) -> Self::Owned {
        Correlation {
            request_id: IntoOwned::into_owned(self.request_id),
            reply_to: self.reply_to.into_owned(),
        }
    }
}

/// Protocol-neutral logical address used by routed messages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Address<'a> {
    /// Resource-oriented address such as HTTP path or Modbus register alias.
    Resource(Cow<'a, str>),
    /// Channel-oriented address such as MQTT or NATS topic.
    Channel(Cow<'a, str>),
}

impl Address<'_> {
    /// Re-borrows this address without forcing ownership changes.
    pub fn as_borrowed(&self) -> Address<'_> {
        match self {
            Self::Resource(value) => Address::Resource(Cow::Borrowed(value.as_ref())),
            Self::Channel(value) => Address::Channel(Cow::Borrowed(value.as_ref())),
        }
    }

    /// Materializes an owned address for long-lived boundaries.
    pub fn into_owned(self) -> Address<'static> {
        IntoOwned::into_owned(self)
    }

    /// Returns the underlying string slice for either address family.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Resource(value) | Self::Channel(value) => value.as_ref(),
        }
    }
}

impl IntoOwned for Address<'_> {
    type Owned = Address<'static>;
    fn into_owned(self) -> Self::Owned {
        match self {
            Self::Resource(value) => Address::Resource(IntoOwned::into_owned(value)),
            Self::Channel(value) => Address::Channel(IntoOwned::into_owned(value)),
        }
    }
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

/// Protocol-neutral broker send options preserved in routed command layer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct BrokerMessageOptions {
    /// Optional delivery guarantee requested by caller.
    pub delivery: Option<DeliveryGuarantee>,
    /// Optional message headers or attributes.
    pub headers: Headers<'static>,
    /// Optional open protocol-specific metadata capture.
    pub extensions: Option<TransportMeta<'static>>,
}

/// Protocol-neutral request options preserved for request/response transports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RequestOptions {
    /// Optional dynamic headers attached to the request.
    pub headers: Headers<'static>,
    /// Optional declared content type.
    pub content_type: Option<String>,
    /// Optional method override for transports that support it.
    pub method: Option<String>,
    /// Optional path override for transports that support it.
    pub path: Option<String>,
    /// Optional open protocol-specific metadata capture.
    pub extensions: Option<TransportMeta<'static>>,
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
    /// Optional open protocol-specific metadata capture.
    pub extensions: Option<TransportMeta<'static>>,
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
pub enum PayloadValue<'a> {
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
    String(Cow<'a, str>),
    /// Opaque binary payload.
    Bytes(Cow<'a, [u8]>),
    /// Ordered heterogeneous sequence payload.
    List(Vec<PayloadValue<'a>>),
    /// Ordered object payload.
    Map(Vec<(Cow<'a, str>, PayloadValue<'a>)>),
}

impl PayloadValue<'_> {
    /// Re-borrows this payload without forcing ownership changes.
    pub fn as_borrowed(&self) -> PayloadValue<'_> {
        match self {
            Self::Null => PayloadValue::Null,
            Self::Bool(value) => PayloadValue::Bool(*value),
            Self::I64(value) => PayloadValue::I64(*value),
            Self::U64(value) => PayloadValue::U64(*value),
            Self::F64(value) => PayloadValue::F64(*value),
            Self::String(value) => PayloadValue::String(Cow::Borrowed(value.as_ref())),
            Self::Bytes(value) => PayloadValue::Bytes(Cow::Borrowed(value.as_ref())),
            Self::List(values) => {
                PayloadValue::List(values.iter().map(PayloadValue::as_borrowed).collect())
            }
            Self::Map(values) => PayloadValue::Map(
                values
                    .iter()
                    .map(|(key, value)| (Cow::Borrowed(key.as_ref()), value.as_borrowed()))
                    .collect(),
            ),
        }
    }

    /// Materializes an owned payload for async or persistence boundaries.
    pub fn into_owned(self) -> PayloadValue<'static> {
        IntoOwned::into_owned(self)
    }
}

impl IntoOwned for PayloadValue<'_> {
    type Owned = PayloadValue<'static>;
    fn into_owned(self) -> Self::Owned {
        match self {
            Self::Null => PayloadValue::Null,
            Self::Bool(value) => PayloadValue::Bool(value),
            Self::I64(value) => PayloadValue::I64(value),
            Self::U64(value) => PayloadValue::U64(value),
            Self::F64(value) => PayloadValue::F64(value),
            Self::String(value) => PayloadValue::String(IntoOwned::into_owned(value)),
            Self::Bytes(value) => PayloadValue::Bytes(IntoOwned::into_owned(value)),
            Self::List(values) => PayloadValue::List(IntoOwned::into_owned(values)),
            Self::Map(values) => PayloadValue::Map(IntoOwned::into_owned(values)),
        }
    }
}

impl From<Vec<u8>> for PayloadValue<'static> {
    fn from(value: Vec<u8>) -> Self {
        Self::Bytes(Cow::Owned(value))
    }
}

impl<'a> From<&'a [u8]> for PayloadValue<'a> {
    fn from(value: &'a [u8]) -> Self {
        Self::Bytes(Cow::Borrowed(value))
    }
}

impl From<String> for PayloadValue<'static> {
    fn from(value: String) -> Self {
        Self::String(Cow::Owned(value))
    }
}

impl<'a> From<&'a str> for PayloadValue<'a> {
    fn from(value: &'a str) -> Self {
        Self::String(Cow::Borrowed(value))
    }
}

impl fmt::Display for PayloadValue<'_> {
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
    Read {
        resource: String,
        options: RequestOptions,
    },
    /// Writes typed payload to named resource.
    Write {
        resource: String,
        payload: PayloadValue<'static>,
        options: RequestOptions,
    },
    /// Invokes operation with optional argument payload.
    Invoke {
        operation: String,
        args: Option<PayloadValue<'static>>,
        options: RequestOptions,
    },
    /// Sends typed payload to broker-oriented channel such as topic, subject, queue, or stream.
    Send {
        channel: BrokerAddress,
        payload: PayloadValue<'static>,
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
    pub correlation: Option<Correlation<'static>>,
}

/// Represents the result of command execution or delivery.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandResult<'a> {
    /// Identifier of command this result belongs to.
    pub command_id: CommandId,
    /// Device that produced this result.
    pub device_id: DeviceId,
    /// Delivery or completion state.
    pub state: DeliveryState,
    /// Optional payload returned by successful completion.
    pub payload: Option<PayloadValue<'a>>,
    /// Optional human-readable error description.
    pub error: Option<Cow<'a, str>>,
    /// Optional correlation metadata preserved from originating command.
    pub correlation: Option<Correlation<'a>>,
}

impl CommandResult<'_> {
    /// Materializes an owned command result for long-lived boundaries.
    pub fn into_owned(self) -> CommandResult<'static> {
        IntoOwned::into_owned(self)
    }
}

impl IntoOwned for CommandResult<'_> {
    type Owned = CommandResult<'static>;
    fn into_owned(self) -> Self::Owned {
        CommandResult {
            command_id: self.command_id,
            device_id: self.device_id,
            state: self.state,
            payload: self.payload.into_owned(),
            error: self.error.into_owned(),
            correlation: self.correlation.into_owned(),
        }
    }
}
