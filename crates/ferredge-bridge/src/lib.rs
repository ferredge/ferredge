#![cfg_attr(not(feature = "std"), no_std)]

//! Shared bridge intermediate representation and semantic planners for ferredge.

extern crate alloc;

mod capability;
mod error;
mod fault;
mod message;
mod op;
pub mod planner;
mod traits;

/// Convenience re-exports for downstream bridge users.
pub mod prelude {
    pub use crate::capability::*;
    pub use crate::error::*;
    pub use crate::fault::*;
    pub use crate::message::*;
    pub use crate::op::*;
    pub use crate::traits::*;
    pub use ferredge_core::prelude::{
        AddressedAccessMeta, Correlation, Header, Headers, IntoOwned, MetaValue, PayloadValue,
        ProtocolId, Route, TransportMeta,
    };
}

pub use capability::*;
pub use error::*;
pub use fault::*;
pub use ferredge_core::prelude::{
    AddressedAccessMeta, Correlation, Header, Headers, IntoOwned, MetaValue, PayloadValue,
    ProtocolId, Route, TransportMeta,
};
pub use message::*;
pub use op::*;
pub use traits::*;

#[cfg(test)]
mod tests;
