#![cfg_attr(not(feature = "std"), no_std)]
//! MQTT protocol adapter for ferredge.
//!
//! With the default `std` feature, this crate provides a live TCP-backed MQTT client runtime,
//! publish/subscribe operations, and background event listening.
//!
//! Without `std`, this crate still supports routed-command conversion into MQTT-native packet
//! types through `TryFrom`, but transport runtime methods return explicit
//! `requires the "std" feature` errors.

#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(feature = "std")]
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc,
};

#[cfg(feature = "std")]
use std::string::{String, ToString};

#[cfg(not(feature = "std"))]
use alloc::string::{String, ToString};

use ferredge_core::prelude::*;

mod convert;
#[cfg(feature = "std")]
mod runtime;
mod types;
#[cfg(all(feature = "tokio-runtime", feature = "async-std-runtime"))]
compile_error!("ferredge-proto-mqtt supports only one std runtime stack feature at a time");
#[cfg(feature = "tokio-runtime")]
mod runtime_stack {
    pub use ferredge_runtime_tokio::{
        TokioNet as StackNet, TokioRuntime as StackRuntime, TokioSocket as StackSocket,
        TokioTask as RuntimeTask,
    };
}
#[cfg(feature = "async-std-runtime")]
mod runtime_stack {
    pub use ferredge_runtime_async_std::{
        AsyncStdNet as StackNet, AsyncStdRuntime as StackRuntime, AsyncStdSocket as StackSocket,
        AsyncStdTask as RuntimeTask,
    };
}

#[cfg(feature = "std")]
use runtime_stack::{RuntimeTask, StackNet, StackRuntime};

#[cfg(test)]
mod tests;

use types::{MqttPacketRequest, MqttResourceAttributes};

#[cfg(feature = "std")]
use runtime::{
    MqttClientSession, build_connect_packet, handle_connection_events, mqtt_version_from_core,
    normalize_broker_addr, read_from_session, send_packet_request,
};

pub use types::{
    MqttCommandConversionError, MqttPublishRequest, MqttSubscriptionRequest, MqttWirePacket,
};

/// Current state of the MQTT background listener runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MqttListenerStatus {
    /// Listener is not running and no failure is recorded.
    Stopped,
    /// Listener is actively polling the broker session.
    Running,
    /// Listener stopped after a runtime or sink failure.
    Failed(String),
}

#[cfg(feature = "std")]
const MQTT_LISTENER_EVENT_BUFFER_CAPACITY: usize = 256;

/// MQTT adapter backed by `mqtt_protocol_core` packet builders.
///
/// In `std` builds this type also owns the live TCP session and background listener runtime.
/// In `no_std` builds it remains useful for packet conversion, but transport operations fail
/// with explicit runtime-availability errors.
#[derive(Clone)]
pub struct MqttDriver {
    /// Device metadata and broker configuration served by this driver.
    pub dvc: Device<MqttResourceAttributes>,
    /// Live MQTT client session for std-enabled runtime transport.
    #[cfg(feature = "std")]
    session: Arc<Mutex<Option<MqttClientSession>>>,
    /// Background listener run flag.
    #[cfg(feature = "std")]
    listener_running: Arc<AtomicBool>,
    /// Background listener thread handle.
    #[cfg(feature = "std")]
    listener_handle: Arc<Mutex<Option<RuntimeTask<()>>>>,
    /// Last background listener failure, if any.
    #[cfg(feature = "std")]
    listener_error: Arc<Mutex<Option<String>>>,
    /// Subscribers interested in listener status transitions.
    #[cfg(feature = "std")]
    listener_status_subscribers: Arc<Mutex<Vec<mpsc::Sender<MqttListenerStatus>>>>,
    /// Std compatibility runtime used for background tasks and bounded channels.
    #[cfg(feature = "std")]
    runtime: Arc<StackRuntime>,
    /// Std compatibility network adapter used for outbound sockets.
    #[cfg(feature = "std")]
    net: StackNet,
}

impl core::fmt::Debug for MqttDriver {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MqttDriver")
            .field("dvc", &self.dvc)
            .finish()
    }
}

