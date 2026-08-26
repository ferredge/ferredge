extern crate alloc;

use alloc::{borrow::Cow, string::String, vec::Vec};

use ferredge_core::prelude::{
    DeliveryGuarantee, Headers, IntoOwned, MetaValue, ProtocolId, ProtocolMeta, TransportMeta,
};
use serde::{Deserialize, Serialize};

use crate::types::MqttCommandConversionError;

/// Capture key for the MQTT topic name.
pub const KEY_TOPIC: &str = "topic";
/// Capture key for the MQTT QoS level; values above `u8::MAX` mark the capture malformed.
pub const KEY_QOS: &str = "qos";
/// Capture key for the MQTT retain flag.
pub const KEY_RETAIN: &str = "retain";
/// Capture key for the MQTT duplicate delivery flag.
pub const KEY_DUPLICATE: &str = "duplicate";
/// Capture key for the MQTT packet identifier; values above `u16::MAX` mark the capture malformed.
pub const KEY_PACKET_ID: &str = "packet-id";
/// Capture key for the MQTT v5 content type property.
pub const KEY_CONTENT_TYPE: &str = "content-type";
/// Capture key for the MQTT v5 payload format indicator; names other than `bytes`/`utf8` mark the capture malformed.
pub const KEY_PAYLOAD_FORMAT: &str = "payload-format";
/// Capture key for the MQTT v5 message expiry interval in seconds; values above `u32::MAX` mark the capture malformed.
pub const KEY_MESSAGE_EXPIRY_INTERVAL_SECS: &str = "message-expiry-interval-secs";
/// Capture key for the MQTT v5 response topic property.
pub const KEY_RESPONSE_TOPIC: &str = "response-topic";
/// Capture key for the MQTT v5 correlation data property.
pub const KEY_CORRELATION_DATA: &str = "correlation-data";
/// Capture key for the MQTT v5 topic alias property; values above `u16::MAX` mark the capture malformed.
pub const KEY_TOPIC_ALIAS: &str = "topic-alias";
/// Capture key for MQTT v5 subscription identifiers; values above `u32::MAX` mark the capture malformed.
pub const KEY_SUBSCRIPTION_IDENTIFIERS: &str = "subscription-identifiers";
/// Capture key for MQTT v5 reason codes rendered as text.
pub const KEY_REASON_CODES: &str = "reason-codes";
/// Capture key for the MQTT v5 reason string property.
pub const KEY_REASON_STRING: &str = "reason-string";
/// Capture key for a durable subscription name.
pub const KEY_DURABLE_NAME: &str = "durable-name";
/// Capture key for a shared subscription group.
pub const KEY_SHARED_GROUP: &str = "shared-group";
/// Capture key for the MQTT v5 no-local subscription flag.
pub const KEY_NO_LOCAL: &str = "no-local";
/// Capture key for the MQTT v5 retain-as-published subscription flag.
pub const KEY_RETAIN_AS_PUBLISHED: &str = "retain-as-published";
/// Capture key for the MQTT v5 retained-message handling policy; codes above 2 mark the capture malformed.
pub const KEY_RETAIN_HANDLING: &str = "retain-handling";

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

/// Maps a protocol-neutral delivery guarantee to the MQTT QoS byte.
pub fn qos_from_delivery(delivery: DeliveryGuarantee) -> u8 {
    match delivery {
        DeliveryGuarantee::BestEffort => 0,
        DeliveryGuarantee::AtLeastOnce => 1,
        DeliveryGuarantee::ExactlyOnce => 2,
    }
}

/// Renders the MQTT payload format indicator as its canonical text name.
pub fn mqtt_payload_format_name(value: &MqttPayloadFormat) -> Cow<'static, str> {
    match value {
        MqttPayloadFormat::Bytes => Cow::Borrowed("bytes"),
        MqttPayloadFormat::Utf8 => Cow::Borrowed("utf8"),
    }
}

/// Parses the canonical text name back into a payload format indicator.
pub fn mqtt_payload_format_from_name(value: &str) -> Option<MqttPayloadFormat> {
    match value {
        "bytes" => Some(MqttPayloadFormat::Bytes),
        "utf8" => Some(MqttPayloadFormat::Utf8),
        _ => None,
    }
}

/// Renders the retained-message handling policy as its wire code.
pub fn retain_handling_code(value: MqttRetainHandling) -> u64 {
    match value {
        MqttRetainHandling::SendRetained => 0,
        MqttRetainHandling::SendRetainedIfNotExists => 1,
        MqttRetainHandling::DoNotSendRetained => 2,
    }
}

