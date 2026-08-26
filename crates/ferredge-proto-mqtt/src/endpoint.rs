extern crate alloc;

use alloc::{string::String, vec::Vec};

use ferredge_core::prelude::{
    AuthConfig, BrokerReconnectConfig, DeliveryGuarantee, Device, DeviceEndpoint, ProtocolId,
    TlsConfig,
};
use serde::{Deserialize, Serialize};

use crate::meta::MqttPayloadFormat;
use crate::types::MqttResourceAttributes;

/// Device definition bound to the MQTT endpoint and resource model.
pub type MqttDevice = Device<MqttResourceAttributes, MqttEndpointConfig>;

/// MQTT last-will publish configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MqttWillConfig {
    /// Topic where broker publishes will on unexpected disconnect.
    pub topic: String,
    /// Will payload bytes.
    pub payload: Vec<u8>,
    /// Delivery guarantee used for will publish.
    pub delivery: Option<DeliveryGuarantee>,
    /// Whether will should be retained by broker.
    pub retain: bool,
    /// Optional MQTT v5 will delay interval in seconds.
    pub delay_interval_secs: Option<u32>,
    /// Optional MQTT v5 will payload format indicator.
    pub payload_format: Option<MqttPayloadFormat>,
    /// Optional MQTT v5 will message expiry interval in seconds.
    pub message_expiry_interval_secs: Option<u32>,
    /// Optional MQTT v5 will content type.
    pub content_type: Option<String>,
    /// Optional MQTT v5 will response topic.
    pub response_topic: Option<String>,
    /// Optional MQTT v5 will correlation data.
    pub correlation_data: Option<Vec<u8>>,
    /// Optional MQTT v5 will user properties.
    pub user_properties: Vec<(String, String)>,
}

/// MQTT v5 CONNECT property set requested during negotiation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MqttConnectProperties {
    /// Client receive maximum requested during CONNECT.
    pub receive_maximum: Option<u16>,
    /// Maximum packet size client is willing to receive.
    pub maximum_packet_size: Option<u32>,
    /// Maximum topic alias value client supports for inbound traffic.
    pub topic_alias_maximum: Option<u16>,
    /// Whether broker should include response information in CONNACK when available.
    pub request_response_information: Option<bool>,
    /// Whether broker should include detailed problem information on failures.
    pub request_problem_information: Option<bool>,
    /// Optional session expiry interval override in seconds.
    pub session_expiry_interval_secs: Option<u32>,
    /// Optional enhanced authentication method.
    pub authentication_method: Option<String>,
    /// Optional enhanced authentication payload.
    pub authentication_data: Option<Vec<u8>>,
    /// Optional MQTT v5 user properties attached to CONNECT.
    pub user_properties: Vec<(String, String)>,
}

/// Negotiated broker capabilities and identifiers learned from CONNACK.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MqttConnackProperties {
    /// Whether broker resumed an existing session.
    pub session_present: bool,
    /// MQTT v5 connect reason code returned by broker.
    pub reason_code: Option<String>,
    /// Optional human-readable connect reason string.
    pub reason_string: Option<String>,
    /// Broker-assigned client identifier, if one was returned.
    pub assigned_client_identifier: Option<String>,
    /// Response information string returned by broker when requested.
    pub response_information: Option<String>,
    /// Alternate server hint returned by broker.
    pub server_reference: Option<String>,
    /// Server-advertised receive maximum.
    pub receive_maximum: Option<u16>,
    /// Server-advertised maximum packet size.
    pub maximum_packet_size: Option<u32>,
    /// Server-advertised topic alias maximum.
    pub topic_alias_maximum: Option<u16>,
    /// Session expiry interval negotiated by broker.
    pub session_expiry_interval_secs: Option<u32>,
    /// Maximum QoS supported by server.
    pub maximum_qos: Option<u8>,
    /// Whether retained messages are supported by server.
    pub retain_available: Option<bool>,
    /// Server-requested keepalive interval override.
    pub server_keep_alive: Option<u16>,
    /// Whether wildcard subscriptions are supported by server.
    pub wildcard_subscription_available: Option<bool>,
    /// Whether subscription identifiers are supported by server.
    pub subscription_identifier_available: Option<bool>,
    /// Whether shared subscriptions are supported by server.
    pub shared_subscription_available: Option<bool>,
    /// Optional enhanced authentication method selected by broker.
    pub authentication_method: Option<String>,
    /// Optional enhanced authentication data returned by broker.
    pub authentication_data: Option<Vec<u8>>,
    /// MQTT v5 user properties returned on CONNACK.
    pub user_properties: Vec<(String, String)>,
}

