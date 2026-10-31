//! Zenoh protocol adapter for ferredge.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

use alloc::string::String;
use ferredge_core::prelude::*;

pub use endpoint::{ZenohDevice, ZenohEndpointConfig};

mod attributes;
mod convert;
mod endpoint;
mod runtime;
mod types;

#[cfg(any(
    all(feature = "tokio-runtime", feature = "async-std-runtime"),
    all(feature = "tokio-runtime", feature = "embassy-runtime"),
    all(feature = "async-std-runtime", feature = "embassy-runtime")
))]
compile_error!("ferredge-proto-zenoh supports only one runtime stack feature at a time");

#[cfg(feature = "tokio-runtime")]
mod runtime_stack {
    pub use ferredge_runtime_tokio::TokioRuntime as StackRuntime;
}

#[cfg(feature = "async-std-runtime")]
mod runtime_stack {
    pub use ferredge_runtime_async_std::AsyncStdRuntime as StackRuntime;
}

#[cfg(feature = "embassy-runtime")]
mod runtime_stack {
    pub use ferredge_runtime_embassy::{
        EmbassyNet as StackNet, EmbassyRuntime as StackRuntime, EmbassyTask as RuntimeTask,
    };
}

#[cfg(not(any(
    feature = "tokio-runtime",
    feature = "async-std-runtime",
    feature = "embassy-runtime"
)))]
compile_error!("ferredge-proto-zenoh requires a supported runtime stack feature to be enabled");

#[cfg(any(feature = "std", feature = "no_std"))]
use runtime_stack::StackRuntime;

#[cfg(feature = "std")]
type RuntimeSender<T> = <StackRuntime as AsyncRuntime>::Sender<T>;

#[cfg(any(feature = "std", feature = "no_std"))]
type RuntimeReceiver<T> = <StackRuntime as AsyncRuntime>::Receiver<T>;

#[cfg(any(feature = "std", feature = "no_std"))]
type RuntimeMutex<T> = <StackRuntime as AsyncRuntime>::Mutex<T>;

#[cfg(feature = "no_std")]
use runtime_stack::{RuntimeTask, StackNet};

#[cfg(test)]
mod tests;

use types::{ZenohNativePlan, ZenohRequest};

pub use convert::ZenohBridgeCodec;
pub use types::{
    ZenohCommandConversionError, ZenohError, ZenohEvent, ZenohOperation, ZenohPublishRequest,
    ZenohQueryRequest, ZenohReply, ZenohResponse, ZenohSubscription,
};

#[cfg(any(feature = "tokio-runtime", feature = "async-std-runtime"))]
use runtime::ZenohSessionState;

/// Current state of the Zenoh background listener runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZenohListenerStatus {
    /// Listener is not running and no failure is recorded.
    Stopped,
    /// Listener is actively polling the Zenoh session.
    Running,
    /// Listener stopped after a runtime or sink failure.
    Failed(String),
}

/// Zenoh adapter backed by the `zenoh` library (std) or `zenoh-nostd` (no_std).
///
/// In `std` builds this type owns the live Zenoh session and background listener runtime.
/// In `no_std` builds it owns the zenoh-nostd session and network stack (runtime-specific).
/// In builds without runtime support it remains useful for packet conversion, but transport
/// operations fail with explicit runtime-availability errors.
#[derive(Clone)]
pub struct ZenohDriver {
    /// Device metadata and Zenoh configuration served by this driver.
    pub dvc: ZenohDevice,

    // std build fields - uses standard zenoh library
    #[cfg(feature = "std")]
    runtime: StackRuntime,

    #[cfg(feature = "std")]
    session: Shared<RuntimeMutex<Option<ZenohSessionState>>>,

    #[cfg(feature = "std")]
    listener_status: Shared<RuntimeMutex<ZenohListenerStatus>>,

    #[cfg(feature = "std")]
    listener_stop_tx: Shared<RuntimeMutex<Option<RuntimeSender<()>>>>,

    #[cfg(feature = "no_std")]
    runtime: StackRuntime,

    #[cfg(feature = "no_std")]
    net: StackNet,

    #[cfg(feature = "no_std")]
    session: Shared<RuntimeMutex<Option<runtime::ZenohNoStdSessionState>>>,