/// Parses the wire code back into a retained-message handling policy.
pub fn retain_handling_from_code(value: u64) -> Option<MqttRetainHandling> {
    match value {
        0 => Some(MqttRetainHandling::SendRetained),
        1 => Some(MqttRetainHandling::SendRetainedIfNotExists),
        2 => Some(MqttRetainHandling::DoNotSendRetained),
        _ => None,
    }
}

/// MQTT-specific broker send options encoded into the transport capture.
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
    /// Optional MQTT v5 response topic.
    pub response_topic: Option<String>,
    /// Optional raw MQTT v5 correlation data.
    pub correlation_data: Option<Vec<u8>>,
}

impl MqttMessageOptions {
    /// Encodes these publish options as an open MQTT transport capture.
    pub fn into_transport_meta(self) -> TransportMeta<'static> {
        let mut meta = TransportMeta::new(ProtocolId::MQTT);
        if self.retain {
            meta.set(KEY_RETAIN, MetaValue::Bool(true));
        }
        if let Some(payload_format) = self.payload_format {
            meta.set(
                KEY_PAYLOAD_FORMAT,
                MetaValue::Str(mqtt_payload_format_name(&payload_format)),
            );
        }
        if let Some(content_type) = self.content_type {
            meta.set(KEY_CONTENT_TYPE, MetaValue::Str(content_type.into()));
        }
        if let Some(expiry) = self.message_expiry_interval_secs {
            meta.set(
                KEY_MESSAGE_EXPIRY_INTERVAL_SECS,
                MetaValue::U64(u64::from(expiry)),
            );
        }
        if let Some(topic_alias) = self.topic_alias {
            meta.set(KEY_TOPIC_ALIAS, MetaValue::U64(u64::from(topic_alias)));
        }
        if !self.user_properties.is_empty() {
            meta.headers = Some(Headers::from(self.user_properties));
        }
        if let Some(response_topic) = self.response_topic {
            meta.set(KEY_RESPONSE_TOPIC, MetaValue::Str(response_topic.into()));
        }
        if let Some(correlation_data) = self.correlation_data {
            meta.set(
                KEY_CORRELATION_DATA,
                MetaValue::Bytes(correlation_data.into()),
            );
        }
        meta
    }
}

/// MQTT-specific broker subscription options encoded into the transport capture.
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

impl MqttSubscriptionOptions {
    /// Encodes these subscription options as an open MQTT transport capture.
    pub fn into_transport_meta(self) -> TransportMeta<'static> {
        let mut meta = TransportMeta::new(ProtocolId::MQTT);
        if self.no_local {
            meta.set(KEY_NO_LOCAL, MetaValue::Bool(true));
        }
        if self.retain_as_published {
            meta.set(KEY_RETAIN_AS_PUBLISHED, MetaValue::Bool(true));
        }
        if let Some(retain_handling) = self.retain_handling {
            meta.set(
                KEY_RETAIN_HANDLING,
                MetaValue::U64(retain_handling_code(retain_handling)),
            );
        }
        if let Some(identifier) = self.subscription_identifier {
            meta.set(
                KEY_SUBSCRIPTION_IDENTIFIERS,
                MetaValue::U64List(alloc::vec![u64::from(identifier)]),
            );
        }
        if !self.user_properties.is_empty() {
            meta.headers = Some(Headers::from(self.user_properties));
        }
        meta
    }
}

