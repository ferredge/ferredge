//! Embassy runtime implementation for Zenoh using zenoh-nostd.
//!
//! This module provides no_std-compatible Zenoh session management for embassy runtimes.
//! It uses the zenoh-nostd crate which has a fundamentally different API than the std zenoh:
//! - Session creation via `zenoh::connect!()` or `zenoh::listen!()` macros
//! - Continuous `.run()` task required in background
//! - Channel-based pub/sub instead of callback-based
//! - All operations are async with builder patterns
//!
//! NOTE: This is currently a stub implementation that returns RuntimeNotAvailable errors.
//! All items are marked with #[allow(dead_code)] as they will be used in Phase 2.

#![allow(dead_code, unused_imports)]

use core::time::Duration;
use alloc::{string::String, vec::Vec};

use ferredge_core::prelude::*;

use crate::{
    types::{
        ZenohError, ZenohEvent, ZenohPublishRequest,
        ZenohReply, ZenohRequest, ZenohResponse, ZenohSubscription,
    },
    runtime_stack::{RuntimeTask, StackNet, StackRuntime},
    RuntimeReceiver, ZenohDriver, ZenohListenerStatus,
};

/// Session state wrapper for zenoh-nostd.
///
/// This will eventually hold:
/// - The zenoh-nostd session handle
/// - Subscription handles
/// - Background task handles
#[derive(Clone)]
pub(crate) struct ZenohNoStdSessionState {
    // Placeholder: will be filled when implementing zenoh-nostd integration
    // session: zenoh_nostd::session::Session<FerredgeZenohConfig>,
    // subscriptions: Vec<SubscriberHandle>,
}

/// Starts a Zenoh session using zenoh-nostd.
///
/// TODO: Implementation requires:
/// 1. Create FerredgeZenohConfig from driver.net
/// 2. Use zenoh::connect!() to establish session
/// 3. Spawn .run() task in background
/// 4. Store session in driver.session
pub(crate) async fn start_session(_driver: &ZenohDriver) -> Result<(), ZenohError> {
    // Stub: Returns error until zenoh-nostd integration is implemented
    Err(ZenohError::RuntimeNotAvailable)
}

/// Stops the Zenoh session.
///
/// TODO: Implementation requires:
/// 1. Close all subscriptions
/// 2. Stop .run() task
/// 3. Close session
pub(crate) async fn stop_session(_driver: &ZenohDriver) -> Result<(), ZenohError> {
    // Stub: Returns error until zenoh-nostd integration is implemented
    Err(ZenohError::RuntimeNotAvailable)
}

/// Publishes data to a Zenoh key expression.
///
/// TODO: zenoh-nostd API: session.put(key_expr, payload).finish().await
pub(crate) async fn publish(
    _driver: &ZenohDriver,
    _request: ZenohPublishRequest,
) -> Result<(), ZenohError> {
    // Stub: Returns error until zenoh-nostd integration is implemented
    Err(ZenohError::RuntimeNotAvailable)
}

/// Subscribes to a Zenoh key expression.
///
/// TODO: Implementation requires:
/// 1. Create static channel for subscriber
/// 2. session.declare_subscriber(key_expr).channel(...).finish().await
/// 3. Spawn task to forward messages from channel to sink
pub(crate) async fn subscribe<S>(
    _driver: &ZenohDriver,
    _subscription: ZenohSubscription,
    _sink: S,
) -> Result<(), ZenohError>
where
    S: EventSink<Event = RoutedEvent<'static>>,
{
    // Stub: Returns error until zenoh-nostd integration is implemented
    Err(ZenohError::RuntimeNotAvailable)
}

/// Unsubscribes from a Zenoh key expression.
pub(crate) async fn unsubscribe(
    _driver: &ZenohDriver,
    _key_expr: &str,
) -> Result<(), ZenohError> {
    // Stub: Returns error until zenoh-nostd integration is implemented
    Err(ZenohError::RuntimeNotAvailable)
}

/// Executes a Zenoh request (publish or query).
///
/// TODO: zenoh-nostd API:
/// - For publish: session.put(key_expr, payload).finish().await
/// - For query: session.get(selector).finish().await
pub(crate) async fn execute_request(
    _driver: &ZenohDriver,
    _request: ZenohRequest,
) -> Result<ZenohResponse, ZenohError> {
    // Stub: Returns error until zenoh-nostd integration is implemented
    Err(ZenohError::RuntimeNotAvailable)
}

/// Starts the background event listener for Zenoh subscriptions.
///
/// TODO: Background task to poll for events from subscriptions
pub(crate) async fn start_listener<S>(
    _driver: &ZenohDriver,
    _sink: S,
) -> Result<(), ZenohError>
where
    S: EventSink<Event = ZenohEvent>,
{
    // Stub: Returns error until zenoh-nostd integration is implemented
    Err(ZenohError::RuntimeNotAvailable)
}

/// Stops the background event listener.
pub(crate) async fn stop_listener(_driver: &ZenohDriver) -> Result<(), ZenohError> {
    // Stub: Returns error until zenoh-nostd integration is implemented
    Err(ZenohError::RuntimeNotAvailable)
}
