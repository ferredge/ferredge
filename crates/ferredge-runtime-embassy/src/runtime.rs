use alloc::{boxed::Box, collections::BTreeMap, vec::Vec};
use core::any::TypeId;
use core::cell::RefCell;
use core::future::Future;
use core::time::Duration;

use embassy_executor::raw::TaskStorage;
use embassy_executor::{SpawnError, Spawner};
use embassy_futures::select::{Either, select};
use embassy_sync::blocking_mutex::Mutex as BlockingMutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::{Channel, TryReceiveError, TrySendError};
use embassy_sync::mutex::{Mutex, MutexGuard, TryLockError};
use embassy_sync::signal::Signal;
use ferredge_core::prelude::*;

use crate::time_util::to_embassy_duration;

/// Embassy tasks live in `&'static` [`TaskStorage`] slots, so dynamically spawned futures
/// need leaked storage. Leaked slots are recorded here keyed by the future's `TypeId` and
/// reused once their task finishes ([`TaskStorage::spawn`] fails while the slot is busy),
/// bounding memory by the peak number of concurrent tasks per future type.
type StoragePool = BlockingMutex<CriticalSectionRawMutex, RefCell<BTreeMap<TypeId, Vec<usize>>>>;

struct RuntimeInner {
    spawner: Spawner,
    storage_pool: StoragePool,
}

/// Embassy-backed [`AsyncRuntime`].
///
/// Construct it in `main` from the executor's [`Spawner`]; there is no ambient executor to
/// grab, so the runtime has no `Default`. `CHANNEL_CAPACITY` fixes the buffer size of every
/// channel created through [`AsyncRuntime::channel`] (embassy channels are compile-time
/// sized); the runtime `capacity` argument must not exceed it.
pub struct EmbassyRuntime<const CHANNEL_CAPACITY: usize = 16> {
    inner: Shared<RuntimeInner>,
}

impl<const CHANNEL_CAPACITY: usize> Clone for EmbassyRuntime<CHANNEL_CAPACITY> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

pub struct EmbassyTask<T> {
    result: Shared<Signal<CriticalSectionRawMutex, Result<T, TaskJoinError>>>,
    abort: Shared<Signal<CriticalSectionRawMutex, ()>>,
    finished: Shared<AtomicFlag>,
    taken: bool,
}

pub struct EmbassySender<T: 'static, const N: usize> {
    inner: Shared<Channel<CriticalSectionRawMutex, T, N>>,
}

pub struct EmbassyReceiver<T: 'static, const N: usize> {
    inner: Shared<Channel<CriticalSectionRawMutex, T, N>>,
}

pub struct EmbassyMutex<T> {
    inner: Mutex<CriticalSectionRawMutex, T>,
}

pub struct EmbassyMutexGuard<'a, T> {
    inner: MutexGuard<'a, CriticalSectionRawMutex, T>,
}

#[derive(Clone)]
pub struct EmbassyInstant {
    inner: embassy_time::Instant,
}

impl<T, const N: usize> Clone for EmbassySender<T, N> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<const CHANNEL_CAPACITY: usize> EmbassyRuntime<CHANNEL_CAPACITY> {
    // `Shared` is the crate-wide Arc alias; the runtime is !Send because `Spawner` is
    // pinned to one executor, which is exactly what ferredge-core's `local` feature allows.
    #[allow(clippy::arc_with_non_send_sync)]
    pub fn new(spawner: Spawner) -> Self {
        Self {
            inner: Shared::new(RuntimeInner {
                spawner,
                storage_pool: BlockingMutex::new(RefCell::new(BTreeMap::new())),
            }),
        }
    }

    fn spawn_wrapped<W>(&self, make: impl FnOnce() -> W) -> Result<(), SpawnError>
    where
        W: Future + 'static,
    {
        let type_id = TypeId::of::<W>();
        // `TaskStorage::spawn` only constructs the future on success, so `make` survives
        // attempts on slots that are still running.
        let mut make = Some(make);
        let candidates = self
            .inner
            .storage_pool
            .lock(|pool| pool.borrow().get(&type_id).cloned().unwrap_or_default());
        for pointer in candidates {
            // SAFETY: the pool entry was stored under `TypeId::of::<W>()`, so the pointer
            // was leaked from a `Box<TaskStorage<W>>` and stays valid forever.
            let storage = unsafe { &*(pointer as *const TaskStorage<W>) };
            if let Ok(token) = storage.spawn(|| (make.take().expect("make reused"))()) {
                self.inner.spawner.spawn(token);
                return Ok(());
            }
        }

        let storage: &'static TaskStorage<W> = Box::leak(Box::new(TaskStorage::new()));
        self.inner.storage_pool.lock(|pool| {
            pool.borrow_mut()
                .entry(type_id)
                .or_default()
                .push(storage as *const TaskStorage<W> as usize);
        });
        let token = storage.spawn(|| (make.take().expect("make reused"))())?;
        self.inner.spawner.spawn(token);
        Ok(())
    }
}

