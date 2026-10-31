extern crate alloc;

use alloc::{string::String, vec::Vec};

use ferredge_core::prelude::{Device, DeviceEndpoint, ProtocolId};
use serde::{Deserialize, Serialize};

use crate::types::ZenohResourceAttributes;

/// Device definition bound to the Zenoh endpoint and resource model.
pub type ZenohDevice = Device<ZenohResourceAttributes, ZenohEndpointConfig>;

/// Zenoh-specific endpoint configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ZenohEndpointConfig {
    /// Optional Zenoh locators to connect to (e.g., "tcp/localhost:7447").
    /// If empty, uses default Zenoh discovery.
    #[serde(default)]
    pub locators: Vec<String>,
}

impl DeviceEndpoint for ZenohEndpointConfig {
    fn protocol(&self) -> ProtocolId {
        ProtocolId::ZENOH
    }
}
