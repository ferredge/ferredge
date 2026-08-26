use alloc::string::String;

use ferredge_core::prelude::DeliveryGuarantee;
use serde::{Deserialize, Serialize};

/// Semantic operation family represented by a bridge message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BridgeOp {
    /// Request/response operation.
    RequestResponse(RequestResponseOp),
    /// Message-oriented operation.
    Messaging(MessagingOp),
    /// Register-access operation.
    RegisterAccess(RegisterAccessOp),
}

/// Request/response operation wrapper.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestResponseOp {
    /// Concrete request/response action.
    pub action: RequestResponseAction,
    /// Optional transport-neutral method override such as an HTTP verb.
    pub method: Option<String>,
    /// Optional declared request content type.
    pub content_type: Option<String>,
}

/// Concrete request/response action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RequestResponseAction {
    /// Read a resource.
    Read,
    /// Write a resource.
    Write,
    /// Invoke an operation.
    Invoke,
}

/// Messaging operation wrapper.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessagingOp {
    /// Concrete messaging action.
    pub action: MessagingAction,
    /// Optional delivery guarantee requested for the operation.
    pub delivery: Option<DeliveryGuarantee>,
    /// Optional durable consumer or subscription name.
    pub durable_name: Option<String>,
    /// Optional shared consumer group identifier.
    pub shared_group: Option<String>,
}

/// Concrete messaging action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessagingAction {
    /// Publish or send a message.
    Publish,
    /// Create a subscription.
    Subscribe,
    /// Remove a subscription.
    Unsubscribe,
}

/// Register-access operation wrapper.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterAccessOp {
    /// Concrete register-access action.
    pub action: RegisterAccessAction,
}

/// Concrete register-access action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RegisterAccessAction {
    /// Read a register or field.
    Read,
    /// Write a register or field.
    Write,
}
