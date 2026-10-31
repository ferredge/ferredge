//! Runtime implementations for Zenoh protocol adapter.
//!
//! This module provides both std (Tokio/Async-std) and no_std (Embassy) runtime support.
//! The std implementation uses the `zenoh` crate, while the embassy implementation uses
//! `zenoh-nostd` with a fundamentally different API.

// ====================================================================================
// STD RUNTIME (Tokio / Async-std)
// ====================================================================================

#[cfg(feature = "std")]
mod std_runtime {
    use std::{collections::HashMap, string::String, time::Duration, vec::Vec};

    use ferredge_core::prelude::*;

    use crate::{
        types::{
            ZenohError, ZenohEvent, ZenohPublishRequest, ZenohQueryResponse,
            ZenohReply, ZenohRequest, ZenohResponse, ZenohSubscription,
        },
        RuntimeReceiver, ZenohDriver, ZenohListenerStatus,
    };

    /// Zenoh session state wrapper.
    pub(crate) struct ZenohSessionState {
        pub(crate) session: zenoh::Session,
        // Store subscriptions as key_expr -> Subscriber handle with default FIFO handler
        pub(crate) subscriptions: HashMap<String, zenoh::pubsub::Subscriber<zenoh::handlers::FifoChannelHandler<zenoh::sample::Sample>>>,
        // Store queryables as key_expr -> unit
        pub(crate) queryables: HashMap<String, ()>,
    }

    impl ZenohSessionState {
        fn new(session: zenoh::Session) -> Self {
            Self {
                session,
                subscriptions: HashMap::new(),
                queryables: HashMap::new(),
            }
        }
    }

    /// Start the Zenoh session.
    pub(crate) async fn start_session(driver: &ZenohDriver) -> Result<(), ZenohError> {
        let mut session_guard = driver
            .session
            .lock()
            .await
            .map_err(|e| ZenohError::Other(e.to_string()))?;

        if session_guard.is_some() {
            return Ok(());
        }

        let config = zenoh::Config::default();
        let session = zenoh::open(config)
            .await
            .map_err(|e| ZenohError::SessionInitFailed(e.to_string()))?;

        *session_guard = Some(ZenohSessionState::new(session));
        Ok(())
    }

    /// Stop the Zenoh session.
    pub(crate) async fn stop_session(driver: &ZenohDriver) -> Result<(), ZenohError> {
        stop_listener(driver).await?;

        let mut session_guard = driver
            .session
            .lock()
            .await
            .map_err(|e| ZenohError::Other(e.to_string()))?;

        if let Some(mut state) = session_guard.take() {
            state.subscriptions.clear();
            state.queryables.clear();
            drop(state.session);
        }

        Ok(())
    }

    /// Publish a message to a key expression.
    pub(crate) async fn publish(
        driver: &ZenohDriver,
        request: ZenohPublishRequest,
    ) -> Result<(), ZenohError> {
        let session_guard = driver
            .session
            .lock()
            .await
            .map_err(|e| ZenohError::Other(e.to_string()))?;
        let state = session_guard
            .as_ref()
            .ok_or(ZenohError::SessionNotInitialized)?;

        state
            .session
            .put(&request.key_expr, request.payload)
            .await
            .map_err(|e| ZenohError::Io(e.to_string()))?;

        Ok(())
    }

    /// Subscribe to a key expression.
    pub(crate) async fn subscribe<S>(
        driver: &ZenohDriver,
        subscription: ZenohSubscription,
        _sink: S,
    ) -> Result<(), ZenohError>
    where
        S: EventSink<Event = RoutedEvent<'static>> + Send,
    {
        let mut session_guard = driver
            .session
            .lock()
            .await
            .map_err(|e| ZenohError::Other(e.to_string()))?;
        let state = session_guard
            .as_mut()
            .ok_or(ZenohError::SessionNotInitialized)?;

        if state.subscriptions.contains_key(&subscription.key_expr) {
            return Ok(());
        }

        let subscriber = state
            .session
            .declare_subscriber(&subscription.key_expr)
            .await
            .map_err(|e| ZenohError::Io(e.to_string()))?;

        state
            .subscriptions
            .insert(subscription.key_expr.clone(), subscriber);

        Ok(())
    }

