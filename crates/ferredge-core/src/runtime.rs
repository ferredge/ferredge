use core::{future::Future, time::Duration};

use crate::sync::AsyncMutex;

/// Error returned by runtime-backed task handles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskJoinError {
    /// Task was cancelled or aborted before successful completion.
    Cancelled,
    /// Task failed because underlying runtime shut down unexpectedly.
    RuntimeUnavailable,
}

/// Error returned by runtime-backed channel operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelError {
    /// Channel is full and cannot currently accept more messages.
    Full,
    /// Channel currently has no item available for nonblocking receive.
    Empty,
    /// Channel was closed by the receiver or runtime.
    Closed,
    /// Channel operation failed because runtime resources are unavailable.
    RuntimeUnavailable,
}

/// Monotonic instant returned by a runtime clock.
///
/// Implementations should represent a monotonically increasing point in time suitable for
/// elapsed-duration checks used by protocol keepalive and timeout logic.
pub trait RuntimeInstant: Clone + Send + Sync + 'static {
    /// Returns the duration elapsed since this instant was captured.
    fn elapsed(&self) -> Duration;
}

/// Handle returned by runtime task spawning.
///
/// Concrete runtimes such as Tokio, async-std, or embassy can map this trait to their own
/// task-handle concepts while keeping protocol crates generic over the runtime.
pub trait TaskHandle<T>: Send
where
    T: Send + 'static,
{
    /// Waits for the task to finish and returns its output or runtime-level failure.
    fn join(&mut self) -> impl Future<Output = Result<T, TaskJoinError>> + Send;

    /// Requests cancellation of the task when supported by the runtime.
    fn abort(&self);

    /// Returns whether the task has already completed.
    fn is_finished(&self) -> bool;
}

/// Async sender half of a bounded runtime channel.
pub trait ChannelSender<T>: Clone + Send + Sync
where
    T: Send + 'static,
{
    /// Sends one item, waiting until capacity is available.
    fn send(&self, item: T) -> impl Future<Output = Result<(), ChannelError>> + Send;

    /// Attempts to send one item without waiting for capacity.
    fn try_send(&self, item: T) -> Result<(), ChannelError>;
}

/// Async receiver half of a bounded runtime channel.
pub trait ChannelReceiver<T>: Send
where
    T: Send + 'static,
{
    /// Receives the next item from the channel.
    fn recv(&mut self) -> impl Future<Output = Result<T, ChannelError>> + Send;

    /// Attempts to receive one item without waiting.
    fn try_recv(&mut self) -> Result<T, ChannelError>;
}