    #[cfg(feature = "no_std")]
    listener_status: Shared<RuntimeMutex<ZenohListenerStatus>>,

    #[cfg(feature = "no_std")]
    listener_handle: Shared<RuntimeMutex<Option<RuntimeTask<()>>>>,
}

impl ZenohDriver {
    /// Creates a new Zenoh driver for the given device.
    ///
    /// Uses the ambient runtime stack, so it is only available on the std runtimes; the
    /// embassy stack has no ambient executor or network stack — construct with
    /// [`ZenohDriver::with_stack`] there.
    #[cfg(any(feature = "tokio-runtime", feature = "async-std-runtime"))]
    pub fn new(dvc: ZenohDevice) -> Self {
        Self::with_stack(dvc, StackRuntime::default())
    }

    /// Creates a new Zenoh driver for std runtimes with explicit runtime.
    #[cfg(feature = "std")]
    pub fn with_stack(dvc: ZenohDevice, runtime: StackRuntime) -> Self {
        Self {
            dvc,
            runtime: runtime.clone(),
            session: Shared::new(runtime.mutex(None)),
            listener_status: Shared::new(runtime.mutex(ZenohListenerStatus::Stopped)),
            listener_stop_tx: Shared::new(runtime.mutex(None)),
        }
    }

    /// Creates a new Zenoh driver for embassy runtime with explicit runtime and net stack.
    #[cfg(feature = "embassy-runtime")]
    pub fn with_stack(dvc: ZenohDevice, runtime: StackRuntime, net: StackNet) -> Self {
        Self {
            dvc,
            runtime: runtime.clone(),
            net,
            session: Shared::new(runtime.mutex(None)),
            listener_status: Shared::new(runtime.mutex(ZenohListenerStatus::Stopped)),
            listener_handle: Shared::new(runtime.mutex(None)),
        }
    }

    /// Creates a new Zenoh driver for the given device (stub when no runtime available).
    #[cfg(not(any(feature = "std", feature = "no_std")))]
    pub fn new(dvc: ZenohDevice) -> Self {
        Self { dvc }
    }

    /// Convert a Command to a ZenohNativePlan.
    pub fn plan_native(
        &self,
        command: &Command,
    ) -> Result<ZenohNativePlan<'static>, ZenohCommandConversionError> {
        convert::plan_from_command(command, &self.dvc)
    }
}

#[cfg(feature = "std")]
impl Lifecycle for ZenohDriver {
    type Error = ZenohError;

    async fn start(&self) -> Result<(), Self::Error> {
        runtime::start_session(self).await
    }

    async fn stop(&self) -> Result<(), Self::Error> {
        runtime::stop_session(self).await
    }
}

#[cfg(feature = "std")]
impl PubSub for ZenohDriver {
    type PublishRequest = ZenohPublishRequest;
    type Subscription = ZenohSubscription;
    type Error = ZenohError;

    async fn publish(&self, request: Self::PublishRequest) -> Result<(), Self::Error> {
        runtime::publish(self, request).await
    }

    async fn subscribe<S>(
        &self,
        subscription: Self::Subscription,
        sink: S,
    ) -> Result<(), Self::Error>
    where
        S: EventSink<Event = RoutedEvent<'static>> + Send,
    {
        runtime::subscribe(self, subscription, sink).await
    }

    async fn unsubscribe(&self, subscription: Self::Subscription) -> Result<(), Self::Error> {
        runtime::unsubscribe(self, subscription).await
    }
}

#[cfg(feature = "std")]
impl RequestResponse for ZenohDriver {
    type Request = ZenohRequest;
    type Response = ZenohResponse;
    type Error = ZenohError;

    async fn execute(&self, request: Self::Request) -> Result<Self::Response, Self::Error> {
        runtime::execute_request(self, request).await
    }
}

#[cfg(feature = "std")]
impl EventSource for ZenohDriver {
    type Event = ZenohEvent;
    type Error = ZenohError;

    async fn start_listening<S>(&self, sink: S) -> Result<(), Self::Error>
    where
        S: EventSink<Event = Self::Event> + Send + 'static,
    {
        runtime::start_listener(self, sink).await
    }

    async fn stop_listening(&self) -> Result<(), Self::Error> {
        runtime::stop_listener(self).await
    }
}

