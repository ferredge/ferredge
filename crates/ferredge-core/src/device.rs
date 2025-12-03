//! Device abstraction layer.
//!
//! This module defines how devices are represented in ferredge.
//! A device is any external entity that can be read from, written to,
//! or commanded through a supported protocol.

use std::fmt;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::Result;
use crate::message::{Command, CommandResponse, Reading};
use crate::types::{DataType, Value};

/// Unique identifier for a device.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DeviceId(pub String);

impl DeviceId {
    /// Creates a new device ID from a string.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Generates a new random device ID.
    pub fn generate() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    /// Returns the ID as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for DeviceId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for DeviceId {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

/// Current operational state of a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceState {
    /// Device is registered but not yet connected
    Registered,
    /// Device is online and operational
    Online,
    /// Device is offline or unreachable
    Offline,
    /// Device is in an error state
    Error,
    /// Device is disabled by administrator
    Disabled,
}

impl Default for DeviceState {
    fn default() -> Self {
        Self::Registered
    }
}

impl fmt::Display for DeviceState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeviceState::Registered => write!(f, "registered"),
            DeviceState::Online => write!(f, "online"),
            DeviceState::Offline => write!(f, "offline"),
            DeviceState::Error => write!(f, "error"),
            DeviceState::Disabled => write!(f, "disabled"),
        }
    }
}

/// Static information about a device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    /// Unique device identifier
    pub id: DeviceId,
    /// Human-readable device name
    pub name: String,
    /// Optional description
    pub description: Option<String>,
    /// The driver/protocol used to communicate with this device
    pub driver_id: String,
    /// Device-specific labels for categorization
    pub labels: Vec<String>,
    /// Reference to the device profile
    pub profile_name: String,
    /// Timestamp when the device was registered
    pub created_at: DateTime<Utc>,
    /// Last update timestamp
    pub updated_at: DateTime<Utc>,
}

impl DeviceInfo {
    /// Creates a new DeviceInfo with required fields.
    pub fn new(
        name: impl Into<String>,
        driver_id: impl Into<String>,
        profile_name: impl Into<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: DeviceId::generate(),
            name: name.into(),
            description: None,
            driver_id: driver_id.into(),
            labels: Vec::new(),
            profile_name: profile_name.into(),
            created_at: now,
            updated_at: now,
        }
    }

    /// Sets the device ID.
    pub fn with_id(mut self, id: DeviceId) -> Self {
        self.id = id;
        self
    }

    /// Sets the description.
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Adds labels.
    pub fn with_labels(mut self, labels: Vec<String>) -> Self {
        self.labels = labels;
        self
    }
}

/// A device profile defines the resources and capabilities of a device type.
///
/// Multiple devices can share the same profile if they have the same
/// interface (e.g., all temperature sensors of a certain model).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceProfile {
    /// Unique profile name
    pub name: String,
    /// Optional description
    pub description: Option<String>,
    /// Manufacturer name
    pub manufacturer: Option<String>,
    /// Device model
    pub model: Option<String>,
    /// Profile labels for categorization
    pub labels: Vec<String>,
    /// Resources (data points) available on devices with this profile
    pub resources: Vec<DeviceResource>,
    /// Commands that can be executed on devices with this profile
    pub commands: Vec<DeviceCommand>,
}

impl DeviceProfile {
    /// Creates a new device profile.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: None,
            manufacturer: None,
            model: None,
            labels: Vec::new(),
            resources: Vec::new(),
            commands: Vec::new(),
        }
    }

    /// Adds a resource to the profile.
    pub fn with_resource(mut self, resource: DeviceResource) -> Self {
        self.resources.push(resource);
        self
    }

    /// Adds a command to the profile.
    pub fn with_command(mut self, command: DeviceCommand) -> Self {
        self.commands.push(command);
        self
    }
}

/// A resource represents a single data point on a device.
///
/// Resources can be readable, writable, or both.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceResource {
    /// Resource name (unique within the device)
    pub name: String,
    /// Human-readable description
    pub description: Option<String>,
    /// Whether this resource can be read
    pub readable: bool,
    /// Whether this resource can be written
    pub writable: bool,
    /// The data type of this resource
    pub data_type: DataType,
    /// Optional unit of measurement
    pub unit: Option<String>,
    /// Protocol-specific attributes (e.g., Modbus register address)
    pub attributes: serde_json::Value,
}

