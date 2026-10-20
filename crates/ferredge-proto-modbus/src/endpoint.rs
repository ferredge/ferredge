extern crate alloc;

use alloc::string::String;
use core::time::Duration;

use ferredge_core::prelude::{
    BackoffStrategy, Device, DeviceEndpoint, ProtocolId, SerialPortConfig,
};
use rmodbus::ModbusProto;
use serde::{Deserialize, Serialize};

use crate::attributes::ModbusResourceAttributes;

/// Device definition bound to the Modbus endpoint and resource model.
pub type ModbusDevice = Device<ModbusResourceAttributes, ModbusEndpoint>;

/// Reconnect and retry policy for Modbus client transports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModbusReconnectConfig {
    /// Whether automatic retry attempts are enabled.
    pub enabled: bool,
    /// Delay before first retry attempt after request failure.
    pub initial_delay: Option<Duration>,
    /// Maximum retry delay cap. Falls back to `initial_delay` when omitted.
    pub max_delay: Option<Duration>,
    /// Backoff strategy used to compute retry delay.
    pub strategy: BackoffStrategy,
    /// Multiplier used by exponential backoff. Ignored for fixed backoff.
    pub multiplier: u32,
    /// Maximum number of retry attempts after initial failure.
    pub max_attempts: u32,
    /// Whether write requests may be retried automatically.
    pub retry_writes: bool,
}

impl Default for ModbusReconnectConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            initial_delay: None,
            max_delay: None,
            strategy: BackoffStrategy::Fixed,
            multiplier: 2,
            max_attempts: 0,
            retry_writes: false,
        }
    }
}

impl ModbusReconnectConfig {
    /// Returns whether the given 1-based reconnect attempt is permitted.
    pub fn allows_attempt(&self, attempt: u32) -> bool {
        self.enabled && attempt <= self.max_attempts
    }

    /// Returns retry delay for given 1-based retry attempt.
    pub fn delay_for_attempt(&self, attempt: u32) -> Option<Duration> {
        let initial_delay = self.initial_delay?;
        if attempt <= 1 {
            return Some(initial_delay);
        }

        let max_delay = self.max_delay.unwrap_or(initial_delay);
        match self.strategy {
            BackoffStrategy::Fixed => Some(initial_delay.min(max_delay)),
            BackoffStrategy::Exponential => {
                let multiplier = u128::from(self.multiplier.max(1));
                let mut millis = initial_delay.as_millis().max(1);
                let max_millis = max_delay.as_millis().max(1);
                for _ in 1..attempt {
                    millis = millis.saturating_mul(multiplier);
                    if millis >= max_millis {
                        millis = max_millis;
                        break;
                    }
                }
                Some(Duration::from_millis(
                    millis.min(u128::from(u64::MAX)) as u64
                ))
            }
            // Forward-compat: strategies added later fall back to a fixed delay.
            _ => Some(initial_delay.min(max_delay)),
        }
    }
}

/// Client behavior options shared across Modbus transports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModbusClientOptions {
    /// Remote unit/slave identifier for addressed requests.
    pub unit_id: u8,
    /// Optional end-to-end request timeout.
    pub request_timeout: Option<Duration>,
    /// Whether TCP/serial sessions should be kept open and reused between requests.
    pub persistent_session: bool,
    /// Reconnect/retry policy for failed requests.
    pub reconnect: ModbusReconnectConfig,
}

impl Default for ModbusClientOptions {
    fn default() -> Self {
        Self {
            unit_id: 1,
            request_timeout: None,
            persistent_session: false,
            reconnect: ModbusReconnectConfig::default(),
        }
    }
}

/// Modbus TCP endpoint configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModbusTcpEndpointConfig {
    /// Remote device address or hostname.
    pub addr: String,
    /// Remote Modbus TCP port.
    pub port: u16,
    /// Shared Modbus client policy for this endpoint.
    pub options: ModbusClientOptions,
}