impl MqttDriver {
    /// Creates a new MQTT driver from device metadata.
    pub fn new(dvc: Device<MqttResourceAttributes>) -> Self {
        Self {
            dvc,
            #[cfg(feature = "std")]
            session: Arc::new(Mutex::new(None)),
            #[cfg(feature = "std")]
            listener_running: Arc::new(AtomicBool::new(false)),
            #[cfg(feature = "std")]
            listener_handle: Arc::new(Mutex::new(None)),
            #[cfg(feature = "std")]
            listener_error: Arc::new(Mutex::new(None)),
            #[cfg(feature = "std")]
            listener_status_subscribers: Arc::new(Mutex::new(Vec::new())),
            #[cfg(feature = "std")]
            runtime: Arc::new(StackRuntime::default()),
            #[cfg(feature = "std")]
            net: StackNet::default(),
        }
    }

    /// Establishes std transport session and MQTT handshake if not already connected.
    #[cfg(feature = "std")]
    pub fn connect(&self) -> Result<(), String> {
        self.ensure_connected()
    }

    /// Reads one batch of inbound MQTT data and converts packets into routed messages.
    #[cfg(feature = "std")]
    pub fn poll(&self) -> Result<Vec<RoutedMessage>, String> {
        self.ensure_connected()?;
        let mut guard = self
            .session
            .lock()
            .map_err(|_| "failed to lock MQTT session".to_string())?;
        let session = guard
            .as_mut()
            .ok_or_else(|| "MQTT session not initialized".to_string())?;
        read_from_session(
            self.runtime.as_ref(),
            session,
            &self.dvc.id,
            Some(std::time::Duration::from_millis(50)),
        )
    }

    #[cfg(feature = "std")]
    fn ensure_connected(&self) -> Result<(), String> {
        let mut guard = self
            .session
            .lock()
            .map_err(|_| "failed to lock MQTT session".to_string())?;
        if guard.is_some() {
            return Ok(());
        }

        let config = match &self.dvc.endpoint {
            DeviceEndpoint::Mqtt(config) => config,
            _ => return Err("device endpoint is not MQTT".to_string()),
        };

        let mut stream = self.runtime.block_on(
            self.net
                .connect(normalize_broker_addr(&config.broker).as_str()),
        )
            .map_err(|e| format!("failed to connect to MQTT broker: {e:?}"))?;
        stream
            .set_read_timeout(Some(std::time::Duration::from_millis(1000)))
            .map_err(|e| format!("failed to set MQTT read timeout: {e:?}"))?;
        stream
            .set_write_timeout(Some(std::time::Duration::from_millis(1000)))
            .map_err(|e| format!("failed to set MQTT write timeout: {e:?}"))?;

        let version = mqtt_version_from_core(config.preferred_protocol_version());
        let mut connection = mqtt_protocol_core::mqtt::Connection::<
            mqtt_protocol_core::mqtt::role::Client,
        >::new(version);
        connection.set_auto_pub_response(true);

        let connect_packet = build_connect_packet(config)?;
        let mut session = MqttClientSession {
            stream,
            connection,
            pending_command_ids: std::collections::HashMap::new(),
            pending_reply_routes: std::collections::HashMap::new(),
        };
        let events = session.connection.checked_send(connect_packet);
        handle_connection_events(self.runtime.as_ref(), &mut session, &self.dvc.id, events)?;
        let _ = read_from_session(
            self.runtime.as_ref(),
            &mut session,
            &self.dvc.id,
            Some(std::time::Duration::from_millis(1000)),
        )?;

        *guard = Some(session);
        Ok(())
    }

    #[cfg(feature = "std")]
    fn stop_listener(&self) -> Result<(), String> {
        self.listener_running.store(false, Ordering::SeqCst);
        let mut handle_guard = self
            .listener_handle
            .lock()
            .map_err(|_| "failed to lock MQTT listener handle".to_string())?;
        if let Some(handle) = handle_guard.take() {
            let mut handle = handle;
            if self.runtime.block_on(handle.join()).is_err() {
                let mut error_guard = self
                    .listener_error
                    .lock()
                    .map_err(|_| "failed to lock MQTT listener error".to_string())?;
                if error_guard.is_none() {
                    *error_guard = Some("MQTT listener thread panicked".to_string());
                }
            }
        }
        self.broadcast_listener_status(self.listener_status()?)?;
        Ok(())
    }

