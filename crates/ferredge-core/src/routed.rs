#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(feature = "std")]
use alloc::borrow::Cow;

#[cfg(not(feature = "std"))]
use alloc::borrow::Cow;

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
pub struct HttpMeta<'a> {
    /// HTTP method used for outbound or inbound message.
    pub method: Option<Cow<'a, str>>,
    /// HTTP path or resource slug associated with message.
    pub path: Option<Cow<'a, str>>,
    /// HTTP response status code when available.
    pub status_code: Option<u16>,
    /// Preserved HTTP headers associated with request or response.
    pub headers: Cow<'a, [(Cow<'a, str>, Cow<'a, str>)]>,
}

impl HttpMeta<'_> {
    /// Materializes an owned HTTP metadata snapshot.
    pub fn into_owned(self) -> HttpMeta<'static> {
        HttpMeta {
            method: self.method.map(|value| Cow::Owned(value.into_owned())),
            path: self.path.map(|value| Cow::Owned(value.into_owned())),
            status_code: self.status_code,
            headers: Cow::Owned(
                self.headers
                    .into_owned()
                    .into_iter()
                    .map(|(key, value)| {
                        (Cow::Owned(key.into_owned()), Cow::Owned(value.into_owned()))
                    })
                    .collect(),
            ),
        }
    }
}

/// Optional MQTT-specific metadata carried alongside routed messages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MqttMeta<'a> {
    /// Topic name associated with publish or inbound message.
    pub topic: Cow<'a, str>,
    /// MQTT quality-of-service level.
    pub qos: u8,
    /// Whether message was retained by broker.
    pub retain: bool,
    /// Whether message had MQTT duplicate delivery flag set.
    pub duplicate: bool,
    /// Packet identifier when protocol exchange exposed one.
    pub packet_id: Option<u16>,
    /// Optional content type declared on MQTT v5 publish.
    pub content_type: Option<Cow<'a, str>>,
    /// Optional MQTT v5 payload format indicator.
    pub payload_format: Option<Cow<'a, str>>,
    /// Optional MQTT v5 message expiry interval in seconds.
    pub message_expiry_interval_secs: Option<u32>,
    /// Optional response topic declared on MQTT v5 publish.
    pub response_topic: Option<Cow<'a, str>>,
    /// Optional correlation data preserved as UTF-8 lossless string view.
    pub correlation_data: Option<Cow<'a, str>>,
    /// Optional raw correlation data preserved losslessly.
    pub correlation_data_bytes: Option<Cow<'a, [u8]>>,
    /// Optional MQTT v5 topic alias.
    pub topic_alias: Option<u16>,
    /// Optional subscription identifiers included by broker on MQTT v5 publish.
    pub subscription_identifiers: Cow<'a, [u32]>,
    /// Optional MQTT v5 user properties preserved from packet metadata.
    pub user_properties: Cow<'a, [(Cow<'a, str>, Cow<'a, str>)]>,
    /// Optional MQTT v5 reason code list for ack or disconnect packets.
    pub reason_codes: Cow<'a, [Cow<'a, str>]>,
    /// Optional MQTT v5 human-readable reason string.
    pub reason_string: Option<Cow<'a, str>>,
}

