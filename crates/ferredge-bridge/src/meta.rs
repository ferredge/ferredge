use alloc::{
    borrow::Cow,
    string::{String, ToString},
    vec::Vec,
};

use ferredge_core::prelude::{Address, MqttRetainHandling};
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

/// Arbitrary protocol key/value metadata preserved by the bridge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeHeader<'a> {
    /// Header or property name.
    pub key: Cow<'a, str>,
    /// Header or property value.
    pub value: Cow<'a, str>,
}

impl From<(String, String)> for BridgeHeader<'static> {
    fn from((key, value): (String, String)) -> Self {
        Self {
            key: Cow::Owned(key),
            value: Cow::Owned(value),
        }
    }
}

impl From<BridgeHeader<'static>> for (String, String) {
    fn from(value: BridgeHeader<'static>) -> Self {
        (value.key.into_owned(), value.value.into_owned())
    }
}

/// Protocol-tagged arbitrary key/value metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BridgeHeaders<'a> {
    /// Arbitrary HTTP headers preserved in arrival/order form.
    Http(Vec<BridgeHeader<'a>>),
    /// Arbitrary MQTT properties preserved in arrival/order form.
    Mqtt(Vec<BridgeHeader<'a>>),
}

impl<'a> BridgeHeaders<'a> {
    /// Builds HTTP headers from already-materialized `Cow` pairs.
    pub fn http_cow(headers: Vec<(Cow<'a, str>, Cow<'a, str>)>) -> BridgeHeaders<'a> {
        BridgeHeaders::Http(
            headers
                .into_iter()
                .map(|(key, value)| BridgeHeader { key, value })
                .collect(),
        )
    }

    /// Builds MQTT properties from already-materialized `Cow` pairs.
    pub fn mqtt_cow(headers: Vec<(Cow<'a, str>, Cow<'a, str>)>) -> BridgeHeaders<'a> {
        BridgeHeaders::Mqtt(
            headers
                .into_iter()
                .map(|(key, value)| BridgeHeader { key, value })
                .collect(),
        )
    }

    /// Builds owned HTTP headers from convenience `(String, String)` pairs.
    pub fn http(headers: Vec<(String, String)>) -> BridgeHeaders<'static> {
        BridgeHeaders::Http(headers.into_iter().map(BridgeHeader::from).collect())
    }

    /// Builds owned MQTT properties from convenience `(String, String)` pairs.
    pub fn mqtt(headers: Vec<(String, String)>) -> BridgeHeaders<'static> {
        BridgeHeaders::Mqtt(headers.into_iter().map(BridgeHeader::from).collect())
    }

    /// Builds borrowed HTTP headers for zero-copy bridge hot paths.
    pub fn http_borrowed<I, K, V>(headers: I) -> BridgeHeaders<'a>
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<Cow<'a, str>>,
        V: Into<Cow<'a, str>>,
    {
        BridgeHeaders::Http(
            headers
                .into_iter()
                .map(|(key, value)| BridgeHeader {
                    key: key.into(),
                    value: value.into(),
                })
                .collect(),
        )
    }

    /// Builds borrowed MQTT properties for zero-copy bridge hot paths.
    pub fn mqtt_borrowed<I, K, V>(headers: I) -> BridgeHeaders<'a>
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<Cow<'a, str>>,
        V: Into<Cow<'a, str>>,
    {
        BridgeHeaders::Mqtt(
            headers
                .into_iter()
                .map(|(key, value)| BridgeHeader {
                    key: key.into(),
                    value: value.into(),
                })
                .collect(),
        )
    }

    /// Iterates HTTP headers without cloning.
    pub fn iter_http_headers(&self) -> impl Iterator<Item = &BridgeHeader<'a>> {
        match self {
            BridgeHeaders::Http(headers) => headers.iter(),
            BridgeHeaders::Mqtt(_) => [].iter(),
        }
    }

    /// Iterates MQTT properties without cloning.
    pub fn iter_mqtt_headers(&self) -> impl Iterator<Item = &BridgeHeader<'a>> {
        match self {
            BridgeHeaders::Http(_) => [].iter(),
            BridgeHeaders::Mqtt(headers) => headers.iter(),
        }
    }

    /// Returns the underlying HTTP header slice when this is `Http`.
    pub fn as_http(&self) -> Option<&[BridgeHeader<'a>]> {
        match self {
            BridgeHeaders::Http(headers) => Some(headers.as_slice()),
            BridgeHeaders::Mqtt(_) => None,
        }
    }

    /// Returns the underlying MQTT property slice when this is `Mqtt`.
    pub fn as_mqtt(&self) -> Option<&[BridgeHeader<'a>]> {
        match self {
            BridgeHeaders::Http(_) => None,
            BridgeHeaders::Mqtt(headers) => Some(headers.as_slice()),
        }
    }

    /// Materializes owned `(String, String)` pairs for convenience boundaries.
    pub fn to_owned_pairs(&self) -> Vec<(String, String)> {
        match self {
            BridgeHeaders::Http(headers) | BridgeHeaders::Mqtt(headers) => headers
                .iter()
                .map(|header| (header.key.to_string(), header.value.to_string()))
                .collect(),
        }
    }

    /// Materializes owned bridge headers for long-lived outbound planning.
    pub fn into_owned(self) -> BridgeHeaders<'static> {
        match self {
            BridgeHeaders::Http(headers) => BridgeHeaders::Http(
                headers
                    .into_iter()
                    .map(|header| BridgeHeader {
                        key: Cow::Owned(header.key.into_owned()),
                        value: Cow::Owned(header.value.into_owned()),
                    })
                    .collect(),
            ),
            BridgeHeaders::Mqtt(headers) => BridgeHeaders::Mqtt(
                headers
                    .into_iter()
                    .map(|header| BridgeHeader {
                        key: Cow::Owned(header.key.into_owned()),
                        value: Cow::Owned(header.value.into_owned()),
                    })
                    .collect(),
            ),
        }
    }
}