    /// Returns the last background listener error captured by the runtime, if any.
    #[cfg(feature = "std")]
    pub fn last_listener_error(&self) -> Result<Option<String>, String> {
        self.listener_error
            .lock()
            .map(|error| error.clone())
            .map_err(|_| "failed to lock MQTT listener error".to_string())
    }

    /// Clears any previously captured background listener error.
    #[cfg(feature = "std")]
    pub fn clear_listener_error(&self) -> Result<(), String> {
        let mut guard = self
            .listener_error
            .lock()
            .map_err(|_| "failed to lock MQTT listener error".to_string())?;
        *guard = None;
        self.broadcast_listener_status(MqttListenerStatus::Stopped)?;
        Ok(())
    }

    /// Returns high-level background listener status for this MQTT driver.
    #[cfg(feature = "std")]
    pub fn listener_status(&self) -> Result<MqttListenerStatus, String> {
        if self.listener_running.load(Ordering::SeqCst) {
            return Ok(MqttListenerStatus::Running);
        }

        match self.last_listener_error()? {
            Some(error) => Ok(MqttListenerStatus::Failed(error)),
            None => Ok(MqttListenerStatus::Stopped),
        }
    }

    /// Subscribes to background listener status changes for this MQTT driver.
    #[cfg(feature = "std")]
    pub fn subscribe_listener_status(&self) -> Result<mpsc::Receiver<MqttListenerStatus>, String> {
        let (tx, rx) = mpsc::channel();
        let current = self.listener_status()?;
        tx.send(current)
            .map_err(|_| "failed to seed MQTT listener status subscriber".to_string())?;
        self.listener_status_subscribers
            .lock()
            .map_err(|_| "failed to lock MQTT listener subscribers".to_string())?
            .push(tx);
        Ok(rx)
    }

    #[cfg(feature = "std")]
    fn broadcast_listener_status(&self, status: MqttListenerStatus) -> Result<(), String> {
        let mut subscribers = self
            .listener_status_subscribers
            .lock()
            .map_err(|_| "failed to lock MQTT listener subscribers".to_string())?;
        subscribers.retain(|subscriber| subscriber.send(status.clone()).is_ok());
        Ok(())
    }
}

#[cfg(feature = "std")]
fn broadcast_listener_status_to_subscribers(
    subscribers: &Arc<Mutex<Vec<mpsc::Sender<MqttListenerStatus>>>>,
    status: MqttListenerStatus,
) {
    if let Ok(mut subscribers) = subscribers.lock() {
        subscribers.retain(|subscriber| subscriber.send(status.clone()).is_ok());
    }
}

impl Lifecycle for MqttDriver {
    type Error = String;

    async fn start(&self) -> Result<(), Self::Error> {
        #[cfg(feature = "std")]
        {
            return self.connect();
        }

        #[cfg(not(feature = "std"))]
        Err("MQTT runtime requires the \"std\" feature".to_string())
    }

    async fn stop(&self) -> Result<(), Self::Error> {
        #[cfg(feature = "std")]
        {
            self.stop_listener()?;
            let mut guard = self
                .session
                .lock()
                .map_err(|_| "failed to lock MQTT session".to_string())?;
            *guard = None;
            self.broadcast_listener_status(self.listener_status()?)?;
            return Ok(());
        }

        #[cfg(not(feature = "std"))]
        Err("MQTT runtime requires the \"std\" feature".to_string())
    }
}

impl PubSub for MqttDriver {
    type PublishRequest = MqttPacketRequest;
    type Subscription = MqttPacketRequest;
    type Error = String;

    async fn publish(&self, request: Self::PublishRequest) -> Result<(), Self::Error> {
        #[cfg(feature = "std")]
        {
            self.ensure_connected()?;
            let mut guard = self
                .session
                .lock()
                .map_err(|_| "failed to lock MQTT session".to_string())?;
            let session = guard
                .as_mut()
                .ok_or_else(|| "MQTT session not initialized".to_string())?;

            send_packet_request(self.runtime.as_ref(), session, &self.dvc.id, request)?;
            let _ = read_from_session(
                self.runtime.as_ref(),
                session,
                &self.dvc.id,
                Some(std::time::Duration::from_millis(250)),
            )?;
            return Ok(());
        }

        #[cfg(not(feature = "std"))]
        Err("MQTT runtime requires the \"std\" feature".to_string())
    }

