extern crate alloc;

use alloc::{string::String, vec::Vec};
use core::time::Duration;

use bitflags::bitflags;
use serde::{Deserialize, Serialize};

use crate::command::BrokerAddress;
use crate::id::DeviceId;
use crate::meta::ProtocolId;

#[cfg(feature = "std")]
pub use std::collections::{HashMap as Map, VecDeque};

#[cfg(feature = "hashbrown")]
pub use hashbrown::collections::{HashMap as Map, VecDeque};

#[cfg(not(any(feature = "std", feature = "hashbrown")))]
pub use alloc::collections::{BTreeMap as Map, VecDeque};

/// High-level device liveness and availability state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum DeviceStatus {
    Online,
    Offline,
    Maintenance,
    Unknown,
}

/// Strategy used to compute delay between broker reconnect attempts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum BackoffStrategy {
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
    pub strategy: BackoffStrategy,
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
            strategy: BackoffStrategy::Exponential,
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
            BackoffStrategy::Fixed => self.initial_delay_ms.min(self.max_delay_ms),
            BackoffStrategy::Exponential => {
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

/// Protocol-owned device endpoint configuration.
pub trait DeviceEndpoint: Clone + core::fmt::Debug + Serialize + for<'de> Deserialize<'de> {
    /// Returns protocol identity for this endpoint configuration.
    fn protocol(&self) -> ProtocolId;
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
pub struct Device<T: DeviceResourceAttributes, E: DeviceEndpoint> {
    /// Stable device identifier.
    pub id: DeviceId,
    /// Human-readable device name.
    pub name: String,
    /// Current reported device status.
    pub status: DeviceStatus,
    /// Protocol-owned transport endpoint configuration.
    pub endpoint: E,
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
    fn broker_reconnect_backoff_caps_and_respects_attempt_limit() {
        let config = BrokerReconnectConfig {
            enabled: true,
            initial_delay_ms: 100,
            max_delay_ms: 1_000,
            strategy: BackoffStrategy::Exponential,
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