/// Typed MQTT view over the open transport metadata capture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MqttMeta<'a> {
    /// Topic name associated with publish or inbound message.
    pub topic: Option<Cow<'a, str>>,
    /// MQTT quality-of-service level.
    pub qos: Option<u8>,
    /// Whether message was or should be retained by broker.
    pub retain: bool,
    /// Whether message had MQTT duplicate delivery flag set.
    pub duplicate: bool,
    /// Packet identifier when protocol exchange exposed one.
    pub packet_id: Option<u16>,
    /// Optional MQTT v5 content type.
    pub content_type: Option<Cow<'a, str>>,
    /// Optional MQTT v5 payload format indicator.
    pub payload_format: Option<MqttPayloadFormat>,
    /// Optional MQTT v5 message expiry interval in seconds.
    pub message_expiry_interval_secs: Option<u32>,
    /// Optional MQTT v5 response topic.
    pub response_topic: Option<Cow<'a, str>>,
    /// Optional raw MQTT v5 correlation data.
    pub correlation_data: Option<Cow<'a, [u8]>>,
    /// Optional MQTT v5 topic alias.
    pub topic_alias: Option<u16>,
    /// Subscription identifiers included by broker on MQTT v5 packets.
    pub subscription_identifiers: Vec<u32>,
    /// MQTT v5 reason codes preserved as text.
    pub reason_codes: Vec<Cow<'a, str>>,
    /// Optional MQTT v5 human-readable reason string.
    pub reason_string: Option<Cow<'a, str>>,
    /// Optional durable subscription name.
    pub durable_name: Option<Cow<'a, str>>,
    /// Optional shared subscription group.
    pub shared_group: Option<Cow<'a, str>>,
    /// MQTT v5 no-local subscription flag.
    pub no_local: bool,
    /// MQTT v5 retain-as-published subscription flag.
    pub retain_as_published: bool,
    /// Optional MQTT v5 retained-message handling policy.
    pub retain_handling: Option<MqttRetainHandling>,
    /// Optional MQTT v5 user properties carried as first-class capture headers.
    pub headers: Option<Headers<'a>>,
}

impl<'a> MqttMeta<'a> {
    /// Decodes an MQTT capture by move, rejecting foreign protocols, malformed entries, and wrong-typed known keys; duplicate keys decode first-occurrence-wins.
    pub fn try_from_transport_meta(
        mut meta: TransportMeta<'a>,
    ) -> Result<Self, MqttCommandConversionError> {
        if meta.protocol != ProtocolId::MQTT {
            return Err(MqttCommandConversionError::ForeignExtensions(meta.protocol));
        }
        let mut view = MqttMeta {
            headers: meta.headers.take(),
            ..MqttMeta::default()
        };
        let mut seen: Vec<Cow<'static, str>> = Vec::new();
        for (key, value) in meta.into_entries() {
            if seen.iter().any(|entry| entry == &key) {
                continue;
            }
            seen.push(key.clone());
            match key.as_ref() {
                KEY_TOPIC => view.topic = Some(expect_str(KEY_TOPIC, value)?),
                KEY_QOS => {
                    view.qos = Some(checked_capture_value(KEY_QOS, expect_u64(KEY_QOS, value)?)?);
                }
                KEY_RETAIN => view.retain = expect_bool(KEY_RETAIN, value)?,
                KEY_DUPLICATE => view.duplicate = expect_bool(KEY_DUPLICATE, value)?,
                KEY_PACKET_ID => {
                    view.packet_id = Some(checked_capture_value(
                        KEY_PACKET_ID,
                        expect_u64(KEY_PACKET_ID, value)?,
                    )?);
                }
                KEY_CONTENT_TYPE => view.content_type = Some(expect_str(KEY_CONTENT_TYPE, value)?),
                KEY_PAYLOAD_FORMAT => {
                    let value = expect_str(KEY_PAYLOAD_FORMAT, value)?;
                    view.payload_format = Some(
                        mqtt_payload_format_from_name(value.as_ref()).ok_or_else(|| {
                            MqttCommandConversionError::InvalidExtensionName {
                                key: KEY_PAYLOAD_FORMAT,
                                value: value.into_owned(),
                            }
                        })?,
                    );
                }
                KEY_MESSAGE_EXPIRY_INTERVAL_SECS => {
                    view.message_expiry_interval_secs = Some(checked_capture_value(
                        KEY_MESSAGE_EXPIRY_INTERVAL_SECS,
                        expect_u64(KEY_MESSAGE_EXPIRY_INTERVAL_SECS, value)?,
                    )?);
                }
                KEY_RESPONSE_TOPIC => {
                    view.response_topic = Some(expect_str(KEY_RESPONSE_TOPIC, value)?);
                }
                KEY_CORRELATION_DATA => {
                    view.correlation_data = Some(expect_bytes(KEY_CORRELATION_DATA, value)?);
                }
                KEY_TOPIC_ALIAS => {
                    view.topic_alias = Some(checked_capture_value(
                        KEY_TOPIC_ALIAS,
                        expect_u64(KEY_TOPIC_ALIAS, value)?,
                    )?);
                }
                KEY_SUBSCRIPTION_IDENTIFIERS => {
                    view.subscription_identifiers =
                        expect_u64_list(KEY_SUBSCRIPTION_IDENTIFIERS, value)?
                            .into_iter()
                            .map(|value| checked_capture_value(KEY_SUBSCRIPTION_IDENTIFIERS, value))
                            .collect::<Result<_, _>>()?;
                }
                KEY_REASON_CODES => {
                    view.reason_codes = expect_str_list(KEY_REASON_CODES, value)?;
                }
                KEY_REASON_STRING => {
                    view.reason_string = Some(expect_str(KEY_REASON_STRING, value)?);
                }
                KEY_DURABLE_NAME => view.durable_name = Some(expect_str(KEY_DURABLE_NAME, value)?),
                KEY_SHARED_GROUP => view.shared_group = Some(expect_str(KEY_SHARED_GROUP, value)?),
                KEY_NO_LOCAL => view.no_local = expect_bool(KEY_NO_LOCAL, value)?,
                KEY_RETAIN_AS_PUBLISHED => {
                    view.retain_as_published = expect_bool(KEY_RETAIN_AS_PUBLISHED, value)?;
                }
                KEY_RETAIN_HANDLING => {
                    let value = expect_u64(KEY_RETAIN_HANDLING, value)?;
                    view.retain_handling = Some(retain_handling_from_code(value).ok_or(
                        MqttCommandConversionError::InvalidExtensionValue {
                            key: KEY_RETAIN_HANDLING,
                            value,
                        },
                    )?);
                }
                _ => {}
            }
        }
        Ok(view)
    }
}

