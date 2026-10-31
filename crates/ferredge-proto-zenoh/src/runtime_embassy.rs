//! Embassy runtime implementation for Zenoh using zenoh-nostd.
//!
//! This module provides no_std-compatible Zenoh session management for embassy runtimes.
//! It uses the zenoh-nostd crate which has a fundamentally different API than the std zenoh:
//! - Session creation via `zenoh::connect!()` or `zenoh::listen!()` macros
//! - Continuous `.run()` task required in background
//! - Channel-based pub/sub instead of callback-based
//! - All operations are async with builder patterns


use alloc::{
    string::{String, ToString},
    vec::Vec,
};

use ferredge_core::prelude::*;

use crate::{
    runtime_stack::RuntimeTask,
    types::{
        ZenohError, ZenohEvent, ZenohPublishRequest,
        ZenohRequest, ZenohResponse, ZenohSubscription,
    },
    ZenohDriver, ZenohListenerStatus,
};

/// Subscription handle with forwarding task for zenoh-nostd channel-based subscriptions.
pub(crate) struct SubscriptionHandle {
    /// Key expression being subscribed to
    pub key_expr: String,
    /// Background task that forwards messages from subscription channel to event sink
    pub forward_task: Option<RuntimeTask<()>>,
}

/// Session state wrapper for zenoh-nostd.
///
/// Holds the zenoh-nostd session, background run task, and active subscriptions.
pub(crate) struct ZenohNoStdSessionState {
    /// The zenoh-nostd session handle
    /// Note: We use a type-erased approach since the actual Config type depends on platform
    #[cfg(feature = "no_std")]
    #[allow(dead_code)]
    session: &'static dyn core::any::Any,

    #[cfg(not(feature = "no_std"))]
    #[allow(dead_code)]
    session_placeholder: (),

    /// Background task running the continuous .run() loop required by zenoh-nostd
    pub run_task: Option<RuntimeTask<()>>,

    /// Active subscriptions with their forwarding tasks
    /// Each subscription has its own channel and task in zenoh-nostd architecture
    pub subscriptions: Vec<SubscriptionHandle>,
}

/// Starts a Zenoh session using zenoh-nostd.
///
/// **Platform-Specific Implementation Required**
///
/// zenoh-nostd requires a config type that implements `ZSessionConfig` which is
/// platform-specific (ESP32, STM32, etc.). The config must provide:
/// - Link manager for the network stack
/// - Buffer allocation strategy
/// - Transport configuration
///
/// ## Example Implementation (when platform config is defined):
///
/// ```rust,ignore
/// // 1. Define platform config (e.g., Esp32Config, Stm32Config)
/// pub struct FerredgeZenohConfig {
///     transports: TransportLinkManager<LinkManager>,
///     network: &'static EmbassyNet,
/// }
///
/// impl ZSessionConfig for FerredgeZenohConfig { /* ... */ }
///
/// // 2. Create session
/// let config = FerredgeZenohConfig::from_net(&driver.net);
/// let session = zenoh::connect!(FerredgeZenohConfig: config, endpoint)?;
///
/// // 3. Spawn .run() task
/// let session_ref: &'static _ = /* leak or static cell */;
/// let run_task = driver.runtime.spawn(async move {
///     session_ref.run().await
/// });
/// ```
pub(crate) async fn start_session(driver: &ZenohDriver) -> Result<(), ZenohError> {
    let session_guard = driver
        .session
        .lock()
        .await
        .map_err(|e| ZenohError::Other(e.to_string()))?;

    // Check if session is already started
    if session_guard.is_some() {
        return Ok(());
    }

    // Platform-specific zenoh-nostd integration required here
    // See function documentation for implementation template

    Err(ZenohError::Other(
        "zenoh-nostd requires platform-specific config (see start_session docs)".to_string(),
    ))
}

/// Stops the Zenoh session.
///
/// This function:
/// 1. Stops the listener first
/// 2. Cancels all subscription forwarding tasks
/// 3. Stops the background .run() task
/// 4. Closes the session
pub(crate) async fn stop_session(driver: &ZenohDriver) -> Result<(), ZenohError> {
    // Stop listener first
    stop_listener(driver).await?;

    let mut session_guard = driver
        .session
        .lock()
        .await
        .map_err(|e| ZenohError::Other(e.to_string()))?;

    if let Some(mut state) = session_guard.take() {
        // Clean up all subscription forwarding tasks
        for mut sub_handle in state.subscriptions.drain(..) {
            // Cancel forwarding task if present
            if let Some(task) = sub_handle.forward_task.take() {
                task.abort();
            }
            // When zenoh-nostd is integrated:
            // subscriber.undeclare().await.ok();
        }

        // Stop the continuous .run() task
        if let Some(run_task) = state.run_task.take() {
            run_task.abort();
        }

        // Session will be dropped automatically
        // When zenoh-nostd is integrated:
        // state.session.close().await.ok();
    }

    Ok(())
}

