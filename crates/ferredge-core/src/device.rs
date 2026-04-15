extern crate alloc;

use alloc::{string::String, vec::Vec};
use core::time::Duration;

use bitflags::bitflags;
use serde::{Deserialize, Serialize};

use crate::command::BrokerAddress;

#[cfg(feature = "std")]
pub use std::collections::{HashMap as Map, VecDeque};

#[cfg(feature = "hashbrown")]
pub use hashbrown::collections::{HashMap as Map, VecDeque};

#[cfg(not(any(feature = "std", feature = "hashbrown")))]
pub use alloc::collections::{BTreeMap as Map, VecDeque};

/// Stable identifier for registered device.
pub type DeviceId = String;

/// High-level device liveness and availability state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeviceStatus {
    Online,
    Offline,
    Maintenance,
    Unknown,
}

/// Transport protocol supported by device endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeviceProtocol {
    MQTT,
    HTTP,
    Modbus,
    CoAP,
}

/// Optional credential set for protocol endpoints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthConfig {
    /// Username or principal used for authentication.
    pub username: Option<String>,
    /// Password or secret associated with username.
    pub password: Option<String>,
}

/// TLS material and flags associated with secure protocol endpoints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TlsConfig {
    /// Whether TLS is enabled for endpoint.
    pub enabled: bool,
    /// Optional PEM-encoded CA certificate.
    pub ca_certificate_pem: Option<String>,
    /// Optional PEM-encoded client certificate.
    pub client_certificate_pem: Option<String>,
    /// Optional PEM-encoded private key for client certificate.
    pub client_key_pem: Option<String>,
}

/// MQTT last-will publish configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MqttWillConfig {
    /// Topic where broker publishes will on unexpected disconnect.
    pub topic: String,
    /// Will payload bytes.
    pub payload: Vec<u8>,
    /// Delivery guarantee used for will publish.
    pub delivery: Option<crate::command::DeliveryGuarantee>,
    /// Whether will should be retained by broker.
    pub retain: bool,
    /// Optional MQTT v5 will delay interval in seconds.
    pub delay_interval_secs: Option<u32>,
    /// Optional MQTT v5 will payload format indicator.
    pub payload_format: Option<crate::command::MqttPayloadFormat>,
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

/// Strategy used to compute delay between broker reconnect attempts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BrokerBackoffStrategy {
    /// Use the same delay for every reconnect attempt.
    Fixed,
    /// Multiply the delay by `multiplier` for each later attempt, capped by `max_delay_ms`.
    Exponential,
}

/// Shared reconnect policy for broker-oriented transports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrokerReconnectConfig {
    /// Whether automatic reconnect attempts are enabled.
    pub enabled: bool,
    /// Delay before the first reconnect attempt in milliseconds.
    pub initial_delay_ms: u64,
    /// Maximum delay between reconnect attempts in milliseconds.
    pub max_delay_ms: u64,
    /// Backoff strategy used to compute retry delay.
    pub strategy: BrokerBackoffStrategy,
    /// Multiplier used by exponential backoff. Ignored for fixed backoff.
    pub multiplier: u32,
    /// Optional maximum reconnect attempts before surfacing failure.
    pub max_attempts: Option<u32>,
    /// Whether broker subscriptions should be replayed after reconnect succeeds.
    pub replay_subscriptions: bool,
    /// Whether outbound requests should be queued while reconnect is in progress.
    pub queue_requests_while_disconnected: bool,
    /// Maximum number of queued outbound recovery requests retained in memory.
    pub max_queued_requests: u32,
}

impl Default for BrokerReconnectConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            initial_delay_ms: 250,
            max_delay_ms: 5_000,
            strategy: BrokerBackoffStrategy::Exponential,
            multiplier: 2,
            max_attempts: None,
            replay_subscriptions: true,
            queue_requests_while_disconnected: true,
            max_queued_requests: 128,
        }
    }
}

impl BrokerReconnectConfig {
    /// Returns whether the given 1-based reconnect attempt is permitted.
    pub fn allows_attempt(&self, attempt: u32) -> bool {
        self.enabled
            && self
                .max_attempts
                .is_none_or(|max_attempts| attempt <= max_attempts)
    }

    /// Returns the backoff delay in milliseconds for the given 1-based reconnect attempt.
    pub fn delay_ms_for_attempt(&self, attempt: u32) -> u64 {
        if attempt <= 1 {
            return self.initial_delay_ms.min(self.max_delay_ms);
        }

        match self.strategy {
            BrokerBackoffStrategy::Fixed => self.initial_delay_ms.min(self.max_delay_ms),
            BrokerBackoffStrategy::Exponential => {
                let multiplier = u64::from(self.multiplier.max(1));
                let mut delay = self.initial_delay_ms.max(1);
                for _ in 1..attempt {
                    delay = delay.saturating_mul(multiplier);
                    if delay >= self.max_delay_ms {
                        return self.max_delay_ms;
                    }
                }
                delay.min(self.max_delay_ms)
            }
        }
    }
}

