extern crate alloc;

use alloc::string::String;

use ferredge_core::prelude::{Device, DeviceEndpoint, ProtocolId};
use serde::{Deserialize, Serialize};

use crate::attributes::HttpResourceAttributes;

/// Device definition bound to the HTTP endpoint and resource model.
pub type HttpDevice = Device<HttpResourceAttributes, HttpEndpointConfig>;

/// HTTP-specific endpoint configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpEndpointConfig {
    /// Base URL used for device requests.
    pub url: String,
}

impl DeviceEndpoint for HttpEndpointConfig {
    fn protocol(&self) -> ProtocolId {
        ProtocolId::HTTP
    }
}
