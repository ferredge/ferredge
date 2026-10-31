extern crate alloc;

use alloc::{borrow::Cow, string::String, vec::Vec};
use ferredge_core::prelude::*;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Zenoh operation type for resource attributes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ZenohOperation {
    /// Publish operation (put).
    Publish,
    /// Subscribe operation.
    Subscribe,
    /// Query operation (get).
    Query,
    /// Queryable operation (respond to queries).
    Queryable,
}

/// Zenoh-specific resource attributes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZenohResourceAttributes {
    /// Zenoh key expression (e.g., "sensors/temperature").
    pub key_expr: String,
    /// Operation type for this resource.
    #[serde(default = "default_operation")]
    pub operation: ZenohOperation,
    /// Optional query timeout in milliseconds.
    pub query_timeout_ms: Option<u64>,
}

fn default_operation() -> ZenohOperation {
    ZenohOperation::Publish
}

/// Native plan for Zenoh operations with lifetime-based zero-copy.
#[derive(Debug, Clone, PartialEq)]
pub enum ZenohNativePlan<'a> {
    /// Publish data to key expression.
    Put {
        command_id: Cow<'a, str>,
        key_expr: Cow<'a, str>,
        payload: PayloadValue<'a>,
    },
    /// Subscribe to key expression.
    Subscribe {
        command_id: Cow<'a, str>,
        key_expr: Cow<'a, str>,
    },
    /// Unsubscribe from key expression.
    Unsubscribe {
        command_id: Cow<'a, str>,
        key_expr: Cow<'a, str>,
    },
    /// Execute GET query.
    Get {
        command_id: Cow<'a, str>,
        key_expr: Cow<'a, str>,
        parameters: Option<Cow<'a, str>>,
    },
    /// Declare queryable.
    DeclareQueryable {
        command_id: Cow<'a, str>,
        key_expr: Cow<'a, str>,
    },
    /// Undeclare queryable.
    UndeclareQueryable {
        command_id: Cow<'a, str>,
        key_expr: Cow<'a, str>,
    },
}

impl ZenohNativePlan<'_> {
    /// Convert to owned version with 'static lifetime.
    pub fn into_owned(self) -> ZenohNativePlan<'static> {
        match self {
            Self::Put {
                command_id,
                key_expr,
                payload,
            } => ZenohNativePlan::Put {
                command_id: Cow::Owned(command_id.into_owned()),
                key_expr: Cow::Owned(key_expr.into_owned()),
                payload: payload.into_owned(),
            },
            Self::Subscribe {
                command_id,
                key_expr,
            } => ZenohNativePlan::Subscribe {
                command_id: Cow::Owned(command_id.into_owned()),
                key_expr: Cow::Owned(key_expr.into_owned()),
            },
            Self::Unsubscribe {
                command_id,
                key_expr,
            } => ZenohNativePlan::Unsubscribe {
                command_id: Cow::Owned(command_id.into_owned()),
                key_expr: Cow::Owned(key_expr.into_owned()),
            },
            Self::Get {
                command_id,
                key_expr,
                parameters,
            } => ZenohNativePlan::Get {
                command_id: Cow::Owned(command_id.into_owned()),
                key_expr: Cow::Owned(key_expr.into_owned()),
                parameters: parameters.map(|p| Cow::Owned(p.into_owned())),
            },
            Self::DeclareQueryable {
                command_id,
                key_expr,
            } => ZenohNativePlan::DeclareQueryable {
                command_id: Cow::Owned(command_id.into_owned()),
                key_expr: Cow::Owned(key_expr.into_owned()),
            },
            Self::UndeclareQueryable {
                command_id,
                key_expr,
            } => ZenohNativePlan::UndeclareQueryable {
                command_id: Cow::Owned(command_id.into_owned()),
                key_expr: Cow::Owned(key_expr.into_owned()),
            },
        }
    }
}

/// Borrowed command reference for Zenoh driver operations.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ZenohCommandRef<'a> {
    #[allow(dead_code)]
    pub device: &'a crate::ZenohDevice,
}

/// Zenoh publish request.
#[derive(Debug, Clone, PartialEq)]
pub struct ZenohPublishRequest {
    /// Original command identifier for correlation.
    pub command_id: String,
    /// Zenoh key expression to publish to.
    pub key_expr: String,
    /// Message payload.
    pub payload: Vec<u8>,
}

/// Zenoh subscription request.
#[derive(Debug, Clone, PartialEq)]
pub struct ZenohSubscription {
    /// Original command identifier for correlation.
    pub command_id: String,
    /// Zenoh key expression to subscribe to.
    pub key_expr: String,
}

/// Zenoh query request.
#[derive(Debug, Clone, PartialEq)]
pub struct ZenohQueryRequest {
    /// Original command identifier for correlation.
    pub command_id: String,
    /// Zenoh key expression to query.
    pub key_expr: String,
    /// Optional query selector parameters.
    pub parameters: Option<String>,
    /// Optional query timeout in milliseconds.
    pub timeout_ms: Option<u64>,
}

/// Zenoh request types.
#[derive(Debug, Clone, PartialEq)]
pub enum ZenohRequest {
    /// Publish request.
    Publish(ZenohPublishRequest),
    /// Query request.
    Query(ZenohQueryRequest),
}

/// Zenoh query response.
#[derive(Debug, Clone, PartialEq)]
pub struct ZenohQueryResponse {
    pub replies: Vec<ZenohReply>,
}

/// Single reply from a query.
#[derive(Debug, Clone, PartialEq)]
pub struct ZenohReply {
    pub key_expr: String,
    pub payload: Vec<u8>,
}

/// Zenoh response types.
#[derive(Debug, Clone, PartialEq)]
pub enum ZenohResponse {
    /// Publish acknowledgment (empty).
    Published,
    /// Query response with replies.
    QueryResponse(ZenohQueryResponse),
}

/// Zenoh event from subscription or queryable.
#[derive(Debug, Clone, PartialEq)]
pub struct ZenohEvent {
    pub key_expr: String,
    pub payload: Vec<u8>,
}

/// Zenoh protocol errors.
#[derive(Debug, Error)]
pub enum ZenohError {
    #[error("Zenoh session not initialized")]
    SessionNotInitialized,

    #[error("Zenoh session initialization failed: {0}")]
    SessionInitFailed(String),

    #[error("Invalid key expression: {0}")]
    InvalidKeyExpr(String),

    #[error("Subscription not found: {0}")]
    SubscriptionNotFound(String),

    #[error("Queryable not found: {0}")]
    QueryableNotFound(String),

    #[error("Query timeout")]
    QueryTimeout,

    #[error("Listener not running")]
    ListenerNotRunning,

    #[error("Listener already running")]
    ListenerAlreadyRunning,

    #[error("IO error: {0}")]
    Io(String),

    #[error("Runtime not available (std feature required)")]
    RuntimeNotAvailable,

    #[error("Other error: {0}")]
    Other(String),
}

/// Command conversion errors.
#[derive(Debug, Error)]
pub enum ZenohCommandConversionError {
    #[error("Unsupported intent for Zenoh: {0}")]
    UnsupportedIntent(String),

    #[error("Missing required field: {0}")]
    MissingField(String),

    #[error("Invalid address type: expected {0}")]
    InvalidAddressType(String),

    #[error("Payload encoding error: {0}")]
    PayloadEncoding(String),
}