    /// Unsubscribe from a key expression.
    pub(crate) async fn unsubscribe(
        driver: &ZenohDriver,
        subscription: ZenohSubscription,
    ) -> Result<(), ZenohError> {
        let mut session_guard = driver
            .session
            .lock()
            .await
            .map_err(|e| ZenohError::Other(e.to_string()))?;
        let state = session_guard
            .as_mut()
            .ok_or(ZenohError::SessionNotInitialized)?;

        if state.subscriptions.remove(&subscription.key_expr).is_none() {
            return Err(ZenohError::SubscriptionNotFound(subscription.key_expr));
        }

        Ok(())
    }

    /// Declare a queryable for responding to queries.
    pub(crate) async fn declare_queryable(
        driver: &ZenohDriver,
        request: crate::types::ZenohQueryableRequest,
    ) -> Result<(), ZenohError> {
        let mut session_guard = driver
            .session
            .lock()
            .await
            .map_err(|e| ZenohError::Other(e.to_string()))?;
        let state = session_guard
            .as_mut()
            .ok_or(ZenohError::SessionNotInitialized)?;

        if state.queryables.contains_key(&request.key_expr) {
            return Ok(());
        }

        // TODO: Implement actual zenoh queryable declaration with query.respond()
        state.queryables.insert(request.key_expr.clone(), ());

        Ok(())
    }

    /// Undeclare a queryable.
    pub(crate) async fn undeclare_queryable(
        driver: &ZenohDriver,
        request: crate::types::ZenohQueryableRequest,
    ) -> Result<(), ZenohError> {
        let mut session_guard = driver
            .session
            .lock()
            .await
            .map_err(|e| ZenohError::Other(e.to_string()))?;
        let state = session_guard
            .as_mut()
            .ok_or(ZenohError::SessionNotInitialized)?;

        if state.queryables.remove(&request.key_expr).is_none() {
            return Err(ZenohError::QueryableNotFound(request.key_expr));
        }

        Ok(())
    }

    /// Execute a request (publish, query, or queryable operations).
    pub(crate) async fn execute_request(
        driver: &ZenohDriver,
        request: ZenohRequest,
    ) -> Result<ZenohResponse, ZenohError> {
        match request {
            ZenohRequest::Publish(pub_req) => {
                publish(driver, pub_req).await?;
                Ok(ZenohResponse::Published)
            }
            ZenohRequest::DeclareQueryable(queryable_req) => {
                declare_queryable(driver, queryable_req).await?;
                Ok(ZenohResponse::QueryableDeclared)
            }
            ZenohRequest::UndeclareQueryable(queryable_req) => {
                undeclare_queryable(driver, queryable_req).await?;
                Ok(ZenohResponse::QueryableUndeclared)
            }
            ZenohRequest::Query(query_req) => {
                let session_guard = driver
                    .session
                    .lock()
                    .await
                    .map_err(|e| ZenohError::Other(e.to_string()))?;
                let state = session_guard
                    .as_ref()
                    .ok_or(ZenohError::SessionNotInitialized)?;

                let mut selector = query_req.key_expr.clone();
                if let Some(params) = query_req.parameters {
                    selector = format!("{}?{}", selector, params);
                }

                let mut get_builder = state.session.get(&selector);

                if let Some(timeout_ms) = query_req.timeout_ms {
                    get_builder = get_builder.timeout(Duration::from_millis(timeout_ms));
                }

                let replies = get_builder
                    .await
                    .map_err(|e| ZenohError::Io(e.to_string()))?;

                let mut collected_replies = Vec::new();

                while let Ok(reply) = replies.recv_async().await {
                    match reply.result() {
                        Ok(sample) => {
                            collected_replies.push(ZenohReply {
                                key_expr: sample.key_expr().to_string(),
                                payload: sample.payload().to_bytes().to_vec(),
                            });
                        }
                        Err(e) => {
                            tracing::warn!("Query reply error: {}", e);
                        }
                    }
                }

                Ok(ZenohResponse::QueryResponse(ZenohQueryResponse {
                    replies: collected_replies,
                }))
            }
        }
    }