/// Publishes data to a Zenoh key expression.
///
/// Uses the zenoh-nostd builder pattern with .finish() terminator.
///
/// ## zenoh-nostd API:
/// ```rust,ignore
/// session.put(key_expr, payload).finish().await?
/// ```
pub(crate) async fn publish(
    driver: &ZenohDriver,
    _request: ZenohPublishRequest,
) -> Result<(), ZenohError> {
    let _session_guard = driver
        .session
        .lock()
        .await
        .map_err(|e| ZenohError::Other(e.to_string()))?;

    let _state = _session_guard
        .as_ref()
        .ok_or(ZenohError::SessionNotInitialized)?;

    #[cfg(feature = "no_std")]
    {
        // When platform-specific session is available:
        // let session: &Session<_> = /* downcast from state.session */;
        // session
        //     .put(request.key_expr.as_str(), &request.payload)
        //     .finish()
        //     .await
        //     .map_err(|e| ZenohError::Io(e.to_string()))?;

        Err(ZenohError::Other(
            "zenoh-nostd publish requires platform-specific session (see publish docs)".to_string(),
        ))
    }

    #[cfg(not(feature = "no_std"))]
    Err(ZenohError::RuntimeNotAvailable)
}

/// Subscribes to a Zenoh key expression.
///
/// ## zenoh-nostd Channel-Based Architecture
///
/// Unlike std Zenoh's FIFO handler, zenoh-nostd uses channels:
///
/// ```rust,ignore
/// // 1. Create static channel (embassy_sync::channel::Channel)
/// use static_cell::StaticCell;
/// static CHANNEL: StaticCell<Channel<Mutex, Sample, 8>> = StaticCell::new();
/// let channel = CHANNEL.init(Channel::new());
///
/// // 2. Declare subscriber with channel
/// let subscriber = session
///     .declare_subscriber("key/expr")?
///     .channel(channel.sender(), channel.receiver())
///     .finish()
///     .await?;
///
/// // 3. Forward samples in background task
/// spawner.spawn(async move {
///     while let Some(sample) = receiver.recv().await {
///         // Forward to event sink
///     }
/// });
/// ```
///
/// **Implementation Note**: Requires static channel allocation strategy
/// (pool or per-subscription static cells).
pub(crate) async fn subscribe<S>(
    driver: &ZenohDriver,
    subscription: ZenohSubscription,
    mut _sink: S,
) -> Result<(), ZenohError>
where
    S: EventSink<Event = RoutedEvent<'static>>,
{
    let mut session_guard = driver
        .session
        .lock()
        .await
        .map_err(|e| ZenohError::Other(e.to_string()))?;

    let state = session_guard
        .as_mut()
        .ok_or(ZenohError::SessionNotInitialized)?;

    // Check if already subscribed
    if state
        .subscriptions
        .iter()
        .any(|s| s.key_expr == subscription.key_expr)
    {
        return Ok(());
    }

    // Implementation requires:
    // 1. Static channel allocation (see function docs)
    // 2. Platform-specific session type with declare_subscriber()
    // 3. Background task spawning with embassy executor

    Err(ZenohError::Other(
        "zenoh-nostd subscribe requires static channels (see subscribe docs)".to_string(),
    ))
}

