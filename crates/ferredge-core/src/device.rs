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

/// Represents the metadata and state of a connected device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    pub id: DeviceId,
    pub name: String,
    pub protocol: String,
    pub status: DeviceStatus,
    // Additional metadata can be stored as a generic map or specific struct
    // depending on future needs.
    pub metadata: Option<Map<String, String>>,
}

impl Device {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        protocol: impl Into<String>,
        metadata: Option<Map<String, String>>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            protocol: protocol.into(),
            status: DeviceStatus::Unknown,
            metadata: metadata,
        }
    }
}
