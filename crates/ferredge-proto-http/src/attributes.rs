use ferredge_core::prelude::DeviceResourceAttributes;
use serde::{Deserialize, Serialize};

/// HTTP-specific resource metadata used to build outbound requests.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpResourceAttributes {
    /// Request path for this resource.
    pub slug: String,
    /// HTTP method to use when interacting with this resource.
    pub method: String,
    /// Optional static headers attached to requests for this resource.
    pub headers: Option<Vec<(String, String)>>,
}

impl DeviceResourceAttributes for HttpResourceAttributes {}