    async fn subscribe<S>(
        &self,
        subscription: Self::Subscription,
        sink: S,
    ) -> Result<(), Self::Error>
    where
        S: EventSink<Event = RoutedEvent> + Send,
    {
        #[cfg(feature = "std")]
        {
            self.ensure_connected()?;
            let mut guard = self
                .session
                .lock()
                .map_err(|_| "failed to lock MQTT session".to_string())?;
            let session = guard
                .as_mut()
                .ok_or_else(|| "MQTT session not initialized".to_string())?;

            send_packet_request(self.runtime.as_ref(), session, &self.dvc.id, subscription)?;
            let messages = read_from_session(
                self.runtime.as_ref(),
                session,
                &self.dvc.id,
                Some(std::time::Duration::from_millis(250)),
            )?;
            let mut sink = sink;
            for message in messages {
                if let RoutedMessage::Event(event) = message {
                    sink.handle(event)
                        .map_err(|_| "failed to forward MQTT event to sink".to_string())?;
                }
            }
            return Ok(());
        }

        #[cfg(not(feature = "std"))]
        Err("MQTT runtime requires the \"std\" feature".to_string())
    }

    async fn unsubscribe(&self, subscription: Self::Subscription) -> Result<(), Self::Error> {
        #[cfg(feature = "std")]
        {
            self.ensure_connected()?;
            let mut guard = self
                .session
                .lock()
                .map_err(|_| "failed to lock MQTT session".to_string())?;
            let session = guard
                .as_mut()
                .ok_or_else(|| "MQTT session not initialized".to_string())?;

            send_packet_request(self.runtime.as_ref(), session, &self.dvc.id, subscription)?;
            let _ = read_from_session(
                self.runtime.as_ref(),
                session,
                &self.dvc.id,
                Some(std::time::Duration::from_millis(250)),
            )?;
            return Ok(());
        }

        #[cfg(not(feature = "std"))]
        Err("MQTT runtime requires the \"std\" feature".to_string())
    }
}

impl EventSource for MqttDriver {
    type Event = RoutedEvent;
    type Error = String;