// Embassy runtime builds
#[cfg(feature = "embassy-runtime")]
impl Lifecycle for ZenohDriver {
    type Error = ZenohError;

    async fn start(&self) -> Result<(), Self::Error> {
        runtime::start_session(self).await
    }

    async fn stop(&self) -> Result<(), Self::Error> {
        runtime::stop_session(self).await
    }
}

// Builds without std or no_std support - return errors for runtime operations
#[cfg(not(any(feature = "std", feature = "no_std")))]
impl Lifecycle for ZenohDriver {
    type Error = ZenohError;

    async fn start(&self) -> Result<(), Self::Error> {
        Err(ZenohError::RuntimeNotAvailable)
    }

    async fn stop(&self) -> Result<(), Self::Error> {
        Err(ZenohError::RuntimeNotAvailable)
    }
}

#[cfg(feature = "embassy-runtime")]
impl PubSub for ZenohDriver {
    type PublishRequest = ZenohPublishRequest;
    type Subscription = ZenohSubscription;
    type Error = ZenohError;

    async fn publish(&self, request: Self::PublishRequest) -> Result<(), Self::Error> {
        runtime::publish(self, request).await
    }

    async fn subscribe<S>(
        &self,
        subscription: Self::Subscription,
        sink: S,
    ) -> Result<(), Self::Error>
    where
        S: EventSink<Event = RoutedEvent<'static>>,
    {
        runtime::subscribe(self, subscription, sink).await
    }

    async fn unsubscribe(&self, subscription: Self::Subscription) -> Result<(), Self::Error> {
        runtime::unsubscribe(self, &subscription.key_expr).await
    }
}

#[cfg(not(any(feature = "std", feature = "embassy-runtime")))]
impl PubSub for ZenohDriver {
    type PublishRequest = ZenohPublishRequest;
    type Subscription = ZenohSubscription;
    type Error = ZenohError;

    async fn publish(&self, _request: Self::PublishRequest) -> Result<(), Self::Error> {
        Err(ZenohError::RuntimeNotAvailable)
    }

    async fn subscribe<S>(
        &self,
        _subscription: Self::Subscription,
        _sink: S,
    ) -> Result<(), Self::Error>
    where
        S: EventSink<Event = RoutedEvent<'static>> + Send,
    {
        Err(ZenohError::RuntimeNotAvailable)
    }

    async fn unsubscribe(&self, _subscription: Self::Subscription) -> Result<(), Self::Error> {
        Err(ZenohError::RuntimeNotAvailable)
    }
}

#[cfg(feature = "embassy-runtime")]
impl RequestResponse for ZenohDriver {
    type Request = ZenohRequest;
    type Response = ZenohResponse;
    type Error = ZenohError;

    async fn execute(&self, request: Self::Request) -> Result<Self::Response, Self::Error> {
        runtime::execute_request(self, request).await
    }
}

#[cfg(not(any(feature = "std", feature = "no_std")))]
impl RequestResponse for ZenohDriver {
    type Request = ZenohRequest;
    type Response = ZenohResponse;
    type Error = ZenohError;

    async fn execute(&self, _request: Self::Request) -> Result<Self::Response, Self::Error> {
        Err(ZenohError::RuntimeNotAvailable)
    }
}

#[cfg(feature = "embassy-runtime")]
impl EventSource for ZenohDriver {
    type Event = ZenohEvent;
    type Error = ZenohError;

    async fn start_listening<S>(&self, sink: S) -> Result<(), Self::Error>
    where
        S: EventSink<Event = Self::Event>,
    {
        runtime::start_listener(self, sink).await
    }

    async fn stop_listening(&self) -> Result<(), Self::Error> {
        runtime::stop_listener(self).await
    }
}

#[cfg(not(any(feature = "std", feature = "no_std")))]
impl EventSource for ZenohDriver {
    type Event = ZenohEvent;
    type Error = ZenohError;

    async fn start_listening<S>(&self, _sink: S) -> Result<(), Self::Error>
    where
        S: EventSink<Event = Self::Event> + Send + 'static,
    {
        Err(ZenohError::RuntimeNotAvailable)
    }

    async fn stop_listening(&self) -> Result<(), Self::Error> {
        Err(ZenohError::RuntimeNotAvailable)
    }
}
