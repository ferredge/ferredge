#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(feature = "std")]
use std::{string::String, vec::Vec};

#[cfg(not(feature = "std"))]
use alloc::{collections::BTreeMap as StdlessMap, string::String, vec::Vec};

use bitflags::bitflags;
use serde::{Deserialize, Serialize};

use crate::command::BrokerAddress;

#[cfg(feature = "std")]
pub use std::collections::HashMap as Map;

#[cfg(not(feature = "std"))]
pub type Map<K, V> = StdlessMap<K, V>;

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
}

/// HTTP-specific endpoint configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpEndpointConfig {
    /// Base URL or host:port target for device.
    pub url: String,
}

/// Modbus TCP-specific endpoint configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModbusTcpEndpointConfig {
    /// Remote device address or hostname.
    pub addr: String,
    /// Remote Modbus TCP port.
    pub port: u16,
}

/// Modbus RTU-specific endpoint configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModbusRtuEndpointConfig {
    /// Serial port device path.
    pub port: String,
    /// Serial baudrate used for connection.
    pub baudrate: u32,
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
            DeviceEndpoint::CoAP(_) => DeviceProtocol::CoAP,
        }
    }
}

/// Marker trait for protocol-specific device resource metadata.
pub trait DeviceResourceAttributes: Clone + core::fmt::Debug + for<'de> Deserialize<'de> + Serialize {}

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
            })
            .protocol(),
            DeviceProtocol::MQTT
        );
        assert_eq!(
            DeviceEndpoint::modbus_tcp(ModbusTcpEndpointConfig {
                addr: "127.0.0.1".to_string(),
                port: 502,
            })
            .protocol(),
            DeviceProtocol::Modbus
        );
        assert_eq!(
            DeviceEndpoint::modbus_rtu(ModbusRtuEndpointConfig {
                port: "/dev/ttyUSB0".to_string(),
                baudrate: 9600,
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
}