/// MQTT-specific endpoint configuration required for real broker connections.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MqttEndpointConfig {
    /// Broker host or address string.
    pub broker: String,
    /// MQTT client identifier.
    pub client_id: String,
    /// Optional username/password authentication.
    pub auth: Option<AuthConfig>,
    /// Optional TLS configuration.
    pub tls: Option<TlsConfig>,
    /// Optional keepalive interval in seconds.
    pub keepalive_secs: Option<u16>,
    /// Whether session starts clean on new connection.
    pub clean_start: bool,
    /// Optional session expiry interval in seconds.
    pub session_expiry_secs: Option<u32>,
    /// Optional default topic prefix or namespace.
    pub topic_prefix: Option<String>,
    /// Optional MQTT v5 CONNECT property set used during negotiation.
    pub connect_properties: MqttConnectProperties,
    /// Optional MQTT last-will configuration.
    pub will: Option<MqttWillConfig>,
    /// Reconnect policy shared with broker-oriented transports.
    pub reconnect: BrokerReconnectConfig,
    /// MQTT protocol versions supported by broker or deployment policy.
    pub supported_versions: Vec<MqttProtocolVersion>,
}

impl DeviceEndpoint for MqttEndpointConfig {
    fn protocol(&self) -> ProtocolId {
        ProtocolId::MQTT
    }
}

/// MQTT protocol versions supported by broker configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MqttProtocolVersion {
    /// MQTT 3.1.1.
    V3_1_1,
    /// MQTT 5.0.
    V5_0,
}

impl MqttEndpointConfig {
    /// Returns highest-preference MQTT version available in config.
    ///
    /// Preference order is MQTT 5.0 first, then MQTT 3.1.1. If no explicit
    /// versions are configured, MQTT 5.0 is assumed by default.
    pub fn preferred_protocol_version(&self) -> MqttProtocolVersion {
        if self.supported_versions.contains(&MqttProtocolVersion::V5_0) {
            MqttProtocolVersion::V5_0
        } else if self
            .supported_versions
            .contains(&MqttProtocolVersion::V3_1_1)
        {
            MqttProtocolVersion::V3_1_1
        } else {
            MqttProtocolVersion::V5_0
        }
    }

    /// Returns whether broker config supports requested MQTT protocol version.
    pub fn supports_protocol_version(&self, version: MqttProtocolVersion) -> bool {
        self.supported_versions.is_empty() || self.supported_versions.contains(&version)
    }
}

#[cfg(test)]
mod tests {
    use alloc::{string::ToString, vec};

    use super::*;

    #[test]
    fn mqtt_endpoint_prefers_v5_then_falls_back_to_v3() {
        let preferred_v5 = MqttEndpointConfig {
            broker: "mqtt://broker".to_string(),
            client_id: "client-v5".to_string(),
            auth: None,
            tls: None,
            keepalive_secs: Some(30),
            clean_start: true,
            session_expiry_secs: None,
            topic_prefix: None,
            connect_properties: MqttConnectProperties::default(),
            reconnect: BrokerReconnectConfig::default(),
            supported_versions: vec![MqttProtocolVersion::V3_1_1, MqttProtocolVersion::V5_0],
            will: None,
        };
        assert_eq!(
            preferred_v5.preferred_protocol_version(),
            MqttProtocolVersion::V5_0
        );
        assert_eq!(preferred_v5.protocol(), ProtocolId::MQTT);

        let fallback_v3 = MqttEndpointConfig {
            supported_versions: vec![MqttProtocolVersion::V3_1_1],
            ..preferred_v5
        };
        assert_eq!(
            fallback_v3.preferred_protocol_version(),
            MqttProtocolVersion::V3_1_1
        );
        assert!(fallback_v3.supports_protocol_version(MqttProtocolVersion::V3_1_1));
        assert!(!fallback_v3.supports_protocol_version(MqttProtocolVersion::V5_0));
    }
}