/// Modbus RTU-over-TCP endpoint configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModbusRtuOverTcpEndpointConfig {
    /// Remote gateway address or hostname.
    pub addr: String,
    /// Remote gateway TCP port.
    pub port: u16,
    /// Shared Modbus client policy for this endpoint.
    pub options: ModbusClientOptions,
}

/// Modbus RTU serial endpoint configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModbusRtuEndpointConfig {
    /// Shared serial configuration for RTU line access.
    pub serial: SerialPortConfig,
    /// Shared Modbus client policy for this endpoint.
    pub options: ModbusClientOptions,
}

/// Modbus ASCII serial endpoint configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModbusAsciiEndpointConfig {
    /// Shared serial configuration for ASCII line access.
    pub serial: SerialPortConfig,
    /// Shared Modbus client policy for this endpoint.
    pub options: ModbusClientOptions,
}

/// Modbus UDP endpoint configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModbusUdpEndpointConfig {
    /// Remote device address or hostname.
    pub addr: String,
    /// Remote Modbus UDP port.
    pub port: u16,
    /// Shared Modbus client policy for this endpoint.
    pub options: ModbusClientOptions,
}

/// Modbus endpoint configuration across supported wire modes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModbusEndpoint {
    /// Modbus TCP.
    Tcp(ModbusTcpEndpointConfig),
    /// Modbus RTU frames tunneled over TCP.
    RtuOverTcp(ModbusRtuOverTcpEndpointConfig),
    /// Modbus RTU over a serial line.
    Rtu(ModbusRtuEndpointConfig),
    /// Modbus ASCII over a serial line.
    Ascii(ModbusAsciiEndpointConfig),
    /// Modbus UDP.
    Udp(ModbusUdpEndpointConfig),
}

impl ModbusEndpoint {
    /// Returns client behavior options for this endpoint.
    pub fn options(&self) -> &ModbusClientOptions {
        match self {
            Self::Tcp(config) => &config.options,
            Self::RtuOverTcp(config) => &config.options,
            Self::Rtu(config) => &config.options,
            Self::Ascii(config) => &config.options,
            Self::Udp(config) => &config.options,
        }
    }

    /// Returns the rmodbus wire framing used by this endpoint.
    pub fn proto(&self) -> ModbusProto {
        match self {
            Self::Tcp(_) | Self::Udp(_) => ModbusProto::TcpUdp,
            Self::RtuOverTcp(_) | Self::Rtu(_) => ModbusProto::Rtu,
            Self::Ascii(_) => ModbusProto::Ascii,
        }
    }
}

impl DeviceEndpoint for ModbusEndpoint {
    fn protocol(&self) -> ProtocolId {
        ProtocolId::MODBUS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modbus_client_options_default_to_common_safe_values() {
        assert_eq!(
            ModbusClientOptions::default(),
            ModbusClientOptions {
                unit_id: 1,
                request_timeout: None,
                persistent_session: false,
                reconnect: ModbusReconnectConfig::default(),
            }
        );
    }

    #[test]
    fn modbus_reconnect_config_compute_retry_delay() {
        let reconnect = ModbusReconnectConfig {
            initial_delay: Some(Duration::from_millis(100)),
            max_delay: Some(Duration::from_millis(500)),
            strategy: BackoffStrategy::Exponential,
            multiplier: 2,
            ..ModbusReconnectConfig::default()
        };

        assert_eq!(
            reconnect.delay_for_attempt(1),
            Some(Duration::from_millis(100))
        );
        assert_eq!(
            reconnect.delay_for_attempt(2),
            Some(Duration::from_millis(200))
        );
        assert_eq!(
            reconnect.delay_for_attempt(3),
            Some(Duration::from_millis(400))
        );
        assert_eq!(
            reconnect.delay_for_attempt(4),
            Some(Duration::from_millis(500))
        );
    }
}
