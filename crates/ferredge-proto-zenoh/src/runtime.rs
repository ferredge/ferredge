#[cfg(feature = "std")]
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
        // Already started
        return Ok(());
    }

    // Open Zenoh session with default config
    let config = zenoh::Config::default();
    let session = zenoh::open(config)
        .await
        .map_err(|e| ZenohError::SessionInitFailed(e.to_string()))?;

    *session_guard = Some(ZenohSessionState::new(session));

    Ok(())
}

/// Stop the Zenoh session.
pub(crate) async fn stop_session(driver: &ZenohDriver) -> Result<(), ZenohError> {
    // Stop listener first
    stop_listener(driver).await?;

    let mut session_guard = driver
        .session
        .lock()
        .await
        .map_err(|e| ZenohError::Other(e.to_string()))?;

    if let Some(mut state) = session_guard.take() {
        // Undeclare all subscriptions
        state.subscriptions.clear();

        // Undeclare all queryables
        state.queryables.clear();

        // Close session
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

    // Check if already subscribed
    if state.subscriptions.contains_key(&subscription.key_expr) {
        return Ok(());
    }

    // Declare Zenoh subscriber for this key expression
    let subscriber = state
        .session
        .declare_subscriber(&subscription.key_expr)
        .await
        .map_err(|e| ZenohError::Io(e.to_string()))?;

    // Store the subscriber handle
    state
        .subscriptions
        .insert(subscription.key_expr.clone(), subscriber);

    // Note: Message forwarding happens in the listener via start_listening

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

    // Check if already declared
    if state.queryables.contains_key(&request.key_expr) {
        return Ok(());
    }

    // Declare Zenoh queryable for this key expression
    // Note: The actual query handling happens in the listener task
    // For now, we just mark it as declared
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

/// Execute a request (query).
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

            // Build query with optional parameters
            let mut selector = query_req.key_expr.clone();
            if let Some(params) = query_req.parameters {
                selector = format!("{}?{}", selector, params);
            }

            let mut get_builder = state.session.get(&selector);

            // Apply optional timeout
            if let Some(timeout_ms) = query_req.timeout_ms {
                get_builder = get_builder.timeout(Duration::from_millis(timeout_ms));
            }

            let replies = get_builder
                .await
                .map_err(|e| ZenohError::Io(e.to_string()))?;

            let mut collected_replies = Vec::new();

            // Collect replies
            while let Ok(reply) = replies.recv_async().await {
                match reply.result() {
                    Ok(sample) => {
                        collected_replies.push(ZenohReply {
                            key_expr: sample.key_expr().to_string(),
                            payload: sample.payload().to_bytes().to_vec(),
                        });
                    }
                    Err(e) => {
                        // Log error but continue collecting other replies
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

    // Spawn background task to poll subscriptions
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
        let _ = stop_tx.send(()).await; // Signal stop
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
        // Check for stop signal (non-blocking poll)
        if stop_rx.try_recv().is_ok() {
            // Stop requested
            return Ok(());
        }

        // Poll all subscriptions for new messages
        let session_guard = driver
            .session
            .lock()
            .await
            .map_err(|e| ZenohError::Other(e.to_string()))?;

        if let Some(state) = session_guard.as_ref() {
            for subscriber in state.subscriptions.values() {
                // Non-blocking poll for new samples
                loop {
                    match subscriber.try_recv() {
                        Ok(Some(sample)) => {
                            let event = ZenohEvent {
                                key_expr: sample.key_expr().to_string(),
                                payload: sample.payload().to_bytes().to_vec(),
                            };

                            // Forward event to sink, stop listener if sink fails
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
                            // Channel is empty, move to next subscription
                            break;
                        }
                        Err(e) => {
                            // Subscriber closed or error, log and continue
                            tracing::warn!("Subscriber error: {}", e);
                            break;
                        }
                    }
                }
            }
        }

        // Release lock before sleeping
        drop(session_guard);

        // Small sleep to avoid busy-waiting
        driver.runtime.sleep(Duration::from_millis(10)).await;
    }
}