    /// Start the background listener for subscriptions and queryables.
    pub(crate) async fn start_listener<S>(driver: &ZenohDriver, _sink: S) -> Result<(), ZenohError>
    where
        S: EventSink<Event = ZenohEvent> + Send + 'static,
    {
        let mut status_guard = driver
            .listener_status
            .lock()
            .await
            .map_err(|e| ZenohError::Other(e.to_string()))?;

        if *status_guard == ZenohListenerStatus::Running {
            return Err(ZenohError::ListenerAlreadyRunning);
        }

        let (stop_tx, stop_rx) = driver.runtime.channel(1);
        *driver
            .listener_stop_tx
            .lock()
            .await
            .map_err(|e| ZenohError::Other(e.to_string()))? = Some(stop_tx);

        *status_guard = ZenohListenerStatus::Running;
        drop(status_guard);

        let driver_clone = driver.clone();
        driver.runtime.spawn(async move {
            if let Err(e) = listener_task(driver_clone.clone(), _sink, stop_rx).await {
                if let Ok(mut status) = driver_clone.listener_status.lock().await {
                    *status = ZenohListenerStatus::Failed(e.to_string());
                }
            } else if let Ok(mut status) = driver_clone.listener_status.lock().await {
                *status = ZenohListenerStatus::Stopped;
            }
        });

        Ok(())
    }

    /// Stop the background listener.
    pub(crate) async fn stop_listener(driver: &ZenohDriver) -> Result<(), ZenohError> {
        let mut stop_tx_guard = driver
            .listener_stop_tx
            .lock()
            .await
            .map_err(|e| ZenohError::Other(e.to_string()))?;

        if let Some(stop_tx) = stop_tx_guard.take() {
            let _ = stop_tx.send(()).await;
        }

        let mut status_guard = driver
            .listener_status
            .lock()
            .await
            .map_err(|e| ZenohError::Other(e.to_string()))?;
        *status_guard = ZenohListenerStatus::Stopped;

        Ok(())
    }

    /// Background listener task that polls subscriptions.
    async fn listener_task<S>(
        driver: ZenohDriver,
        mut _sink: S,
        mut stop_rx: RuntimeReceiver<()>,
    ) -> Result<(), ZenohError>
    where
        S: EventSink<Event = ZenohEvent> + Send + 'static,
    {
        loop {
            if stop_rx.try_recv().is_ok() {
                return Ok(());
            }

            let session_guard = driver
                .session
                .lock()
                .await
                .map_err(|e| ZenohError::Other(e.to_string()))?;

            if let Some(state) = session_guard.as_ref() {
                for subscriber in state.subscriptions.values() {
                    loop {
                        match subscriber.try_recv() {
                            Ok(Some(sample)) => {
                                let event = ZenohEvent {
                                    key_expr: sample.key_expr().to_string(),
                                    payload: sample.payload().to_bytes().to_vec(),
                                };

                                if _sink.handle(event).is_err() {
                                    drop(session_guard);
                                    let mut status_guard = driver
                                        .listener_status
                                        .lock()
                                        .await
                                        .map_err(|e| ZenohError::Other(e.to_string()))?;
                                    *status_guard = ZenohListenerStatus::Failed("Sink closed".to_string());
                                    return Err(ZenohError::Io("sink closed".to_string()));
                                }
                            }
                            Ok(None) => {
                                break;
                            }
                            Err(e) => {
                                tracing::warn!("Subscriber error: {}", e);
                                break;
                            }
                        }
                    }
                }
            }

            drop(session_guard);
            driver.runtime.sleep(Duration::from_millis(10)).await;
        }
    }
}

#[cfg(feature = "std")]
pub(crate) use std_runtime::*;

// ====================================================================================
// EMBASSY RUNTIME (Embedded / no_std)
// ====================================================================================

#[cfg(feature = "embassy-runtime")]
mod embassy_runtime {
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

    /// Subscription handle with forwarding task.
    pub(crate) struct SubscriptionHandle {
        pub key_expr: String,
        pub forward_task: Option<RuntimeTask<()>>,
    }

    /// Session state wrapper for zenoh-nostd.
    pub(crate) struct ZenohNoStdSessionState {
        #[cfg(feature = "no_std")]
        #[allow(dead_code)]
        session: &'static dyn core::any::Any,

        #[cfg(not(feature = "no_std"))]
        #[allow(dead_code)]
        session_placeholder: (),

        pub run_task: Option<RuntimeTask<()>>,
        pub subscriptions: Vec<SubscriptionHandle>,
    }