impl DeviceResource {
    /// Creates a new readable resource.
    pub fn readable(name: impl Into<String>, data_type: DataType) -> Self {
        Self {
            name: name.into(),
            description: None,
            readable: true,
            writable: false,
            data_type,
            unit: None,
            attributes: serde_json::Value::Null,
        }
    }

    /// Creates a new writable resource.
    pub fn writable(name: impl Into<String>, data_type: DataType) -> Self {
        Self {
            name: name.into(),
            description: None,
            readable: false,
            writable: true,
            data_type,
            unit: None,
            attributes: serde_json::Value::Null,
        }
    }

    /// Creates a new read-write resource.
    pub fn read_write(name: impl Into<String>, data_type: DataType) -> Self {
        Self {
            name: name.into(),
            description: None,
            readable: true,
            writable: true,
            data_type,
            unit: None,
            attributes: serde_json::Value::Null,
        }
    }

    /// Sets the unit.
    pub fn with_unit(mut self, unit: impl Into<String>) -> Self {
        self.unit = Some(unit.into());
        self
    }

    /// Sets protocol-specific attributes.
    pub fn with_attributes(mut self, attributes: serde_json::Value) -> Self {
        self.attributes = attributes;
        self
    }
}

/// A command that can be executed on a device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceCommand {
    /// Command name
    pub name: String,
    /// Whether this command reads data
    pub get: bool,
    /// Whether this command writes data
    pub set: bool,
    /// Resources involved in this command
    pub resource_operations: Vec<ResourceOperation>,
}

/// An operation on a resource as part of a command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceOperation {
    /// The resource name
    pub resource: String,
    /// Default value for set operations
    pub default_value: Option<Value>,
    /// Parameter mappings
    pub mappings: serde_json::Value,
}

/// The core Device trait that all device implementations must satisfy.
///
/// This trait defines the interface for communicating with devices.
/// Protocol-specific drivers implement this trait to provide
/// uniform access to different device types.
pub trait Device: Send + Sync + Clone {
    /// Returns the device information.
    fn info(&self) -> &DeviceInfo;

    /// Returns the current device state.
    fn state(&self) -> DeviceState;

    /// Reads a value from a named resource.
    fn read(&self, resource: &str) -> impl std::future::Future<Output = Result<Reading>> + Send;

    /// Writes a value to a named resource.
    fn write(
        &self,
        resource: &str,
        value: Value,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Executes a command on the device.
    fn execute(
        &self,
        command: Command,
    ) -> impl std::future::Future<Output = Result<CommandResponse>> + Send;

    /// Called when the device should connect/initialize.
    fn connect(&self) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Called when the device should disconnect/cleanup.
    fn disconnect(&self) -> impl std::future::Future<Output = Result<()>> + Send;
}

// Delegate Device for Arc<T> so Arc<MockDevice> implements Device when MockDevice does.
impl<T: Device + ?Sized> Device for Arc<T> {
    fn info(&self) -> &DeviceInfo {
        (&**self).info()
    }

    fn state(&self) -> DeviceState {
        (&**self).state()
    }

    fn read(&self, resource: &str) -> impl Future<Output = Result<Reading>> + Send {
        (&**self).read(resource)
    }

    fn write(&self, resource: &str, value: Value) -> impl Future<Output = Result<()>> + Send {
        (&**self).write(resource, value)
    }

    fn execute(&self, command: Command) -> impl Future<Output = Result<CommandResponse>> + Send {
        (&**self).execute(command)
    }

    fn connect(&self) -> impl Future<Output = Result<()>> + Send {
        (&**self).connect()
    }

    fn disconnect(&self) -> impl Future<Output = Result<()>> + Send {
        (&**self).disconnect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_device_id_generation() {
        let id1 = DeviceId::generate();
        let id2 = DeviceId::generate();
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_device_info_builder() {
        let info = DeviceInfo::new("temp-sensor-1", "modbus", "temperature-sensor")
            .with_description("Main hall temperature sensor")
            .with_labels(vec!["temperature".to_string(), "hall".to_string()]);

        assert_eq!(info.name, "temp-sensor-1");
        assert_eq!(info.driver_id, "modbus");
        assert!(info.description.is_some());
        assert_eq!(info.labels.len(), 2);
    }

    #[test]
    fn test_device_profile() {
        let profile = DeviceProfile::new("temperature-sensor")
            .with_resource(
                DeviceResource::readable("temperature", DataType::Float32).with_unit("°C"),
            )
            .with_resource(DeviceResource::readable("humidity", DataType::Float32).with_unit("%"));

        assert_eq!(profile.resources.len(), 2);
    }
}