impl<T> TaskHandle<T> for EmbassyTask<T>
where
    T: MaybeSend + 'static,
{
    async fn join(&mut self) -> Result<T, TaskJoinError> {
        if self.taken {
            return Err(TaskJoinError::Cancelled);
        }
        let result = self.result.wait().await;
        self.taken = true;
        result
    }

    fn abort(&self) {
        self.abort.signal(());
    }

    fn is_finished(&self) -> bool {
        self.finished.load(AtomicOrdering::Acquire)
    }
}

impl<T, const N: usize> ChannelSender<T> for EmbassySender<T, N>
where
    T: MaybeSend + 'static,
{
    async fn send(&self, item: T) -> Result<(), ChannelError> {
        self.inner.send(item).await;
        Ok(())
    }

    fn try_send(&self, item: T) -> Result<(), ChannelError> {
        match self.inner.try_send(item) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err(ChannelError::Full),
        }
    }
}

impl<T, const N: usize> ChannelReceiver<T> for EmbassyReceiver<T, N>
where
    T: MaybeSend + 'static,
{
    async fn recv(&mut self) -> Result<T, ChannelError> {
        Ok(self.inner.receive().await)
    }

    fn try_recv(&mut self) -> Result<T, ChannelError> {
        match self.inner.try_receive() {
            Ok(item) => Ok(item),
            Err(TryReceiveError::Empty) => Err(ChannelError::Empty),
        }
    }
}

impl<T> core::ops::Deref for EmbassyMutexGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<T> core::ops::DerefMut for EmbassyMutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl<T> AsyncMutex<T> for EmbassyMutex<T>
where
    T: MaybeSend + 'static,
{
    type Guard<'a>
        = EmbassyMutexGuard<'a, T>
    where
        T: 'a;

    async fn lock(&self) -> Result<Self::Guard<'_>, MutexError> {
        Ok(EmbassyMutexGuard {
            inner: self.inner.lock().await,
        })
    }

    fn try_lock(&self) -> Result<Self::Guard<'_>, MutexError> {
        self.inner
            .try_lock()
            .map(|inner| EmbassyMutexGuard { inner })
            .map_err(|TryLockError| MutexError::Busy)
    }
}

impl RuntimeInstant for EmbassyInstant {
    fn elapsed(&self) -> Duration {
        Duration::from_micros(self.inner.elapsed().as_micros())
    }
}

impl<const CHANNEL_CAPACITY: usize> AsyncRuntime for EmbassyRuntime<CHANNEL_CAPACITY> {
    type Task<T>
        = EmbassyTask<T>
    where
        T: MaybeSend + 'static;
    type Sender<T>
        = EmbassySender<T, CHANNEL_CAPACITY>
    where
        T: MaybeSend + 'static;
    type Receiver<T>
        = EmbassyReceiver<T, CHANNEL_CAPACITY>
    where
        T: MaybeSend + 'static;
    type Mutex<T>
        = EmbassyMutex<T>
    where
        T: MaybeSend + 'static;
    type Instant = EmbassyInstant;

    fn spawn<F>(&self, future: F) -> Self::Task<F::Output>
    where
        F: Future + MaybeSend + 'static,
        F::Output: MaybeSend + 'static,
    {
        let result = Shared::new(Signal::new());
        let abort = Shared::new(Signal::new());
        let finished = Shared::new(AtomicFlag::new(false));
        let task = EmbassyTask {
            result: result.clone(),
            abort: abort.clone(),
            finished: finished.clone(),
            taken: false,
        };

        let spawned = self.spawn_wrapped(move || async move {
            let outcome = match select(future, abort.wait()).await {
                Either::First(value) => Ok(value),
                Either::Second(()) => Err(TaskJoinError::Cancelled),
            };
            finished.store(true, AtomicOrdering::Release);
            result.signal(outcome);
        });
        if spawned.is_err() {
            task.finished.store(true, AtomicOrdering::Release);
            task.result.signal(Err(TaskJoinError::RuntimeUnavailable));
        }
        task
    }

    fn channel<T>(&self, capacity: usize) -> (Self::Sender<T>, Self::Receiver<T>)
    where
        T: MaybeSend + 'static,
    {
        // Embassy channels are compile-time sized; every channel has CHANNEL_CAPACITY slots.
        debug_assert!(
            capacity <= CHANNEL_CAPACITY,
            "requested channel capacity exceeds CHANNEL_CAPACITY"
        );
        let _ = capacity;
        let channel = Shared::new(Channel::new());
        (
            EmbassySender {
                inner: channel.clone(),
            },
            EmbassyReceiver { inner: channel },
        )
    }

    fn mutex<T>(&self, value: T) -> Self::Mutex<T>
    where
        T: MaybeSend + 'static,
    {
        EmbassyMutex {
            inner: Mutex::new(value),
        }
    }

    fn now(&self) -> Self::Instant {
        EmbassyInstant {
            inner: embassy_time::Instant::now(),
        }
    }

    async fn sleep(&self, duration: Duration) {
        embassy_time::Timer::after(to_embassy_duration(duration)).await;
    }
}
