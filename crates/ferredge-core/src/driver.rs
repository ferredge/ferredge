//! Driver abstraction layer.
//!
//! Drivers are protocol-specific implementations that know how to
//! communicate with devices over a particular protocol (MQTT, Modbus, etc.).

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::device::{Device, DeviceId, DeviceInfo, DeviceProfile};
use crate::error::Result;

/// Unique identifier for a driver.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DriverId(pub String);

impl DriverId {
    /// Creates a new driver ID.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Returns the ID as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for DriverId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<&str> for DriverId {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

/// Information about a driver.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriverInfo {
    /// Unique driver identifier
    pub id: DriverId,
    /// Human-readable name
    pub name: String,
    /// Description
    pub description: Option<String>,
    /// Protocol this driver implements
    pub protocol: String,
    /// Version string
    pub version: String,
}

impl DriverInfo {
    /// Creates new driver info.
    pub fn new(id: impl Into<String>, name: impl Into<String>, protocol: impl Into<String>) -> Self {
        Self {
            id: DriverId::new(id),
            name: name.into(),
            description: None,
            protocol: protocol.into(),
            version: "0.1.0".to_string(),
        }
    }

    /// Sets the description.
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Sets the version.
    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = version.into();
        self
    }
}

/// The core Driver trait that protocol implementations must satisfy.
///
/// A driver is responsible for:
/// - Creating device instances from configuration
/// - Managing the protocol-specific connection
/// - Translating between ferredge's abstract interface and protocol specifics
pub trait Driver: Send + Sync {
    /// Returns information about this driver.
    fn info(&self) -> &DriverInfo;

    /// Initializes the driver.
    fn init(&self) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Shuts down the driver.
    fn shutdown(&self) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Creates a device instance from configuration.
    ///
    /// The driver uses the device info and profile to create an appropriate
    /// device instance that can communicate via this driver's protocol.
    fn create_device(
        &self,
        info: DeviceInfo,
        profile: DeviceProfile,
        config: serde_json::Value,
    ) -> impl std::future::Future<Output = Result<Arc<dyn Device>>> + Send;

    /// Validates device configuration before creation.
    fn validate_config(&self, config: &serde_json::Value) -> Result<()>;

    /// Returns supported profiles for this driver.
    fn supported_profiles(&self) -> Vec<DeviceProfile> {
        Vec::new()
    }
}

/// A type-erased driver reference.
pub type DynDriver = Arc<dyn Driver>;

/// Driver registry for managing available drivers.
pub struct DriverRegistry {
    drivers: std::collections::HashMap<DriverId, DynDriver>,
}

impl DriverRegistry {
    /// Creates a new empty registry.
    pub fn new() -> Self {
        Self {
            drivers: std::collections::HashMap::new(),
        }
    }

    /// Registers a driver.
    pub fn register(&mut self, driver: DynDriver) {
        let id = driver.info().id.clone();
        self.drivers.insert(id, driver);
    }

    /// Gets a driver by ID.
    pub fn get(&self, id: &DriverId) -> Option<&DynDriver> {
        self.drivers.get(id)
    }

    /// Returns all registered drivers.
    pub fn all(&self) -> impl Iterator<Item = &DynDriver> {
        self.drivers.values()
    }

    /// Returns the number of registered drivers.
    pub fn len(&self) -> usize {
        self.drivers.len()
    }

    /// Returns true if no drivers are registered.
    pub fn is_empty(&self) -> bool {
        self.drivers.is_empty()
    }
}

impl Default for DriverRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_driver_info() {
        let info = DriverInfo::new("modbus", "Modbus Driver", "modbus")
            .with_description("Driver for Modbus TCP/RTU devices")
            .with_version("1.0.0");

        assert_eq!(info.id.as_str(), "modbus");
        assert_eq!(info.protocol, "modbus");
    }

    #[test]
    fn test_driver_registry() {
        let registry = DriverRegistry::new();
        assert!(registry.is_empty());
    }
}
