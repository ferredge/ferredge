//! Embassy adapters for the ferredge core runtime, network, and serial traits.
//!
//! Embassy is a single-executor async ecosystem: spawners are pinned to one executor,
//! network sockets borrow a shared stack, and none of it is `Send`. These adapters
//! therefore enable the `local` feature of `ferredge-core`, which erases the `Send`/`Sync`
//! bounds from the adapter traits. A build using this crate cannot also use the
//! tokio or async-std adapters.
//!
//! The crate is `no_std` (with `alloc`) by default; the `std` feature exists for host
//! builds and tests. On embedded targets the application must provide a `critical-section`
//! implementation and an `embassy-time` driver, exactly as with any embassy firmware.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

#[cfg(feature = "net")]
mod net;
#[cfg(feature = "runtime")]
mod runtime;
#[cfg(feature = "serial")]
mod serial;
#[cfg(all(test, feature = "runtime", feature = "std"))]
mod tests;
#[cfg(any(feature = "runtime", feature = "net"))]
mod time_util;

#[cfg(feature = "net")]
pub use net::*;
#[cfg(feature = "runtime")]
pub use runtime::*;
#[cfg(feature = "serial")]
pub use serial::*;
