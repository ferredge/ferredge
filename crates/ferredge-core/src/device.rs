#[cfg(not(feature = "std"))]
extern crate alloc;

use bitflags::bitflags;
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
    Http { url: String },
    Mqtt { broker: String },
    ModbusTCP { addr: String, port: u16 },
    ModbusRTU { port: String, baudrate: u32 },
    CoAP { url: String },
}

impl DeviceEndpoint {
    pub fn protocol(&self) -> DeviceProtocol {
        match self {
            DeviceEndpoint::Http { .. } => DeviceProtocol::HTTP,
            DeviceEndpoint::Mqtt { .. } => DeviceProtocol::MQTT,
            DeviceEndpoint::ModbusTCP { .. } => DeviceProtocol::Modbus,
            DeviceEndpoint::ModbusRTU { .. } => DeviceProtocol::Modbus,
            DeviceEndpoint::CoAP { .. } => DeviceProtocol::CoAP,
        }
    }
}

// marker trait for device resource types
pub trait DeviceResourceAttributes: for<'de> Deserialize<'de> + Serialize {}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
    pub struct DeviceResourceAccessPermission: u8 {
        const READ    = 0b0000_0001;
        const WRITE   = 0b0000_0010;
        const EXECUTE = 0b0000_0100;
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(bound(deserialize = "T: DeviceResourceAttributes"))]
pub struct DeviceResource<T>
where
    T: DeviceResourceAttributes,
{
    pub name: String,
    pub resource_attributes: T,
    pub unit: Option<String>,
    pub permission: Option<DeviceResourceAccessPermission>,
}

/// Represents the metadata and state of a connected device.
#[derive(Debug)]
pub struct Device<T: DeviceResourceAttributes> {
    pub id: DeviceId,
    pub name: String,
    pub status: DeviceStatus,
    pub endpoint: DeviceEndpoint,
    // Additional metadata can be stored as a generic map or specific struct
    // depending on future needs.
    pub metadata: Option<Map<String, String>>,
    // max resources the device can handle
    pub max_connections: Option<u32>,

    pub resources: Option<Vec<DeviceResource<T>>>,
}
