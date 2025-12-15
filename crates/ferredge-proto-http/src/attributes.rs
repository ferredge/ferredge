use ferredge_core::prelude::DeviceResourceAttributes;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpResourceAttributes {
    pub slug: String,
    pub method: String,
    pub headers: Option<Vec<(String, String)>>,
}

impl DeviceResourceAttributes for HttpResourceAttributes {}
