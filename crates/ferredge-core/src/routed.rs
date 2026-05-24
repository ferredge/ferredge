#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(feature = "std")]
use std::{string::String, vec::Vec};

#[cfg(not(feature = "std"))]
use alloc::{string::String, vec::Vec};

use serde::{Deserialize, Serialize};

use crate::{
    command::{Address, Command, CommandResult, Correlation, PayloadValue},
    device::{DeviceId, DeviceProtocol},
};

/// Identifies concrete source or target transport endpoint inside router graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndpointRef {
    /// Device identifier registered with router.
    pub device_id: DeviceId,
    /// Transport protocol used by device endpoint.
    pub protocol: DeviceProtocol,
}

/// Optional HTTP-specific metadata carried alongside routed messages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpMeta {
    /// HTTP method used for outbound or inbound message.
    pub method: Option<String>,
    /// HTTP path or resource slug associated with message.
    pub path: Option<String>,
    /// HTTP response status code when available.
    pub status_code: Option<u16>,
    /// Preserved HTTP headers associated with request or response.
    pub headers: Vec<(String, String)>,
}

/// Optional MQTT-specific metadata carried alongside routed messages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MqttMeta {
    /// Topic name associated with publish or inbound message.
    pub topic: String,
    /// MQTT quality-of-service level.
    pub qos: u8,
    /// Whether message was retained by broker.
    pub retain: bool,
    /// Whether message had MQTT duplicate delivery flag set.
    pub duplicate: bool,
    /// Packet identifier when protocol exchange exposed one.
    pub packet_id: Option<u16>,
    /// Optional content type declared on MQTT v5 publish.
    pub content_type: Option<String>,
    /// Optional MQTT v5 payload format indicator.
    pub payload_format: Option<String>,
    /// Optional MQTT v5 message expiry interval in seconds.
    pub message_expiry_interval_secs: Option<u32>,
    /// Optional response topic declared on MQTT v5 publish.
    pub response_topic: Option<String>,
    /// Optional correlation data preserved as UTF-8 lossless string view.
    pub correlation_data: Option<String>,
    /// Optional raw correlation data preserved losslessly.
    pub correlation_data_bytes: Option<Vec<u8>>,
    /// Optional MQTT v5 topic alias.
    pub topic_alias: Option<u16>,
    /// Optional subscription identifiers included by broker on MQTT v5 publish.
    pub subscription_identifiers: Vec<u32>,
    /// Optional MQTT v5 user properties preserved from packet metadata.
    pub user_properties: Vec<(String, String)>,
    /// Optional MQTT v5 reason code list for ack or disconnect packets.
    pub reason_codes: Vec<String>,
    /// Optional MQTT v5 human-readable reason string.
    pub reason_string: Option<String>,
}

/// Transport-specific message metadata preserved during routing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransportMeta {
    /// HTTP transport metadata.
    Http(HttpMeta),
    /// MQTT transport metadata.
    Mqtt(MqttMeta),
}

/// Normalized inbound event emitted by one protocol adapter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoutedEvent {
    /// Source endpoint that emitted this event.
    pub source: EndpointRef,
    /// Logical address targeted by event, such as resource or topic.
    pub address: Address,
    /// Typed event payload preserved across routing.
    pub payload: PayloadValue,
    /// Optional request correlation for reply-topic or async workflows.
    pub correlation: Option<Correlation>,
    /// Optional transport metadata preserved from source protocol.
    pub transport: Option<TransportMeta>,
}

/// Normalized command result or transport completion emitted by one adapter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoutedResult {
    /// Source endpoint that produced this result.
    pub source: EndpointRef,
    /// Protocol-neutral command result payload.
    pub result: CommandResult,
    /// Optional transport metadata preserved from source protocol.
    pub transport: Option<TransportMeta>,
}

/// Unified message envelope accepted by router and bridge layers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RoutedMessage {
    /// Routed command envelope.
    Command(Command),
    /// Routed event envelope.
    Event(RoutedEvent),
    /// Routed result envelope.
    Result(RoutedResult),
}