/// Generic async runtime contract shared by protocol adapters.
///
/// Implementations are intended for executor ecosystems such as Tokio, async-std, or embassy.
/// The trait focuses on the minimum orchestration pieces protocol drivers usually need:
/// task spawning, bounded channels, and timers.
pub trait AsyncRuntime: Clone + Send + Sync + 'static {
    /// Task handle returned by `spawn`.
    type Task<T>: TaskHandle<T>
    where
        T: Send + 'static;

    /// Bounded channel sender created by `channel`.
    type Sender<T>: ChannelSender<T>
    where
        T: Send + 'static;

    /// Bounded channel receiver created by `channel`.
    type Receiver<T>: ChannelReceiver<T>
    where
        T: Send + 'static;

    /// Async mutex type created by `mutex`.
    type Mutex<T>: AsyncMutex<T>
    where
        T: Send + 'static;

    /// Monotonic instant type returned by `now`.
    type Instant: RuntimeInstant;

    /// Spawns one background future onto the runtime.
    fn spawn<F>(&self, future: F) -> Self::Task<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static;

    /// Creates one bounded channel suitable for backpressure-aware event pipelines.
    fn channel<T>(&self, capacity: usize) -> (Self::Sender<T>, Self::Receiver<T>)
    where
        T: Send + 'static;

    /// Creates one async mutex backed by the selected runtime.
    fn mutex<T>(&self, value: T) -> Self::Mutex<T>
    where
        T: Send + 'static;

    /// Returns the current monotonic instant for elapsed-time checks.
    fn now(&self) -> Self::Instant;

    /// Suspends the current task for the requested duration.
    fn sleep(&self, duration: Duration) -> impl Future<Output = ()> + Send;
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::{
        pin::Pin,
        task::{Context, Poll, Waker},
    };

    #[derive(Clone, Default)]
    struct MockRuntime;

    struct MockTask<T>(Option<T>);

    impl<T> TaskHandle<T> for MockTask<T>
    where
        T: Send + 'static,
    {
        async fn join(&mut self) -> Result<T, TaskJoinError> {
            self.0.take().ok_or(TaskJoinError::Cancelled)
        }

        fn abort(&self) {}

        fn is_finished(&self) -> bool {
            self.0.is_some()
        }
    }

    struct MockSender<T>(core::marker::PhantomData<fn() -> T>);

    impl<T> Clone for MockSender<T> {
        fn clone(&self) -> Self {
            Self(core::marker::PhantomData)
        }
    }

    impl<T> Default for MockSender<T> {
        fn default() -> Self {
            Self(core::marker::PhantomData)
        }
    }

    impl<T> ChannelSender<T> for MockSender<T>
    where
        T: Send + 'static,
    {
        async fn send(&self, _item: T) -> Result<(), ChannelError> {
            Ok(())
        }

        fn try_send(&self, _item: T) -> Result<(), ChannelError> {
            Ok(())
        }
    }

    struct MockReceiver<T>(core::marker::PhantomData<fn() -> T>);
    struct MockMutex<T>(std::sync::Mutex<T>);
    struct MockGuard<'a, T>(std::sync::MutexGuard<'a, T>);
    #[derive(Clone)]
    struct MockInstant(std::time::Instant);

    impl<T> core::ops::Deref for MockGuard<'_, T> {
        type Target = T;

        fn deref(&self) -> &Self::Target {
            &self.0
        }
    }

    impl<T> core::ops::DerefMut for MockGuard<'_, T> {
        fn deref_mut(&mut self) -> &mut Self::Target {
            &mut self.0
        }
    }

    impl<T> Default for MockReceiver<T> {
        fn default() -> Self {
            Self(core::marker::PhantomData)
        }
    }

    impl<T> ChannelReceiver<T> for MockReceiver<T>
    where
        T: Send + 'static,
    {
        async fn recv(&mut self) -> Result<T, ChannelError> {
            Err(ChannelError::Closed)
        }

        fn try_recv(&mut self) -> Result<T, ChannelError> {
            Err(ChannelError::Closed)
        }
    }

    impl<T> AsyncMutex<T> for MockMutex<T>
    where
        T: Send + 'static,
    {
        type Guard<'a>
            = MockGuard<'a, T>
        where
            T: 'a;

        async fn lock(&self) -> Result<Self::Guard<'_>, crate::sync::MutexError> {
            self.0
                .lock()
                .map(MockGuard)
                .map_err(|_| crate::sync::MutexError::RuntimeUnavailable)
        }

        fn try_lock(&self) -> Result<Self::Guard<'_>, crate::sync::MutexError> {
            self.0
                .try_lock()
                .map(MockGuard)
                .map_err(|_| crate::sync::MutexError::Busy)
        }
    }

    impl RuntimeInstant for MockInstant {
        fn elapsed(&self) -> Duration {
            self.0.elapsed()
        }
    }

    impl AsyncRuntime for MockRuntime {
        type Task<T>
            = MockTask<T>
        where
            T: Send + 'static;
        type Sender<T>
            = MockSender<T>
        where
            T: Send + 'static;
        type Receiver<T>
            = MockReceiver<T>
        where
            T: Send + 'static;
        type Mutex<T>
            = MockMutex<T>
        where
            T: Send + 'static;
        type Instant = MockInstant;

        fn spawn<F>(&self, _future: F) -> Self::Task<F::Output>
        where
            F: Future + Send + 'static,
            F::Output: Send + 'static,
        {
            MockTask(None)
        }

        fn channel<T>(&self, _capacity: usize) -> (Self::Sender<T>, Self::Receiver<T>)
        where
            T: Send + 'static,
        {
            (MockSender::default(), MockReceiver::default())
        }

        fn mutex<T>(&self, value: T) -> Self::Mutex<T>
        where
            T: Send + 'static,
        {
            MockMutex(std::sync::Mutex::new(value))
        }

        fn now(&self) -> Self::Instant {
            MockInstant(std::time::Instant::now())
        }

        async fn sleep(&self, _duration: Duration) {}
    }

    fn block_on<F: Future>(future: F) -> F::Output {
        let waker = Waker::noop();
        let mut context = Context::from_waker(waker);
        let mut future = Pin::from(Box::new(future));
        loop {
            match future.as_mut().poll(&mut context) {
                Poll::Ready(output) => return output,
                Poll::Pending => core::hint::spin_loop(),
            }
        }
    }

    #[test]
    fn mock_runtime_contract_is_usable() {
        let runtime = MockRuntime;
        let (sender, mut receiver) = runtime.channel::<u8>(4);
        let _task = runtime.spawn(async { 42u8 });

        assert!(block_on(sender.send(1)).is_ok());
        assert!(sender.try_send(2).is_ok());
        assert_eq!(block_on(receiver.recv()), Err(ChannelError::Closed));
        block_on(runtime.sleep(Duration::from_millis(0)));
    }
}
