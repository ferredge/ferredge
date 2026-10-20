extern crate alloc;

use alloc::borrow::Cow;

use serde::{Deserialize, Serialize};

use crate::{command::Address, owned::IntoOwned};

/// Typed addressed-access metadata used by addressed field/register planners and codecs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct AddressedAccessMeta {
    /// Protocol-defined base address or offset.
    pub address: u32,
    /// Protocol-defined address-space or domain identifier.
    pub domain: Cow<'static, str>,
    /// Optional span or quantity in addressable units.
    pub quantity: Option<u16>,
}

impl AddressedAccessMeta {
    /// Creates addressed-access metadata for the given address in the given domain.
    pub fn new(address: u32, domain: Cow<'static, str>, quantity: Option<u16>) -> Self {
        Self {
            address,
            domain,
            quantity,
        }
    }
}

/// Lossless logical route carried by routed and bridge envelopes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Route<'a> {
    /// Resource-oriented route such as HTTP request/response.
    RequestResponse {
        /// Logical resource name targeted by the command or result.
        resource: Cow<'a, str>,
        /// Optional concrete transport path preserved alongside the logical resource.
        path: Option<Cow<'a, str>>,
    },
    /// Messaging-oriented route such as MQTT topic.
    Messaging {
        /// Topic or channel name used by the messaging transport.
        topic: Cow<'a, str>,
    },
    /// Addressed-access route such as a register, fieldbus offset, or memory-mapped point.
    AddressedAccess {
        /// Logical resource name associated with the addressed access.
        resource: Cow<'a, str>,
        /// Typed addressed-access metadata used by addressed-access codecs.
        access: AddressedAccessMeta,
        /// Optional protocol-scoped node identifier such as a Modbus unit id.
        node_id: Option<u32>,
    },
}

impl Route<'_> {
    /// Re-borrows this route without forcing ownership changes.
    pub fn as_borrowed(&self) -> Route<'_> {
        match self {
            Self::RequestResponse { resource, path } => Route::RequestResponse {
                resource: Cow::Borrowed(resource.as_ref()),
                path: path.as_ref().map(|path| Cow::Borrowed(path.as_ref())),
            },
            Self::Messaging { topic } => Route::Messaging {
                topic: Cow::Borrowed(topic.as_ref()),
            },
            Self::AddressedAccess {
                resource,
                access,
                node_id,
            } => Route::AddressedAccess {
                resource: Cow::Borrowed(resource.as_ref()),
                access: access.clone(),
                node_id: *node_id,
            },
        }
    }

    /// Materializes an owned route for long-lived boundaries.
    pub fn into_owned(self) -> Route<'static> {
        IntoOwned::into_owned(self)
    }
}

impl IntoOwned for Route<'_> {
    type Owned = Route<'static>;
    fn into_owned(self) -> Self::Owned {
        match self {
            Self::RequestResponse { resource, path } => Route::RequestResponse {
                resource: IntoOwned::into_owned(resource),
                path: path.into_owned(),
            },
            Self::Messaging { topic } => Route::Messaging {
                topic: IntoOwned::into_owned(topic),
            },
            Self::AddressedAccess {
                resource,
                access,
                node_id,
            } => Route::AddressedAccess {
                resource: IntoOwned::into_owned(resource),
                access,
                node_id,
            },
        }
    }
}

impl<'a> From<Address<'a>> for Route<'a> {
    fn from(address: Address<'a>) -> Self {
        match address {
            Address::Channel(topic) => Route::Messaging { topic },
            Address::Resource(resource) => Route::RequestResponse {
                resource,
                path: None,
            },
        }
    }
}