    async fn start_listening<S>(&self, sink: S) -> Result<(), Self::Error>
    where
        S: EventSink<Event = Self::Event> + Send + 'static,
    {
        #[cfg(feature = "std")]
        {
            self.ensure_connected()?;
            if self.listener_running.swap(true, Ordering::SeqCst) {
                return Err("MQTT listener already running".to_string());
            }

            let session = Arc::clone(&self.session);
            let running = Arc::clone(&self.listener_running);
            let listener_error = Arc::clone(&self.listener_error);
            let status_subscribers = Arc::clone(&self.listener_status_subscribers);
            let device_id = self.dvc.id.clone();
            let runtime = self.runtime.clone();
            self.clear_listener_error()?;
            self.broadcast_listener_status(MqttListenerStatus::Running)?;
            let handle = runtime.clone().spawn(async move {
                let (event_tx, mut event_rx) =
                    runtime.channel::<RoutedEvent>(MQTT_LISTENER_EVENT_BUFFER_CAPACITY);
                let sink_running = Arc::clone(&running);
                let sink_listener_error = Arc::clone(&listener_error);
                let sink_status_subscribers = Arc::clone(&status_subscribers);
                let sink_runtime = runtime.clone();
                let mut sink_handle = sink_runtime.spawn(async move {
                    let mut sink = sink;
                    loop {
                        match event_rx.recv().await {
                            Ok(event) => {
                                if sink.handle(event).is_err() {
                                    if let Ok(mut error_guard) = sink_listener_error.lock() {
                                        *error_guard =
                                            Some("failed to forward MQTT event to sink".to_string());
                                    }
                                    broadcast_listener_status_to_subscribers(
                                        &sink_status_subscribers,
                                        MqttListenerStatus::Failed(
                                            "failed to forward MQTT event to sink".to_string(),
                                        ),
                                    );
                                    sink_running.store(false, Ordering::SeqCst);
                                    break;
                                }
                            }
                            Err(ChannelError::Closed | ChannelError::RuntimeUnavailable) => break,
                            Err(ChannelError::Full) => {}
                        }
                    }
                });

                while running.load(Ordering::SeqCst) {
                    let messages = {
                        let mut guard = match session.lock() {
                            Ok(guard) => guard,
                            Err(_) => {
                                if let Ok(mut error_guard) = listener_error.lock() {
                                    *error_guard = Some(
                                        "failed to lock MQTT session while listening".to_string(),
                                    );
                                }
                                broadcast_listener_status_to_subscribers(
                                    &status_subscribers,
                                    MqttListenerStatus::Failed(
                                        "failed to lock MQTT session while listening".to_string(),
                                    ),
                                );
                                break;
                            }
                        };
                        let session = match guard.as_mut() {
                            Some(session) => session,
                            None => {
                                if let Ok(mut error_guard) = listener_error.lock() {
                                    *error_guard = Some(
                                        "MQTT session missing while listener was running".to_string(),
                                    );
                                }
                                broadcast_listener_status_to_subscribers(
                                    &status_subscribers,
                                    MqttListenerStatus::Failed(
                                        "MQTT session missing while listener was running"
                                            .to_string(),
                                    ),
                                );
                                break;
                            }
                        };
                        read_from_session(
                            runtime.as_ref(),
                            session,
                            &device_id,
                            Some(std::time::Duration::from_millis(250)),
                        )
                    };

                    match messages {
                        Ok(messages) => {
                            for message in messages {
                                if let RoutedMessage::Event(event) = message {
                                    match event_tx.try_send(event) {
                                        Ok(()) => {}
                                        Err(ChannelError::Full) => {
                                            if let Ok(mut error_guard) = listener_error.lock() {
                                                *error_guard = Some(
                                                    "MQTT listener event backlog exceeded"
                                                        .to_string(),
                                                );
                                            }
                                            broadcast_listener_status_to_subscribers(
                                                &status_subscribers,
                                                MqttListenerStatus::Failed(
                                                    "MQTT listener event backlog exceeded"
                                                        .to_string(),
                                                ),
                                            );
                                            running.store(false, Ordering::SeqCst);
                                            break;
                                        }
                                        Err(
                                            ChannelError::Closed
                                                | ChannelError::RuntimeUnavailable,
                                        ) => {
                                            if let Ok(mut error_guard) = listener_error.lock() {
                                                *error_guard = Some(
                                                    "MQTT listener sink worker disconnected"
                                                        .to_string(),
                                                );
                                            }
                                            broadcast_listener_status_to_subscribers(
                                                &status_subscribers,
                                                MqttListenerStatus::Failed(
                                                    "MQTT listener sink worker disconnected"
                                                        .to_string(),
                                                ),
                                            );
                                            running.store(false, Ordering::SeqCst);
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                        Err(error) => {
                            if let Ok(mut error_guard) = listener_error.lock() {
                                *error_guard = Some(error.clone());
                            }
                            broadcast_listener_status_to_subscribers(
                                &status_subscribers,
                                MqttListenerStatus::Failed(error),
                            );
                            running.store(false, Ordering::SeqCst);
                            break;
                        }
                    }
                }
                running.store(false, Ordering::SeqCst);
                drop(event_tx);
                let _ = runtime.block_on(sink_handle.join());
                let final_status = match listener_error.lock() {
                    Ok(error_guard) => match error_guard.clone() {
                        Some(error) => MqttListenerStatus::Failed(error),
                        None => MqttListenerStatus::Stopped,
                    },
                    Err(_) => MqttListenerStatus::Failed(
                        "failed to lock MQTT listener error after listener exit".to_string(),
                    ),
                };
                broadcast_listener_status_to_subscribers(&status_subscribers, final_status);
            });

            let mut handle_guard = self
                .listener_handle
                .lock()
                .map_err(|_| "failed to lock MQTT listener handle".to_string())?;
            *handle_guard = Some(handle);
            return Ok(());
        }

        #[cfg(not(feature = "std"))]
        {
            let _ = sink;
            Err("MQTT runtime requires the \"std\" feature".to_string())
        }
    }

    async fn stop_listening(&self) -> Result<(), Self::Error> {
        #[cfg(feature = "std")]
        {
            return self.stop_listener();
        }

        #[cfg(not(feature = "std"))]
        Err("MQTT runtime requires the \"std\" feature".to_string())
    }
}

pub use types::MqttCommandRef;
