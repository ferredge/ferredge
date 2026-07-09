//! Feature-gated thread-safety bounds for runtime, net, and serial traits.
//!
//! Multithreaded runtimes (Tokio, async-std) move tasks between worker threads, so the
//! adapter traits normally require `Send`/`Sync`. Single-executor runtimes such as embassy
//! are deliberately `!Send` (sockets borrow a `RefCell` network stack, spawners are pinned
//! to one executor). The `local` feature erases the thread-safety requirements so those
//! runtimes can implement the same traits.
//!
//! A build that enables `local` cannot use the multithreaded adapters: their
//! implementations rely on the `Send`/`Sync` guarantees that `local` removes. Enable it
//! only through a single-executor adapter crate such as `ferredge-runtime-embassy`.

/// Alias for `Send` unless the `local` feature erases the requirement.
#[cfg(not(feature = "local"))]
pub trait MaybeSend: Send {}
#[cfg(not(feature = "local"))]
impl<T: Send + ?Sized> MaybeSend for T {}

/// Alias for `Send` unless the `local` feature erases the requirement.
#[cfg(feature = "local")]
pub trait MaybeSend {}
#[cfg(feature = "local")]
impl<T: ?Sized> MaybeSend for T {}

/// Alias for `Sync` unless the `local` feature erases the requirement.
#[cfg(not(feature = "local"))]
pub trait MaybeSync: Sync {}
#[cfg(not(feature = "local"))]
impl<T: Sync + ?Sized> MaybeSync for T {}

/// Alias for `Sync` unless the `local` feature erases the requirement.
#[cfg(feature = "local")]
pub trait MaybeSync {}
#[cfg(feature = "local")]
impl<T: ?Sized> MaybeSync for T {}