/// Requires one capture value to be UTF-8 text.
fn expect_str<'a>(
    key: &'static str,
    value: MetaValue<'a>,
) -> Result<Cow<'a, str>, MqttCommandConversionError> {
    match value {
        MetaValue::Str(value) => Ok(value),
        _ => Err(MqttCommandConversionError::InvalidExtensionType { key }),
    }
}

/// Requires one capture value to be an unsigned integer.
fn expect_u64(key: &'static str, value: MetaValue<'_>) -> Result<u64, MqttCommandConversionError> {
    match value {
        MetaValue::U64(value) => Ok(value),
        _ => Err(MqttCommandConversionError::InvalidExtensionType { key }),
    }
}

/// Requires one capture value to be a boolean flag.
fn expect_bool(
    key: &'static str,
    value: MetaValue<'_>,
) -> Result<bool, MqttCommandConversionError> {
    match value {
        MetaValue::Bool(value) => Ok(value),
        _ => Err(MqttCommandConversionError::InvalidExtensionType { key }),
    }
}

/// Requires one capture value to be opaque bytes.
fn expect_bytes<'a>(
    key: &'static str,
    value: MetaValue<'a>,
) -> Result<Cow<'a, [u8]>, MqttCommandConversionError> {
    match value {
        MetaValue::Bytes(value) => Ok(value),
        _ => Err(MqttCommandConversionError::InvalidExtensionType { key }),
    }
}

/// Requires one capture value to be an unsigned integer list.
fn expect_u64_list(
    key: &'static str,
    value: MetaValue<'_>,
) -> Result<Vec<u64>, MqttCommandConversionError> {
    match value {
        MetaValue::U64List(values) => Ok(values),
        _ => Err(MqttCommandConversionError::InvalidExtensionType { key }),
    }
}

/// Requires one capture value to be a text list.
fn expect_str_list<'a>(
    key: &'static str,
    value: MetaValue<'a>,
) -> Result<Vec<Cow<'a, str>>, MqttCommandConversionError> {
    match value {
        MetaValue::StrList(values) => Ok(values),
        _ => Err(MqttCommandConversionError::InvalidExtensionType { key }),
    }
}

/// Range-checks one capture integer against its MQTT wire field width.
pub(crate) fn checked_capture_value<T: TryFrom<u64>>(
    key: &'static str,
    value: u64,
) -> Result<T, MqttCommandConversionError> {
    T::try_from(value).map_err(|_| MqttCommandConversionError::InvalidExtensionValue { key, value })
}

