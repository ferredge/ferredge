#[cfg(not(feature = "std"))]
extern crate alloc;

use core::{future::Future, ops::DerefMut};

use crate::maybe::{MaybeSend, MaybeSync};

#[cfg(feature = "std")]
pub use std::sync::Arc as Shared;

#[cfg(not(feature = "std"))]
pub use alloc::sync::Arc as Shared;

pub use core::sync::atomic::{AtomicBool as AtomicFlag, Ordering as AtomicOrdering};

/// Error returned by async mutex operations.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MutexError {
    /// Mutex is currently held by another task and try-lock could not acquire it.
    #[error("mutex is busy")]
    Busy,
    /// Locking failed because the underlying runtime is unavailable.
    #[error("runtime is unavailable")]
    RuntimeUnavailable,
}

/// Async mutex contract used by protocol crates without tying them to one executor.
pub trait AsyncMutex<T>: MaybeSend + MaybeSync
where
    T: MaybeSend + 'static,
{
    /// Guard type returned by mutex lock operations.
    type Guard<'a>: DerefMut<Target = T> + 'a
    where
        Self: 'a,
        T: 'a;

    /// Waits until the mutex can be acquired.
    fn lock(&self) -> impl Future<Output = Result<Self::Guard<'_>, MutexError>> + MaybeSend;

    /// Attempts to acquire the mutex without waiting.
    fn try_lock(&self) -> Result<Self::Guard<'_>, MutexError>;
}