/// MQTT-specific endpoint configuration required for real broker connections.
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

/// MQTT protocol versions supported by core broker configuration.
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

/// HTTP-specific endpoint configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpEndpointConfig {
    /// Base URL or host:port target for device.
    pub url: String,
}

/// Supported Modbus wire encodings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModbusWireMode {
    Tcp,
    Rtu,
    Ascii,
    Udp,
}

/// Shared Modbus client policy applied across transports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModbusClientOptions {
    /// Remote unit/slave identifier for addressed requests.
    pub unit_id: u8,
    /// Optional end-to-end request timeout.
    pub request_timeout: Option<Duration>,
    /// Optional delay between consecutive requests.
    pub inter_request_delay: Option<Duration>,
    /// Maximum number of retry attempts after initial failure.
    pub max_retries: u32,
}

impl Default for ModbusClientOptions {
    fn default() -> Self {
        Self {
            unit_id: 1,
            request_timeout: None,
            inter_request_delay: None,
            max_retries: 0,
        }
    }
}

/// Number of data bits used by one serial frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SerialDataBits {
    Five,
    Six,
    Seven,
    Eight,
}

/// Parity setting used by serial transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SerialParity {
    None,
    Even,
    Odd,
}

/// Stop-bit count used by serial transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SerialStopBits {
    One,
    Two,
}

/// Flow-control mode used by serial transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SerialFlowControl {
    None,
    Software,
    Hardware,
}

/// Shared serial port configuration usable by Modbus RTU and ASCII.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SerialPortConfig {
    /// Serial port device path or identifier.
    pub path: String,
    /// Serial baudrate used for connection.
    pub baudrate: u32,
    /// Number of data bits per frame.
    pub data_bits: SerialDataBits,
    /// Parity mode used by the line.
    pub parity: SerialParity,
    /// Stop-bit count used by the line.
    pub stop_bits: SerialStopBits,
    /// Flow-control mode, if any.
    pub flow_control: SerialFlowControl,
    /// Optional read timeout.
    pub read_timeout: Option<Duration>,
    /// Optional write timeout.
    pub write_timeout: Option<Duration>,
}

impl Default for SerialPortConfig {
    fn default() -> Self {
        Self {
            path: String::new(),
            baudrate: 9_600,
            data_bits: SerialDataBits::Eight,
            parity: SerialParity::None,
            stop_bits: SerialStopBits::One,
            flow_control: SerialFlowControl::None,
            read_timeout: None,
            write_timeout: None,
        }
    }
}

/// Modbus TCP-specific endpoint configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModbusTcpEndpointConfig {
    /// Remote device address or hostname.
    pub addr: String,
    /// Remote Modbus TCP port.
    pub port: u16,
    /// Shared Modbus client policy for this endpoint.
    pub options: ModbusClientOptions,
}

/// Modbus RTU-specific endpoint configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModbusRtuEndpointConfig {
    /// Shared serial configuration for RTU line access.
    pub serial: SerialPortConfig,
    /// Shared Modbus client policy for this endpoint.
    pub options: ModbusClientOptions,
}

/// Modbus ASCII-specific endpoint configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModbusAsciiEndpointConfig {
    /// Shared serial configuration for ASCII line access.
    pub serial: SerialPortConfig,
    /// Shared Modbus client policy for this endpoint.
    pub options: ModbusClientOptions,
}

/// Modbus UDP-specific endpoint configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModbusUdpEndpointConfig {
    /// Remote device address or hostname.
    pub addr: String,
    /// Remote Modbus UDP port.
    pub port: u16,
    /// Shared Modbus client policy for this endpoint.
    pub options: ModbusClientOptions,
}

/// CoAP-specific endpoint configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoapEndpointConfig {
    /// Base CoAP URL for device.
    pub url: String,
}

/// Concrete connection endpoint for one device transport.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeviceEndpoint {
    Http(HttpEndpointConfig),
    Mqtt(MqttEndpointConfig),
    ModbusTCP(ModbusTcpEndpointConfig),
    ModbusRTU(ModbusRtuEndpointConfig),
    ModbusASCII(ModbusAsciiEndpointConfig),
    ModbusUDP(ModbusUdpEndpointConfig),
    CoAP(CoapEndpointConfig),
}

impl DeviceEndpoint {
    /// Creates an HTTP endpoint from dedicated config.
    pub fn http(config: HttpEndpointConfig) -> Self {
        Self::Http(config)
    }