impl<'a> ProtocolMeta<'a> for MqttMeta<'a> {
    const PROTOCOL: ProtocolId = ProtocolId::MQTT;

    /// Encodes this typed view as an open MQTT transport capture without copying values.
    fn into_transport_meta(self) -> TransportMeta<'a> {
        let mut meta = TransportMeta::new(ProtocolId::MQTT);
        if let Some(topic) = self.topic {
            meta.set(KEY_TOPIC, MetaValue::Str(topic));
        }
        if let Some(qos) = self.qos {
            meta.set(KEY_QOS, MetaValue::U64(u64::from(qos)));
        }
        if self.retain {
            meta.set(KEY_RETAIN, MetaValue::Bool(true));
        }
        if self.duplicate {
            meta.set(KEY_DUPLICATE, MetaValue::Bool(true));
        }
        if let Some(packet_id) = self.packet_id {
            meta.set(KEY_PACKET_ID, MetaValue::U64(u64::from(packet_id)));
        }
        if let Some(content_type) = self.content_type {
            meta.set(KEY_CONTENT_TYPE, MetaValue::Str(content_type));
        }
        if let Some(payload_format) = self.payload_format {
            meta.set(
                KEY_PAYLOAD_FORMAT,
                MetaValue::Str(mqtt_payload_format_name(&payload_format)),
            );
        }
        if let Some(expiry) = self.message_expiry_interval_secs {
            meta.set(
                KEY_MESSAGE_EXPIRY_INTERVAL_SECS,
                MetaValue::U64(u64::from(expiry)),
            );
        }
        if let Some(response_topic) = self.response_topic {
            meta.set(KEY_RESPONSE_TOPIC, MetaValue::Str(response_topic));
        }
        if let Some(correlation_data) = self.correlation_data {
            meta.set(KEY_CORRELATION_DATA, MetaValue::Bytes(correlation_data));
        }
        if let Some(topic_alias) = self.topic_alias {
            meta.set(KEY_TOPIC_ALIAS, MetaValue::U64(u64::from(topic_alias)));
        }
        if !self.subscription_identifiers.is_empty() {
            meta.set(
                KEY_SUBSCRIPTION_IDENTIFIERS,
                MetaValue::U64List(
                    self.subscription_identifiers
                        .into_iter()
                        .map(u64::from)
                        .collect(),
                ),
            );
        }
        if !self.reason_codes.is_empty() {
            meta.set(KEY_REASON_CODES, MetaValue::StrList(self.reason_codes));
        }
        if let Some(reason_string) = self.reason_string {
            meta.set(KEY_REASON_STRING, MetaValue::Str(reason_string));
        }
        if let Some(durable_name) = self.durable_name {
            meta.set(KEY_DURABLE_NAME, MetaValue::Str(durable_name));
        }
        if let Some(shared_group) = self.shared_group {
            meta.set(KEY_SHARED_GROUP, MetaValue::Str(shared_group));
        }
        if self.no_local {
            meta.set(KEY_NO_LOCAL, MetaValue::Bool(true));
        }
        if self.retain_as_published {
            meta.set(KEY_RETAIN_AS_PUBLISHED, MetaValue::Bool(true));
        }
        if let Some(retain_handling) = self.retain_handling {
            meta.set(
                KEY_RETAIN_HANDLING,
                MetaValue::U64(retain_handling_code(retain_handling)),
            );
        }
        meta.headers = self.headers;
        meta
    }

    /// Decodes an MQTT transport capture into a typed view by move; foreign or malformed captures yield `None`.
    fn from_transport_meta(meta: TransportMeta<'a>) -> Option<Self> {
        Self::try_from_transport_meta(meta).ok()
    }

    /// Borrows an MQTT transport capture as a typed view; foreign or malformed captures yield `None`.
    fn view(meta: &'a TransportMeta<'a>) -> Option<MqttMeta<'a>> {
        Self::try_from_transport_meta(meta.as_borrowed()).ok()
    }
}

impl MqttMeta<'_> {
    /// Materializes an owned typed view for long-lived boundaries.
    pub fn into_owned(self) -> MqttMeta<'static> {
        IntoOwned::into_owned(self)
    }
}

impl IntoOwned for MqttMeta<'_> {
    type Owned = MqttMeta<'static>;
    fn into_owned(self) -> Self::Owned {
        MqttMeta {
            topic: self.topic.into_owned(),
            qos: self.qos,
            retain: self.retain,
            duplicate: self.duplicate,
            packet_id: self.packet_id,
            content_type: self.content_type.into_owned(),
            payload_format: self.payload_format,
            message_expiry_interval_secs: self.message_expiry_interval_secs,
            response_topic: self.response_topic.into_owned(),
            correlation_data: self.correlation_data.into_owned(),
            topic_alias: self.topic_alias,
            subscription_identifiers: self.subscription_identifiers,
            reason_codes: IntoOwned::into_owned(self.reason_codes),
            reason_string: self.reason_string.into_owned(),
            durable_name: self.durable_name.into_owned(),
            shared_group: self.shared_group.into_owned(),
            no_local: self.no_local,
            retain_as_published: self.retain_as_published,
            retain_handling: self.retain_handling,
            headers: self.headers.into_owned(),
        }
    }
}