impl From<Vec<(String, String)>> for BridgeHeaders<'static> {
    fn from(value: Vec<(String, String)>) -> Self {
        BridgeHeaders::http(value)
    }
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

/// Borrowable correlation metadata exposed on bridge commands/results.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeCorrelation<'a> {
    /// Correlation/request identifier.
    pub request_id: Cow<'a, str>,
    /// Optional logical reply destination.
    pub reply_to: Option<Address<'a>>,
}

impl BridgeRoute<'_> {
    pub fn into_owned(self) -> BridgeRoute<'static> {
        match self {
            BridgeRoute::RequestResponse { resource, path } => BridgeRoute::RequestResponse {
                resource: Cow::Owned(resource.into_owned()),
                path: path.map(|value| Cow::Owned(value.into_owned())),
            },
            BridgeRoute::Messaging { topic } => BridgeRoute::Messaging {
                topic: Cow::Owned(topic.into_owned()),
            },
            BridgeRoute::AddressedAccess {
                resource,
                access,
                node_id,
            } => BridgeRoute::AddressedAccess {
                resource: Cow::Owned(resource.into_owned()),
                access,
                node_id,
            },
        }
    }
}

impl HttpBridgeMeta<'_> {
    pub fn into_owned(self) -> HttpBridgeMeta<'static> {
        HttpBridgeMeta {
            method: self.method.map(|value| Cow::Owned(value.into_owned())),
            path: self.path.map(|value| Cow::Owned(value.into_owned())),
            status_code: self.status_code,
            content_type: self
                .content_type
                .map(|value| Cow::Owned(value.into_owned())),
        }
    }
}

impl MqttBridgeMeta<'_> {
    pub fn into_owned(self) -> MqttBridgeMeta<'static> {
        MqttBridgeMeta {
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
            correlation_data_bytes: self.correlation_data_bytes,
            topic_alias: self.topic_alias,
            subscription_identifiers: self.subscription_identifiers,
            reason_codes: self
                .reason_codes
                .into_iter()
                .map(|value| Cow::Owned(value.into_owned()))
                .collect(),
            reason_string: self
                .reason_string
                .map(|value| Cow::Owned(value.into_owned())),
            durable_name: self
                .durable_name
                .map(|value| Cow::Owned(value.into_owned())),
            shared_group: self
                .shared_group
                .map(|value| Cow::Owned(value.into_owned())),
            no_local: self.no_local,
            retain_as_published: self.retain_as_published,
            retain_handling: self.retain_handling,
        }
    }
}

impl BridgeTransportMeta<'_> {
    pub fn into_owned(self) -> BridgeTransportMeta<'static> {
        match self {
            BridgeTransportMeta::Http(meta) => BridgeTransportMeta::Http(meta.into_owned()),
            BridgeTransportMeta::Mqtt(meta) => BridgeTransportMeta::Mqtt(meta.into_owned()),
        }
    }
}

impl BridgeCorrelation<'_> {
    pub fn into_owned(self) -> BridgeCorrelation<'static> {
        BridgeCorrelation {
            request_id: Cow::Owned(self.request_id.into_owned()),
            reply_to: self.reply_to.map(Address::into_owned),
        }
    }
}