    /// Creates an MQTT endpoint from dedicated config.
    pub fn mqtt(config: MqttEndpointConfig) -> Self {
        Self::Mqtt(config)
    }

    /// Creates a Modbus TCP endpoint from dedicated config.
    pub fn modbus_tcp(config: ModbusTcpEndpointConfig) -> Self {
        Self::ModbusTCP(config)
    }

    /// Creates a Modbus RTU endpoint from dedicated config.
    pub fn modbus_rtu(config: ModbusRtuEndpointConfig) -> Self {
        Self::ModbusRTU(config)
    }

    /// Creates a Modbus ASCII endpoint from dedicated config.
    pub fn modbus_ascii(config: ModbusAsciiEndpointConfig) -> Self {
        Self::ModbusASCII(config)
    }

    /// Creates a Modbus UDP endpoint from dedicated config.
    pub fn modbus_udp(config: ModbusUdpEndpointConfig) -> Self {
        Self::ModbusUDP(config)
    }

    /// Creates a CoAP endpoint from dedicated config.
    pub fn coap(config: CoapEndpointConfig) -> Self {
        Self::CoAP(config)
    }

    /// Returns protocol implied by endpoint variant.
    pub fn protocol(&self) -> DeviceProtocol {
        match self {
            DeviceEndpoint::Http(_) => DeviceProtocol::HTTP,
            DeviceEndpoint::Mqtt(_) => DeviceProtocol::MQTT,
            DeviceEndpoint::ModbusTCP(_) => DeviceProtocol::Modbus,
            DeviceEndpoint::ModbusRTU(_) => DeviceProtocol::Modbus,
            DeviceEndpoint::ModbusASCII(_) => DeviceProtocol::Modbus,
            DeviceEndpoint::ModbusUDP(_) => DeviceProtocol::Modbus,
            DeviceEndpoint::CoAP(_) => DeviceProtocol::CoAP,
        }
    }
}

/// Marker trait for protocol-specific device resource metadata.
pub trait DeviceResourceAttributes:
    Clone + core::fmt::Debug + for<'de> Deserialize<'de> + Serialize
{
}

bitflags! {
    /// Allowed operations for device resource or channel descriptors.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
    pub struct DeviceResourceAccessPermission: u8 {
        const READ      = 0b0000_0001;
        const WRITE     = 0b0000_0010;
        const EXECUTE   = 0b0000_0100;
        const PUBLISH   = 0b0000_1000;
        const SUBSCRIBE = 0b0001_0000;
    }
}

/// Protocol-specific resource descriptor attached to one device.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(bound(deserialize = "T: DeviceResourceAttributes"))]
pub struct DeviceResource<T>
where
    T: DeviceResourceAttributes,
{
    /// Human-readable resource name.
    pub name: String,
    /// Protocol-specific resource metadata.
    pub resource_attributes: T,
    /// Optional engineering unit for resource value.
    pub unit: Option<String>,
    /// Optional allowed operations for this resource.
    pub permission: Option<DeviceResourceAccessPermission>,
}

/// Broker-oriented destination or subscription descriptor attached to one device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceMessageEndpoint {
    /// Human-readable endpoint name.
    pub name: String,
    /// Concrete broker address such as topic, queue, subject, or stream.
    pub address: BrokerAddress,
    /// Optional transport-agnostic broker metadata entries.
    pub metadata: Option<Map<String, String>>,
    /// Optional allowed operations for this broker endpoint.
    pub permission: Option<DeviceResourceAccessPermission>,
}

