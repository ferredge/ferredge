#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(feature = "std")]
use std::{string::String, vec::Vec};

#[cfg(not(feature = "std"))]
use alloc::{string::String, vec::Vec};

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

/// Protocol-neutral operation intent routed between adapters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Intent {
    /// Reads current state from named resource.
    Read {
        resource: String,
    },
    /// Writes payload bytes to named resource.
    Write {
        resource: String,
        payload: Vec<u8>,
    },
    /// Invokes operation with optional argument payload.
    Invoke {
        operation: String,
        args: Option<Vec<u8>>,
    },
    /// Sends payload to broker-oriented channel such as topic, subject, queue, or stream.
    Send {
        channel: BrokerAddress,
        payload: Vec<u8>,
        options: BrokerMessageOptions,
    },
    /// Subscribes to broker-oriented channel such as topic, subject, queue, or stream.
    Subscribe {
        channel: BrokerAddress,
        options: BrokerSubscriptionOptions,
    },
    /// Removes prior subscription for channel.
    Unsubscribe {
        channel: BrokerAddress,
    },
}

/// Represents a protocol-neutral command sent to a device or the core.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    pub payload: Option<Vec<u8>>,
    /// Optional human-readable error description.
    pub error: Option<String>,
    /// Optional correlation metadata preserved from originating command.
    pub correlation: Option<Correlation>,
}
