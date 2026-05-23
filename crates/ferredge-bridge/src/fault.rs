use alloc::string::String;

use ferredge_core::prelude::DeviceProtocol;
use serde::{Deserialize, Serialize};

/// Normalized bridge fault with protocol-neutral classification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeFault {
    /// High-level fault category.
    pub category: BridgeFaultCategory,
    /// Optional protocol-native status or exception code.
    pub protocol_code: Option<String>,
    /// Whether retrying the operation may succeed.
    pub retryable: bool,
    /// Optional source context for the fault.
    pub source: Option<BridgeFaultSource>,
    /// Optional human-readable detail.
    pub detail: Option<String>,
}

/// High-level bridge fault category.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BridgeFaultCategory {
    /// The requested mapping or operation is unsupported.
    Unsupported,
    /// Addressed resource or endpoint was not found.
    NotFound,
    /// The provided input could not be represented or validated.
    InvalidInput,
    /// Underlying transport failed.
    Transport,
    /// Operation timed out.
    Timeout,
    /// Protocol-specific failure or exception.
    Protocol,
    /// Operation was explicitly rejected.
    Rejected,
    /// Internal adapter or planner failure.
    Internal,
}

/// Source context associated with a normalized bridge fault.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeFaultSource {
    /// Protocol where the failure originated.
    pub protocol: Option<DeviceProtocol>,
    /// Optional adapter-specific location hint.
    pub location: Option<String>,
}

impl BridgeFault {
    /// Builds a protocol-category fault with protocol identity attached.
    pub fn protocol(
        code: impl Into<String>,
        retryable: bool,
        protocol: DeviceProtocol,
        detail: Option<String>,
    ) -> Self {
        Self {
            category: BridgeFaultCategory::Protocol,
            protocol_code: Some(code.into()),
            retryable,
            source: Some(BridgeFaultSource {
                protocol: Some(protocol),
                location: None,
            }),
            detail,
        }
    }
}