/// Represents the metadata and state of a connected device.
#[derive(Debug, Clone)]
pub struct Device<T: DeviceResourceAttributes> {
    /// Stable device identifier.
    pub id: DeviceId,
    /// Human-readable device name.
    pub name: String,
    /// Current reported device status.
    pub status: DeviceStatus,
    /// Concrete transport endpoint details.
    pub endpoint: DeviceEndpoint,
    /// Optional free-form metadata entries.
    pub metadata: Option<Map<String, String>>,
    /// Optional maximum concurrent connections supported by device.
    pub max_connections: Option<u32>,
    /// Resource descriptors addressable through request/response style protocols.
    pub resources: Map<String, DeviceResource<T>>,
    /// Broker endpoint descriptors addressable through message-oriented protocols.
    pub message_endpoints: Vec<DeviceMessageEndpoint>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_endpoint_creators_return_expected_protocols() {
        assert_eq!(
            DeviceEndpoint::http(HttpEndpointConfig {
                url: "127.0.0.1:8080".to_string(),
            })
            .protocol(),
            DeviceProtocol::HTTP
        );
        assert_eq!(
            DeviceEndpoint::mqtt(MqttEndpointConfig {
                broker: "mqtt://broker".to_string(),
                client_id: "client-1".to_string(),
                auth: None,
                tls: None,
                keepalive_secs: Some(30),
                clean_start: true,
                session_expiry_secs: None,
                topic_prefix: None,
                will: None,
                connect_properties: MqttConnectProperties::default(),
                reconnect: BrokerReconnectConfig::default(),
                supported_versions: vec![MqttProtocolVersion::V5_0, MqttProtocolVersion::V3_1_1],
            })
            .protocol(),
            DeviceProtocol::MQTT
        );
        assert_eq!(
            DeviceEndpoint::modbus_tcp(ModbusTcpEndpointConfig {
                addr: "127.0.0.1".to_string(),
                port: 502,
                options: ModbusClientOptions::default(),
            })
            .protocol(),
            DeviceProtocol::Modbus
        );
        assert_eq!(
            DeviceEndpoint::modbus_rtu(ModbusRtuEndpointConfig {
                serial: SerialPortConfig {
                    path: "/dev/ttyUSB0".to_string(),
                    ..SerialPortConfig::default()
                },
                options: ModbusClientOptions::default(),
            })
            .protocol(),
            DeviceProtocol::Modbus
        );
        assert_eq!(
            DeviceEndpoint::modbus_ascii(ModbusAsciiEndpointConfig {
                serial: SerialPortConfig {
                    path: "/dev/ttyUSB1".to_string(),
                    baudrate: 19_200,
                    ..SerialPortConfig::default()
                },
                options: ModbusClientOptions::default(),
            })
            .protocol(),
            DeviceProtocol::Modbus
        );
        assert_eq!(
            DeviceEndpoint::modbus_udp(ModbusUdpEndpointConfig {
                addr: "127.0.0.1".to_string(),
                port: 502,
                options: ModbusClientOptions::default(),
            })
            .protocol(),
            DeviceProtocol::Modbus
        );
        assert_eq!(
            DeviceEndpoint::coap(CoapEndpointConfig {
                url: "coap://127.0.0.1".to_string(),
            })
            .protocol(),
            DeviceProtocol::CoAP
        );
    }

    #[test]
    fn modbus_client_options_default_to_common_safe_values() {
        assert_eq!(
            ModbusClientOptions::default(),
            ModbusClientOptions {
                unit_id: 1,
                request_timeout: None,
                inter_request_delay: None,
                max_retries: 0,
            }
        );
    }

    #[test]
    fn serial_port_config_defaults_match_common_modbus_line_settings() {
        assert_eq!(
            SerialPortConfig::default(),
            SerialPortConfig {
                path: String::new(),
                baudrate: 9_600,
                data_bits: SerialDataBits::Eight,
                parity: SerialParity::None,
                stop_bits: SerialStopBits::One,
                flow_control: SerialFlowControl::None,
                read_timeout: None,
                write_timeout: None,
            }
        );
    }

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

        let fallback_v3 = MqttEndpointConfig {
            broker: "mqtt://broker".to_string(),
            client_id: "client-v3".to_string(),
            auth: None,
            tls: None,
            keepalive_secs: Some(30),
            clean_start: true,
            session_expiry_secs: None,
            topic_prefix: None,
            connect_properties: MqttConnectProperties::default(),
            reconnect: BrokerReconnectConfig::default(),
            supported_versions: vec![MqttProtocolVersion::V3_1_1],
            will: None,
        };
        assert_eq!(
            fallback_v3.preferred_protocol_version(),
            MqttProtocolVersion::V3_1_1
        );
        assert!(fallback_v3.supports_protocol_version(MqttProtocolVersion::V3_1_1));
        assert!(!fallback_v3.supports_protocol_version(MqttProtocolVersion::V5_0));
    }

    #[test]
    fn broker_reconnect_backoff_caps_and_respects_attempt_limit() {
        let config = BrokerReconnectConfig {
            enabled: true,
            initial_delay_ms: 100,
            max_delay_ms: 1_000,
            strategy: BrokerBackoffStrategy::Exponential,
            multiplier: 3,
            max_attempts: Some(3),
            replay_subscriptions: true,
            queue_requests_while_disconnected: true,
            max_queued_requests: 16,
        };

        assert!(config.allows_attempt(1));
        assert!(config.allows_attempt(3));
        assert!(!config.allows_attempt(4));
        assert_eq!(config.delay_ms_for_attempt(1), 100);
        assert_eq!(config.delay_ms_for_attempt(2), 300);
        assert_eq!(config.delay_ms_for_attempt(3), 900);
        assert_eq!(config.delay_ms_for_attempt(4), 1_000);
    }
}
