use alloc::{borrow::Cow, vec::Vec};

use ferredge_core::prelude::{IntoOwned, MqttRetainHandling};
use serde::{Deserialize, Serialize};

/// Typed addressed-access metadata used by addressed field/register planners and codecs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddressedAccessMeta {
    /// Protocol-defined base address or offset.
    pub address: u32,
    /// Protocol-defined address-space or domain identifier.
    pub domain: Cow<'static, str>,
    /// Optional span or quantity in addressable units.
    pub quantity: Option<u16>,
}

/// Lossless logical route carried by the bridge layer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BridgeRoute<'a> {
    /// Resource-oriented route such as HTTP request/response.
    RequestResponse {
        /// Logical resource name targeted by the command or result.
        resource: Cow<'a, str>,
        /// Optional concrete transport path preserved alongside the logical resource.
        path: Option<Cow<'a, str>>,
    },
    /// Messaging-oriented route such as MQTT topic.
    Messaging {
        /// Topic or channel name used by the messaging transport.
        topic: Cow<'a, str>,
    },
    /// Addressed-access route such as a register, fieldbus offset, or memory-mapped point.
    AddressedAccess {
        /// Logical resource name associated with the addressed access.
        resource: Cow<'a, str>,
        /// Typed addressed-access metadata used by addressed-access codecs.
        access: AddressedAccessMeta,
        /// Optional protocol-scoped node identifier such as a Modbus unit id.
        node_id: Option<u32>,
    },
}

/// Typed HTTP metadata preserved separately from arbitrary headers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct HttpBridgeMeta<'a> {
    /// HTTP method associated with the request or response context.
    pub method: Option<Cow<'a, str>>,
    /// HTTP path associated with the request or response context.
    pub path: Option<Cow<'a, str>>,
    /// HTTP response status code when available.
    pub status_code: Option<u16>,
    /// Declared HTTP content type when available.
    pub content_type: Option<Cow<'a, str>>,
}

/// Typed MQTT metadata preserved separately from arbitrary properties.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MqttBridgeMeta<'a> {
    /// MQTT QoS level when available.
    pub qos: Option<u8>,
    /// Whether the publish should be or was retained.
    pub retain: bool,
    /// Whether the duplicate delivery flag was set.
    pub duplicate: bool,
    /// Packet identifier when exposed by the transport.
    pub packet_id: Option<u16>,
    /// MQTT content type property.
    pub content_type: Option<Cow<'a, str>>,
    /// MQTT payload format indicator rendered as text.
    pub payload_format: Option<Cow<'a, str>>,
    /// MQTT message expiry interval in seconds.
    pub message_expiry_interval_secs: Option<u32>,
    /// MQTT response topic property.
    pub response_topic: Option<Cow<'a, str>>,
    /// UTF-8 correlation data view when available.
    pub correlation_data: Option<Cow<'a, str>>,
    /// Raw correlation data preserved losslessly.
    pub correlation_data_bytes: Option<Vec<u8>>,
    /// MQTT topic alias value.
    pub topic_alias: Option<u16>,
    /// MQTT subscription identifiers reported on inbound packets.
    pub subscription_identifiers: Vec<u32>,
    /// MQTT reason codes preserved as text.
    pub reason_codes: Vec<Cow<'a, str>>,
    /// MQTT reason string property.
    pub reason_string: Option<Cow<'a, str>>,
    /// Durable subscription name when relevant.
    pub durable_name: Option<Cow<'a, str>>,
    /// Shared subscription group when relevant.
    pub shared_group: Option<Cow<'a, str>>,
    /// MQTT no-local subscription flag.
    pub no_local: bool,
    /// MQTT retain-as-published subscription flag.
    pub retain_as_published: bool,
    /// MQTT retained-message handling policy.
    pub retain_handling: Option<MqttRetainHandling>,
}

/// Typed transport metadata preserved by the bridge layer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BridgeTransportMeta<'a> {
    /// HTTP typed transport metadata.
    Http(HttpBridgeMeta<'a>),
    /// MQTT typed transport metadata.
    Mqtt(MqttBridgeMeta<'a>),
}

impl BridgeRoute<'_> {
    /// Materializes an owned bridge route for long-lived boundaries.
    pub fn into_owned(self) -> BridgeRoute<'static> {
        IntoOwned::into_owned(self)
    }
}

impl IntoOwned for BridgeRoute<'_> {
    type Owned = BridgeRoute<'static>;
    fn into_owned(self) -> Self::Owned {
        match self {
            BridgeRoute::RequestResponse { resource, path } => BridgeRoute::RequestResponse {
                resource: IntoOwned::into_owned(resource),
                path: path.into_owned(),
            },
            BridgeRoute::Messaging { topic } => BridgeRoute::Messaging {
                topic: IntoOwned::into_owned(topic),
            },
            BridgeRoute::AddressedAccess {
                resource,
                access,
                node_id,
            } => BridgeRoute::AddressedAccess {
                resource: IntoOwned::into_owned(resource),
                access,
                node_id,
            },
        }
    }
}

impl HttpBridgeMeta<'_> {
    /// Materializes owned HTTP bridge metadata for long-lived boundaries.
    pub fn into_owned(self) -> HttpBridgeMeta<'static> {
        IntoOwned::into_owned(self)
    }
}

impl IntoOwned for HttpBridgeMeta<'_> {
    type Owned = HttpBridgeMeta<'static>;
    fn into_owned(self) -> Self::Owned {
        HttpBridgeMeta {
            method: self.method.into_owned(),
            path: self.path.into_owned(),
            status_code: self.status_code,
            content_type: self.content_type.into_owned(),
        }
    }
}

impl MqttBridgeMeta<'_> {
    /// Materializes owned MQTT bridge metadata for long-lived boundaries.
    pub fn into_owned(self) -> MqttBridgeMeta<'static> {
        IntoOwned::into_owned(self)
    }
}

impl IntoOwned for MqttBridgeMeta<'_> {
    type Owned = MqttBridgeMeta<'static>;
    fn into_owned(self) -> Self::Owned {
        MqttBridgeMeta {
            qos: self.qos,
            retain: self.retain,
            duplicate: self.duplicate,
            packet_id: self.packet_id,
            content_type: self.content_type.into_owned(),
            payload_format: self.payload_format.into_owned(),
            message_expiry_interval_secs: self.message_expiry_interval_secs,
            response_topic: self.response_topic.into_owned(),
            correlation_data: self.correlation_data.into_owned(),
            correlation_data_bytes: self.correlation_data_bytes,
            topic_alias: self.topic_alias,
            subscription_identifiers: self.subscription_identifiers,
            reason_codes: self.reason_codes.into_owned(),
            reason_string: self.reason_string.into_owned(),
            durable_name: self.durable_name.into_owned(),
            shared_group: self.shared_group.into_owned(),
            no_local: self.no_local,
            retain_as_published: self.retain_as_published,
            retain_handling: self.retain_handling,
        }
    }
}

impl BridgeTransportMeta<'_> {
    /// Materializes owned bridge transport metadata for long-lived boundaries.
    pub fn into_owned(self) -> BridgeTransportMeta<'static> {
        IntoOwned::into_owned(self)
    }
}

impl IntoOwned for BridgeTransportMeta<'_> {
    type Owned = BridgeTransportMeta<'static>;
    fn into_owned(self) -> Self::Owned {
        match self {
            BridgeTransportMeta::Http(meta) => BridgeTransportMeta::Http(meta.into_owned()),
            BridgeTransportMeta::Mqtt(meta) => BridgeTransportMeta::Mqtt(meta.into_owned()),
        }
    }
}