    /// Start the Zenoh session (zenoh-nostd).
    pub(crate) async fn start_session(driver: &ZenohDriver) -> Result<(), ZenohError> {
        let session_guard = driver
            .session
            .lock()
            .await
            .map_err(|e| ZenohError::Other(e.to_string()))?;

        if session_guard.is_some() {
            return Ok(());
        }

        Err(ZenohError::Other(
            "zenoh-nostd requires platform-specific config (see PLATFORM_INTEGRATION.md)".to_string(),
        ))
    }

    /// Stop the Zenoh session.
    pub(crate) async fn stop_session(driver: &ZenohDriver) -> Result<(), ZenohError> {
        stop_listener(driver).await?;

        let mut session_guard = driver
            .session
            .lock()
            .await
            .map_err(|e| ZenohError::Other(e.to_string()))?;

        if let Some(mut state) = session_guard.take() {
            for mut sub_handle in state.subscriptions.drain(..) {
                if let Some(task) = sub_handle.forward_task.take() {
                    task.abort();
                }
            }

            if let Some(run_task) = state.run_task.take() {
                run_task.abort();
            }
        }

        Ok(())
    }

    /// Publish data.
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
            Err(ZenohError::Other(
                "zenoh-nostd publish requires platform-specific session".to_string(),
            ))
        }

        #[cfg(not(feature = "no_std"))]
        Err(ZenohError::RuntimeNotAvailable)
    }

    /// Subscribe to key expression.
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

        if state
            .subscriptions
            .iter()
            .any(|s| s.key_expr == subscription.key_expr)
        {
            return Ok(());
        }

        Err(ZenohError::Other(
            "zenoh-nostd subscribe requires static channels (see PLATFORM_INTEGRATION.md)".to_string(),
        ))
    }

    /// Unsubscribe from key expression.
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

        if let Some(pos) = state.subscriptions.iter().position(|s| s.key_expr == key_expr) {
            let mut sub_handle = state.subscriptions.remove(pos);

            if let Some(task) = sub_handle.forward_task.take() {
                task.abort();
            }

            Ok(())
        } else {
            Err(ZenohError::SubscriptionNotFound(key_expr.to_string()))
        }
    }

    /// Declare queryable.
    pub(crate) async fn declare_queryable(
        driver: &ZenohDriver,
        request: crate::types::ZenohQueryableRequest,
    ) -> Result<(), ZenohError> {
        let _session_guard = driver
            .session
            .lock()
            .await
            .map_err(|e| ZenohError::Other(e.to_string()))?;

        #[cfg(feature = "no_std")]
        {
            let _ = request;
            Err(ZenohError::Other(
                "Queryable support requires zenoh-nostd integration".to_string(),
            ))
        }

        #[cfg(not(feature = "no_std"))]
        Err(ZenohError::RuntimeNotAvailable)
    }

    /// Undeclare queryable.
    pub(crate) async fn undeclare_queryable(
        driver: &ZenohDriver,
        request: crate::types::ZenohQueryableRequest,
    ) -> Result<(), ZenohError> {
        let _session_guard = driver
            .session
            .lock()
            .await
            .map_err(|e| ZenohError::Other(e.to_string()))?;

        #[cfg(feature = "no_std")]
        {
            let _ = request;
            Err(ZenohError::Other(
                "Queryable support requires zenoh-nostd integration".to_string(),
            ))
        }

        #[cfg(not(feature = "no_std"))]
        Err(ZenohError::RuntimeNotAvailable)
    }

    /// Execute request.
    pub(crate) async fn execute_request(
        driver: &ZenohDriver,
        request: ZenohRequest,
    ) -> Result<ZenohResponse, ZenohError> {
        match request {
            ZenohRequest::Publish(pub_req) => {
                publish(driver, pub_req).await?;
                Ok(ZenohResponse::Published)
            }
            ZenohRequest::DeclareQueryable(queryable_req) => {
                declare_queryable(driver, queryable_req).await?;
                Ok(ZenohResponse::QueryableDeclared)
            }
            ZenohRequest::UndeclareQueryable(queryable_req) => {
                undeclare_queryable(driver, queryable_req).await?;
                Ok(ZenohResponse::QueryableUndeclared)
            }
            ZenohRequest::Query(_query_req) => {
                Err(ZenohError::RuntimeNotAvailable)
            }
        }
    }

    /// Start listener.
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

        *status_guard = ZenohListenerStatus::Running;

        Ok(())
    }

    /// Stop listener.
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
}

#[cfg(feature = "embassy-runtime")]
pub(crate) use embassy_runtime::*;