impl MqttMeta<'_> {
    /// Materializes an owned MQTT metadata snapshot.
    pub fn into_owned(self) -> MqttMeta<'static> {
        MqttMeta {
            topic: Cow::Owned(self.topic.into_owned()),
            qos: self.qos,
            retain: self.retain,
            duplicate: self.duplicate,
            packet_id: self.packet_id,
            content_type: self
                .content_type
                .map(|value| Cow::Owned(value.into_owned())),
            payload_format: self
                .payload_format
                .map(|value| Cow::Owned(value.into_owned())),
            message_expiry_interval_secs: self.message_expiry_interval_secs,
            response_topic: self
                .response_topic
                .map(|value| Cow::Owned(value.into_owned())),
            correlation_data: self
                .correlation_data
                .map(|value| Cow::Owned(value.into_owned())),
            correlation_data_bytes: self
                .correlation_data_bytes
                .map(|value| Cow::Owned(value.into_owned())),
            topic_alias: self.topic_alias,
            subscription_identifiers: Cow::Owned(self.subscription_identifiers.into_owned()),
            user_properties: Cow::Owned(
                self.user_properties
                    .into_owned()
                    .into_iter()
                    .map(|(key, value)| {
                        (Cow::Owned(key.into_owned()), Cow::Owned(value.into_owned()))
                    })
                    .collect(),
            ),
            reason_codes: Cow::Owned(
                self.reason_codes
                    .into_owned()
                    .into_iter()
                    .map(|value| Cow::Owned(value.into_owned()))
                    .collect(),
            ),
            reason_string: self
                .reason_string
                .map(|value| Cow::Owned(value.into_owned())),
        }
    }
}

/// Transport-specific message metadata preserved during routing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransportMeta<'a> {
    /// HTTP transport metadata.
    Http(HttpMeta<'a>),
    /// MQTT transport metadata.
    Mqtt(MqttMeta<'a>),
}

impl TransportMeta<'_> {
    /// Materializes an owned transport metadata snapshot.
    pub fn into_owned(self) -> TransportMeta<'static> {
        match self {
            Self::Http(meta) => TransportMeta::Http(meta.into_owned()),
            Self::Mqtt(meta) => TransportMeta::Mqtt(meta.into_owned()),
        }
    }
}

/// Normalized inbound event emitted by one protocol adapter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoutedEvent<'a> {
    /// Source endpoint that emitted this event.
    pub source: EndpointRef,
    /// Logical address targeted by event, such as resource or topic.
    pub address: Address<'a>,
    /// Typed event payload preserved across routing.
    pub payload: PayloadValue<'a>,
    /// Optional request correlation for reply-topic or async workflows.
    pub correlation: Option<Correlation<'a>>,
    /// Optional transport metadata preserved from source protocol.
    pub transport: Option<TransportMeta<'a>>,
}

impl RoutedEvent<'_> {
    /// Materializes an owned routed event for queues or persistence.
    pub fn into_owned(self) -> RoutedEvent<'static> {
        RoutedEvent {
            source: self.source,
            address: self.address.into_owned(),
            payload: self.payload.into_owned(),
            correlation: self.correlation.map(Correlation::into_owned),
            transport: self.transport.map(TransportMeta::into_owned),
        }
    }
}

/// Normalized command result or transport completion emitted by one adapter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoutedResult<'a> {
    /// Source endpoint that produced this result.
    pub source: EndpointRef,
    /// Protocol-neutral command result payload.
    pub result: CommandResult<'a>,
    /// Optional transport metadata preserved from source protocol.
    pub transport: Option<TransportMeta<'a>>,
}

impl RoutedResult<'_> {
    /// Materializes an owned routed result for queues or persistence.
    pub fn into_owned(self) -> RoutedResult<'static> {
        RoutedResult {
            source: self.source,
            result: self.result.into_owned(),
            transport: self.transport.map(TransportMeta::into_owned),
        }
    }
}

/// Unified message envelope accepted by router and bridge layers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RoutedMessage<'a> {
    /// Routed command envelope.
    Command(Command),
    /// Routed event envelope.
    Event(RoutedEvent<'a>),
    /// Routed result envelope.
    Result(RoutedResult<'a>),
}

impl RoutedMessage<'_> {
    /// Materializes an owned routed message for async boundaries.
    pub fn into_owned(self) -> RoutedMessage<'static> {
        match self {
            Self::Command(command) => RoutedMessage::Command(command),
            Self::Event(event) => RoutedMessage::Event(event.into_owned()),
            Self::Result(result) => RoutedMessage::Result(result.into_owned()),
        }
    }
}