/// Unsubscribes from a Zenoh key expression.
///
/// Removes the subscription and cancels its forwarding task.
pub(crate) async fn unsubscribe(
    driver: &ZenohDriver,
    key_expr: &str,
) -> Result<(), ZenohError> {
    let mut session_guard = driver
        .session
        .lock()
        .await
        .map_err(|e| ZenohError::Other(e.to_string()))?;

    let state = session_guard
        .as_mut()
        .ok_or(ZenohError::SessionNotInitialized)?;

    // Find and remove the subscription
    if let Some(pos) = state.subscriptions.iter().position(|s| s.key_expr == key_expr) {
        let mut sub_handle = state.subscriptions.remove(pos);

        // TODO: When zenoh-nostd is available:
        // 1. Undeclare the subscriber (best effort):
        //    subscriber.undeclare().await.ok();
        //
        // 2. Cancel the forwarding task:
        //    if let Some(task) = sub_handle.forward_task.take() {
        //        task.cancel().await;
        //    }

        // Clear task handle (placeholder)
        if let Some(_task) = sub_handle.forward_task.take() {
            // Would cancel the task here
        }

        // Cancel the forwarding task
        if let Some(task) = sub_handle.forward_task.take() {
            task.abort();
        }

        // When zenoh-nostd is integrated:
        // subscriber.undeclare().await.ok();

        Ok(())
    } else {
        Err(ZenohError::SubscriptionNotFound(key_expr.to_string()))
    }
}

/// Executes a Zenoh request (publish or query).
///
/// Handles both publish and query operations using zenoh-nostd's .finish() builder pattern.
pub(crate) async fn execute_request(
    driver: &ZenohDriver,
    request: ZenohRequest,
) -> Result<ZenohResponse, ZenohError> {
    match request {
        ZenohRequest::Publish(pub_req) => {
            // Delegate to publish function
            publish(driver, pub_req).await?;
            Ok(ZenohResponse::Published)
        }
        ZenohRequest::Query(_query_req) => {
            let session_guard = driver
                .session
                .lock()
                .await
                .map_err(|e| ZenohError::Other(e.to_string()))?;

            let _state = session_guard
                .as_ref()
                .ok_or(ZenohError::SessionNotInitialized)?;

            // TODO: When zenoh-nostd is available:
            //
            // 1. Build selector with optional parameters:
            //    let mut selector = query_req.key_expr.clone();
            //    if let Some(params) = query_req.parameters {
            //        selector = format!("{}?{}", selector, params);
            //    }
            //
            // 2. Build query with optional timeout:
            //    let mut get_builder = state.session.get(&selector);
            //    if let Some(timeout_ms) = query_req.timeout_ms {
            //        get_builder = get_builder.timeout(Duration::from_millis(timeout_ms));
            //    }
            //
            // 3. Execute query and collect replies:
            //    let replies = get_builder
            //        .finish()
            //        .await
            //        .map_err(|e| ZenohError::Io(e.to_string()))?;
            //
            //    let mut collected_replies = Vec::new();
            //    while let Ok(reply) = replies.recv().await {
            //        match reply.result() {
            //            Ok(sample) => {
            //                collected_replies.push(ZenohReply {
            //                    key_expr: sample.key_expr().to_string(),
            //                    payload: sample.payload().to_bytes().to_vec(),
            //                });
            //            }
            //            Err(e) => {
            //                tracing::warn!("Query reply error: {}", e);
            //            }
            //        }
            //    }
            //
            //    Ok(ZenohResponse::QueryResponse(ZenohQueryResponse {
            //        replies: collected_replies,
            //    }))

            // For now, return error until zenoh-nostd types are available
            Err(ZenohError::RuntimeNotAvailable)
        }
    }
}

/// Starts the background event listener for Zenoh subscriptions.
///
/// In the Embassy runtime, each subscription spawns its own forwarding task
/// (see subscribe() function), so this listener just updates the status.
///
/// This is different from the std runtime which has a central polling loop.
pub(crate) async fn start_listener<S>(
    driver: &ZenohDriver,
    _sink: S,
) -> Result<(), ZenohError>
where
    S: EventSink<Event = ZenohEvent>,
{
    let mut status_guard = driver
        .listener_status
        .lock()
        .await
        .map_err(|e| ZenohError::Other(e.to_string()))?;

    if *status_guard == ZenohListenerStatus::Running {
        return Err(ZenohError::ListenerAlreadyRunning);
    }

    // Update status to Running
    // Note: Event forwarding is handled by per-subscription tasks spawned in subscribe()
    *status_guard = ZenohListenerStatus::Running;

    Ok(())
}

/// Stops the background event listener.
///
/// Updates the listener status. Actual subscription tasks are stopped in stop_session().
pub(crate) async fn stop_listener(driver: &ZenohDriver) -> Result<(), ZenohError> {
    let mut status_guard = driver
        .listener_status
        .lock()
        .await
        .map_err(|e| ZenohError::Other(e.to_string()))?;

    if *status_guard == ZenohListenerStatus::Stopped {
        return Ok(());
    }

    *status_guard = ZenohListenerStatus::Stopped;
    Ok(())
}
