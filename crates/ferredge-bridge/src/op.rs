use alloc::borrow::Cow;

use ferredge_core::prelude::{DeliveryGuarantee, IntoOwned};
use serde::{Deserialize, Serialize};

/// Semantic operation family represented by a bridge message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BridgeOp<'a> {
    /// Request/response operation.
    RequestResponse(RequestResponseOp<'a>),
    /// Message-oriented operation.
    Messaging(MessagingOp<'a>),
    /// Register-access operation.
    RegisterAccess(RegisterAccessOp),
}

impl BridgeOp<'_> {
    /// Materializes an owned bridge operation for async boundaries.
    pub fn into_owned(self) -> BridgeOp<'static> {
        IntoOwned::into_owned(self)
    }
}

impl IntoOwned for BridgeOp<'_> {
    type Owned = BridgeOp<'static>;
    fn into_owned(self) -> Self::Owned {
        match self {
            Self::RequestResponse(op) => BridgeOp::RequestResponse(op.into_owned()),
            Self::Messaging(op) => BridgeOp::Messaging(op.into_owned()),
            Self::RegisterAccess(op) => BridgeOp::RegisterAccess(op),
        }
    }
}

/// Request/response operation wrapper.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestResponseOp<'a> {
    /// Concrete request/response action.
    pub action: RequestResponseAction,
    /// Optional transport-neutral method override such as an HTTP verb.
    pub method: Option<Cow<'a, str>>,
    /// Optional declared request content type.
    pub content_type: Option<Cow<'a, str>>,
}

impl RequestResponseOp<'_> {
    /// Materializes an owned request/response operation for async boundaries.
    pub fn into_owned(self) -> RequestResponseOp<'static> {
        IntoOwned::into_owned(self)
    }
}

impl IntoOwned for RequestResponseOp<'_> {
    type Owned = RequestResponseOp<'static>;
    fn into_owned(self) -> Self::Owned {
        RequestResponseOp {
            action: self.action,
            method: self.method.into_owned(),
            content_type: self.content_type.into_owned(),
        }
    }
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
pub struct MessagingOp<'a> {
    /// Concrete messaging action.
    pub action: MessagingAction,
    /// Optional delivery guarantee requested for the operation.
    pub delivery: Option<DeliveryGuarantee>,
    /// Optional durable consumer or subscription name.
    pub durable_name: Option<Cow<'a, str>>,
    /// Optional shared consumer group identifier.
    pub shared_group: Option<Cow<'a, str>>,
}

impl MessagingOp<'_> {
    /// Materializes an owned messaging operation for async boundaries.
    pub fn into_owned(self) -> MessagingOp<'static> {
        IntoOwned::into_owned(self)
    }
}

impl IntoOwned for MessagingOp<'_> {
    type Owned = MessagingOp<'static>;
    fn into_owned(self) -> Self::Owned {
        MessagingOp {
            action: self.action,
            delivery: self.delivery,
            durable_name: self.durable_name.into_owned(),
            shared_group: self.shared_group.into_owned(),
        }
    }
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
