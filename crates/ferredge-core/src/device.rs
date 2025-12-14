#[cfg(not(feature = "std"))]
extern crate alloc;

use serde::{Deserialize, Serialize};

#[cfg(feature = "std")]
pub use std::collections::HashMap as Map;

#[cfg(not(feature = "std"))]
pub use alloc::collections::BTreeMap as Map;

pub type DeviceId = String;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeviceStatus {
    Online,
    Offline,
    Maintenance,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DeviceProtocol {
    MQTT,
    HTTP,
    Modbus,
    CoAP,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DeviceEndpoint {
    Http { url: String, port: u16 },
    Mqtt { broker: String, topic: String },
    Modbus { address: u8, port: u16 },
    CoAP { url: String, port: u16 },
}

impl DeviceEndpoint  {
    fn protocol(&self) -> DeviceProtocol {
        match self {
            DeviceEndpoint::Http { .. } => DeviceProtocol::HTTP,
            DeviceEndpoint::Mqtt { .. } => DeviceProtocol::MQTT,
            DeviceEndpoint::Modbus { .. } => DeviceProtocol::Modbus,
            DeviceEndpoint::CoAP { .. } => DeviceProtocol::CoAP,
        }
    }
}

/// Represents the metadata and state of a connected device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    pub id: DeviceId,
    pub name: String,
    pub protocol: DeviceProtocol,
    pub status: DeviceStatus,
    pub endpoint: DeviceEndpoint,
    // Additional metadata can be stored as a generic map or specific struct
    // depending on future needs.
    pub metadata: Option<Map<String, String>>,
}

impl Device {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        endpoint: impl Into<DeviceEndpoint>,
        metadata: Option<Map<String, String>>,
    ) -> Self {
        let endpoint = endpoint.into();
        Self {
            id: id.into(),
            name: name.into(),
            protocol: endpoint.protocol(),
            endpoint: endpoint.clone(),
            status: DeviceStatus::Unknown,
            metadata: metadata,
        }
    }
}
