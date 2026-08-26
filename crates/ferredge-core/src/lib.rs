#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;
#[cfg(test)]
extern crate std;

mod command;
mod device;
mod headers;
mod maybe;
mod meta;
#[cfg(feature = "net")]
mod net;
mod owned;
mod route;
mod routed;
mod router;
#[cfg(feature = "runtime")]
mod runtime;
#[cfg(feature = "serial")]
mod serial;
#[cfg(feature = "sync")]
mod sync;

pub mod prelude {
    pub use crate::command::*;
    pub use crate::device::*;
    pub use crate::headers::*;
    pub use crate::maybe::*;
    pub use crate::meta::*;
    #[cfg(feature = "net")]
    pub use crate::net::*;
    pub use crate::owned::*;
    pub use crate::route::*;
    pub use crate::routed::*;
    pub use crate::router::*;
    #[cfg(feature = "runtime")]
    pub use crate::runtime::*;
    #[cfg(feature = "serial")]
    pub use crate::serial::*;
    #[cfg(feature = "sync")]
    pub use crate::sync::*;
}
