//! MQTT protocol adapter for ferredge.

extern crate alloc;

use alloc::string::{String, ToString};

use ferredge_bridge::{BridgeAdapter, BridgeCodec, BridgeMessage, planner};
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

// compile error when no runtime stack feature is enabled
#[cfg(not(any(feature = "tokio-runtime", feature = "async-std-runtime")))]
compile_error!("ferredge-proto-mqtt requires a supported runtime stack feature to be enabled");

use runtime_stack::{RuntimeTask, StackNet, StackRuntime};

type RuntimeSender<T> = <StackRuntime as AsyncRuntime>::Sender<T>;

type RuntimeReceiver<T> = <StackRuntime as AsyncRuntime>::Receiver<T>;

type RuntimeMutex<T> = <StackRuntime as AsyncRuntime>::Mutex<T>;

#[cfg(all(test, feature = "integration"))]
mod mosquitto_tests;
#[cfg(test)]
mod tests;

use types::{MqttPacketRequest, MqttResourceAttributes};

use runtime::{
    MqttClientSession, build_connect_packet, disconnect_session, mqtt_version_from_core,
    normalize_broker_addr, read_from_session_async, send_packet_request_async,
};

pub use convert::MqttBridgeCodec;
pub use types::{
    MqttAuthChallenge, MqttAuthFlowReason, MqttAuthProvider, MqttAuthResponse, MqttAuthStage,
    MqttCommandConversionError, MqttPublishRequest, MqttSubscriptionRequest, MqttWirePacket,
};

#[cfg(feature = "std")]
type MqttAuthHandler = Shared<dyn MqttAuthProvider>;

/// Bridge adapter for MQTT messaging semantics.
pub struct MqttBridgeAdapter;

const MQTT_CONNECT_IO_TIMEOUT_MS: u64 = 1_000;
const MQTT_ACK_READ_TIMEOUT_MS: u64 = 250;
const MQTT_LISTENER_IDLE_POLL_MS: u64 = 250;
const MQTT_SESSION_WAIT_POLL_MS: u64 = 10;
const MQTT_LISTENER_COMMAND_TIMEOUT_MS: u64 = 750;
const MQTT_LISTENER_COMMAND_CHANNEL_CAPACITY: usize = 64;

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
    session: Shared<RuntimeMutex<Option<MqttClientSession>>>,
    /// Last negotiated CONNACK capabilities and identifiers observed from broker.
    negotiated_connack: Shared<RuntimeMutex<Option<MqttConnackProperties>>>,
    /// Background listener run flag.
    listener_running: Shared<AtomicFlag>,
    /// Background listener task handle.
    listener_handle: Shared<RuntimeMutex<Option<RuntimeTask<()>>>>,
    /// Last background listener failure, if any.
    listener_error: Shared<RuntimeMutex<Option<String>>>,
    /// Subscribers interested in listener status transitions.
    listener_status_subscribers: Shared<RuntimeMutex<Vec<RuntimeSender<MqttListenerStatus>>>>,
    /// Command path into the active listener-owned session.
    listener_command_sender: Shared<RuntimeMutex<Option<RuntimeSender<SessionCommand>>>>,
    /// Selected runtime used for background tasks.
    runtime: Shared<StackRuntime>,
    /// Selected network adapter used for outbound sockets.
    net: StackNet,
    /// Desired subscription state to replay after reconnect.
    desired_subscriptions: Shared<RuntimeMutex<Map<String, MqttPacketRequest>>>,
    /// Queued outbound requests waiting for broker recovery.
    recovery_queue: Shared<RuntimeMutex<VecDeque<MqttPacketRequest>>>,
    /// Optional enhanced authentication callback for MQTT v5 AUTH flow.
    auth_handler: Shared<RuntimeMutex<Option<MqttAuthHandler>>>,
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
        let runtime = Shared::new(StackRuntime::default());

        Self {
            dvc,
            session: Shared::new(runtime.mutex(None)),
            negotiated_connack: Shared::new(runtime.mutex(None)),
            listener_running: Shared::new(AtomicFlag::new(false)),
            listener_handle: Shared::new(runtime.mutex(None)),
            listener_error: Shared::new(runtime.mutex(None)),
            listener_status_subscribers: Shared::new(runtime.mutex(Vec::new())),
            listener_command_sender: Shared::new(runtime.mutex(None)),
            runtime: runtime.clone(),
            net: StackNet::default(),
            desired_subscriptions: Shared::new(runtime.mutex(Map::new())),
            recovery_queue: Shared::new(runtime.mutex(VecDeque::new())),
            auth_handler: Shared::new(runtime.mutex(None)),
        }
    }

    pub fn bridge_packet_request(
        &self,
        command: Command,
    ) -> Result<MqttPacketRequest, MqttCommandConversionError> {
        let message = planner_message_for_command(command)?;
        MqttBridgeCodec::new(&self.dvc).encode(&message)
    }

    /// Registers enhanced MQTT v5 auth callback used for connect-time and re-auth exchanges.
    pub fn set_auth_handler<P>(&self, handler: P) -> Result<(), String>
    where
        P: MqttAuthProvider + 'static,
    {
        let mut guard = self
            .runtime
            .block_on(self.auth_handler.lock())
            .map_err(|_| "failed to lock MQTT auth handler".to_string())?;
        *guard = Some(Shared::new(handler));
        Ok(())
    }

    async fn ensure_connected_async(&self) -> Result<(), String> {
        {
            let guard = self
                .session
                .lock()
                .await
                .map_err(|_| "failed to lock MQTT session".to_string())?;
            if guard.is_some() {
                return Ok(());
            }
        }

        let config = match &self.dvc.endpoint {
            DeviceEndpoint::Mqtt(config) => config,
            _ => return Err("device endpoint is not MQTT".to_string()),
        };

        let mut stream = self
            .net
            .connect(normalize_broker_addr(&config.broker).as_str())
            .await
            .map_err(|e| format!("failed to connect to MQTT broker: {e:?}"))?;
        stream
            .set_read_timeout(Some(core::time::Duration::from_millis(
                MQTT_CONNECT_IO_TIMEOUT_MS,
            )))
            .map_err(|e| format!("failed to set MQTT read timeout: {e:?}"))?;
        stream
            .set_write_timeout(Some(core::time::Duration::from_millis(
                MQTT_CONNECT_IO_TIMEOUT_MS,
            )))
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
            protocol_version: config.preferred_protocol_version(),
            keepalive_secs: config.keepalive_secs,
            last_activity: self.runtime.now(),
            awaiting_pingresp: false,
            pending_command_ids: Map::new(),
            pending_reply_routes: Map::new(),
            authentication_method: config.connect_properties.authentication_method.clone(),
        };
        let events = session.connection.checked_send(connect_packet);
        runtime::handle_connection_events_async(
            &self.runtime,
            &mut session,
            &self.dvc.id,
            Some(&self.negotiated_connack),
            Some(&self.auth_handler),
            events,
        )
        .await?;
        loop {
            let _ = read_from_session_async(
                &self.runtime,
                &mut session,
                &self.dvc.id,
                Some(&self.negotiated_connack),
                Some(&self.auth_handler),
                Some(core::time::Duration::from_millis(
                    MQTT_CONNECT_IO_TIMEOUT_MS,
                )),
            )
            .await?;

            let negotiated = self
                .negotiated_connack
                .lock()
                .await
                .map_err(|_| "failed to lock negotiated MQTT CONNACK state".to_string())?
                .clone();
            if let Some(connack) = negotiated {
                if connack.reason_code.as_deref() == Some("Success") {
                    break;
                }
                let detail = connack
                    .reason_string
                    .clone()
                    .unwrap_or_else(|| connack.reason_code.clone().unwrap_or_default());
                return Err(format!("mqtt connect rejected: {detail}"));
            }
        }

        let mut guard = self
            .session
            .lock()
            .await
            .map_err(|_| "failed to lock MQTT session".to_string())?;
        if guard.is_none() {
            *guard = Some(session);
        }
        Ok(())
    }

    /// Returns the most recent broker CONNACK negotiation snapshot, if available.

    pub fn negotiated_connack(&self) -> Result<Option<MqttConnackProperties>, String> {
        let guard = self
            .runtime
            .block_on(self.negotiated_connack.lock())
            .map_err(|_| "failed to lock negotiated MQTT CONNACK state".to_string())?;
        Ok((*guard).clone())
    }

    fn reconnect_config(&self) -> Result<BrokerReconnectConfig, String> {
        match &self.dvc.endpoint {
            DeviceEndpoint::Mqtt(config) => Ok(config.reconnect.clone()),
            _ => Err("device endpoint is not MQTT".to_string()),
        }
    }

    fn request_topic_key(request: &MqttPacketRequest) -> Option<String> {
        match &request.packet {
            MqttWirePacket::V5Subscribe(packet) => packet
                .entries()
                .first()
                .map(|entry| entry.topic_filter().to_string()),
            MqttWirePacket::V3Subscribe(packet) => packet
                .entries()
                .first()
                .map(|entry| entry.topic_filter().to_string()),
            MqttWirePacket::V5Unsubscribe(packet) => packet
                .entries()
                .first()
                .map(|entry| entry.as_str().to_string()),
            MqttWirePacket::V3Unsubscribe(packet) => packet
                .entries()
                .first()
                .map(|entry| entry.as_str().to_string()),
            _ => None,
        }
    }

    fn track_subscription_intent(&self, request: &MqttPacketRequest) -> Result<(), String> {
        let Some(topic) = Self::request_topic_key(request) else {
            return Ok(());
        };
        let mut desired = self
            .runtime
            .block_on(self.desired_subscriptions.lock())
            .map_err(|_| "failed to lock MQTT desired subscriptions".to_string())?;
        match request.packet {
            MqttWirePacket::V5Subscribe(_) | MqttWirePacket::V3Subscribe(_) => {
                desired.insert(topic, request.clone());
            }
            MqttWirePacket::V5Unsubscribe(_) | MqttWirePacket::V3Unsubscribe(_) => {
                desired.remove(&topic);
            }
            _ => {}
        }
        Ok(())
    }

    fn enqueue_recovery_request(&self, request: MqttPacketRequest) -> Result<(), String> {
        let reconnect = self.reconnect_config()?;
        if !reconnect.queue_requests_while_disconnected {
            return Err("MQTT recovery queue is disabled".to_string());
        }

        let mut queue = self
            .runtime
            .block_on(self.recovery_queue.lock())
            .map_err(|_| "failed to lock MQTT recovery queue".to_string())?;
        if queue.len() >= reconnect.max_queued_requests as usize {
            queue.pop_front();
        }
        queue.push_back(request);
        Ok(())
    }

    async fn replay_recovery_state_on_session_async(
        &self,
        session: &mut MqttClientSession,
    ) -> Result<Vec<RoutedEvent>, String> {
        let reconnect = self.reconnect_config()?;
        let result = async {
            let mut recovered_events = Vec::new();
            if reconnect.replay_subscriptions {
                let desired = self
                    .desired_subscriptions
                    .lock()
                    .await
                    .map_err(|_| "failed to lock MQTT desired subscriptions".to_string())?
                    .values()
                    .cloned()
                    .collect::<Vec<_>>();
                for request in desired {
                    send_packet_request_async(&self.runtime, session, &self.dvc.id, request)
                        .await?;
                    let messages = read_from_session_async(
                        &self.runtime,
                        session,
                        &self.dvc.id,
                        Some(&self.negotiated_connack),
                        Some(&self.auth_handler),
                        Some(core::time::Duration::from_millis(MQTT_ACK_READ_TIMEOUT_MS)),
                    )
                    .await?;
                    recovered_events.extend(messages.into_iter().filter_map(
                        |message| match message {
                            RoutedMessage::Event(event) => Some(event),
                            _ => None,
                        },
                    ));
                }
            }

            let queued = {
                let mut queue = self
                    .recovery_queue
                    .lock()
                    .await
                    .map_err(|_| "failed to lock MQTT recovery queue".to_string())?;
                queue.drain(..).collect::<Vec<_>>()
            };
            for request in queued {
                send_packet_request_async(&self.runtime, session, &self.dvc.id, request).await?;
                let messages = read_from_session_async(
                    &self.runtime,
                    session,
                    &self.dvc.id,
                    Some(&self.negotiated_connack),
                    Some(&self.auth_handler),
                    Some(core::time::Duration::from_millis(MQTT_ACK_READ_TIMEOUT_MS)),
                )
                .await?;
                recovered_events.extend(messages.into_iter().filter_map(|message| match message {
                    RoutedMessage::Event(event) => Some(event),
                    _ => None,
                }));
            }
            Ok::<Vec<RoutedEvent>, String>(recovered_events)
        }
        .await;

        result
    }

    fn stop_listener(&self) -> Result<(), String> {
        self.listener_running.store(false, AtomicOrdering::SeqCst);
        {
            let mut sender_guard = self
                .runtime
                .block_on(self.listener_command_sender.lock())
                .map_err(|_| "failed to lock MQTT listener command sender".to_string())?;
            *sender_guard = None;
        }
        let mut handle_guard = self
            .runtime
            .block_on(self.listener_handle.lock())
            .map_err(|_| "failed to lock MQTT listener handle".to_string())?;
        if let Some(handle) = handle_guard.take() {
            let mut handle = handle;
            if self.runtime.block_on(handle.join()).is_err() {
                let mut error_guard = self
                    .runtime
                    .block_on(self.listener_error.lock())
                    .map_err(|_| "failed to lock MQTT listener error".to_string())?;
                if error_guard.is_none() {
                    *error_guard = Some("MQTT listener task panicked".to_string());
                }
            }
        }
        self.broadcast_listener_status(self.listener_status()?)?;
        Ok(())
    }

    /// Returns the last background listener error captured by the runtime, if any.
    pub fn last_listener_error(&self) -> Result<Option<String>, String> {
        {
            let error = self
                .runtime
                .block_on(self.listener_error.lock())
                .map_err(|_| "failed to lock MQTT listener error".to_string())?;
            return Ok(error.clone());
        }

        #[cfg(not(feature = "std"))]
        {
            Ok(None)
        }
    }

    /// Clears any previously captured background listener error.
    pub fn clear_listener_error(&self) -> Result<(), String> {
        {
            let mut guard = self
                .runtime
                .block_on(self.listener_error.lock())
                .map_err(|_| "failed to lock MQTT listener error".to_string())?;
            *guard = None;
            self.broadcast_listener_status(MqttListenerStatus::Stopped)?;
            return Ok(());
        }

        #[cfg(not(feature = "std"))]
        {
            Ok(())
        }
    }

    /// Returns high-level background listener status for this MQTT driver.
    pub fn listener_status(&self) -> Result<MqttListenerStatus, String> {
        {
            if self.listener_running.load(AtomicOrdering::SeqCst) {
                return Ok(MqttListenerStatus::Running);
            }

            return match self.last_listener_error()? {
                Some(error) => Ok(MqttListenerStatus::Failed(error)),
                None => Ok(MqttListenerStatus::Stopped),
            };
        }

        #[cfg(not(feature = "std"))]
        {
            Ok(MqttListenerStatus::Stopped)
        }
    }

    /// Subscribes to background listener status changes for this MQTT driver.

    pub fn subscribe_listener_status(&self) -> Result<RuntimeReceiver<MqttListenerStatus>, String> {
        let (tx, rx) = self.runtime.channel(8);
        let current = self.listener_status()?;
        tx.try_send(current)
            .map_err(|_| "failed to seed MQTT listener status subscriber".to_string())?;
        self.runtime
            .block_on(self.listener_status_subscribers.lock())
            .map_err(|_| "failed to lock MQTT listener subscribers".to_string())?
            .push(tx);
        Ok(rx)
    }

    fn broadcast_listener_status(&self, status: MqttListenerStatus) -> Result<(), String> {
        let mut subscribers = self
            .runtime
            .block_on(self.listener_status_subscribers.lock())
            .map_err(|_| "failed to lock MQTT listener subscribers".to_string())?;
        subscribers.retain(|subscriber| {
            !matches!(
                subscriber.try_send(status.clone()),
                Err(ChannelError::Closed)
            )
        });
        Ok(())
    }

    async fn send_listener_command(
        &self,
        command: SessionCommand,
    ) -> Result<SessionCommandResult, String> {
        let sender = self
            .listener_command_sender
            .lock()
            .await
            .map_err(|_| "failed to lock MQTT listener command sender".to_string())?
            .clone()
            .ok_or_else(|| "MQTT listener command path is not initialized".to_string())?;

        let (response_tx, mut response_rx) = self.runtime.channel(1);
        let command = command.with_response(response_tx);
        sender
            .send(command)
            .await
            .map_err(|_| "failed to send MQTT command to listener".to_string())?;

        let deadline = self.runtime.now();
        loop {
            match response_rx.try_recv() {
                Ok(result) => return result,
                Err(ChannelError::Empty) => {
                    if deadline.elapsed()
                        >= core::time::Duration::from_millis(MQTT_LISTENER_COMMAND_TIMEOUT_MS)
                    {
                        return Err("MQTT listener command timed out".to_string());
                    }
                    self.runtime
                        .sleep(core::time::Duration::from_millis(MQTT_SESSION_WAIT_POLL_MS))
                        .await;
                }
                Err(ChannelError::Closed) => {
                    return Err("MQTT listener command response channel closed".to_string());
                }
                Err(ChannelError::Full | ChannelError::RuntimeUnavailable) => {
                    return Err("MQTT listener command response unavailable".to_string());
                }
            }
        }
    }

    async fn take_session_wait_async(&self) -> Result<MqttClientSession, String> {
        loop {
            {
                let mut guard = self
                    .session
                    .lock()
                    .await
                    .map_err(|_| "failed to lock MQTT session".to_string())?;
                if let Some(session) = guard.take() {
                    return Ok(session);
                }
                if !self.listener_running.load(AtomicOrdering::SeqCst) {
                    return Err("MQTT session not initialized".to_string());
                }
            }

            self.runtime
                .sleep(core::time::Duration::from_millis(MQTT_SESSION_WAIT_POLL_MS))
                .await;
        }
    }

    async fn restore_session(&self, session: MqttClientSession) -> Result<(), String> {
        let mut guard = self
            .session
            .lock()
            .await
            .map_err(|_| "failed to lock MQTT session".to_string())?;
        *guard = Some(session);
        Ok(())
    }

    async fn clear_session(&self) -> Result<(), String> {
        let mut guard = self
            .session
            .lock()
            .await
            .map_err(|_| "failed to lock MQTT session".to_string())?;
        *guard = None;
        Ok(())
    }
}

#[derive(Debug)]
enum SessionCommandKind {
    Publish(MqttPacketRequest),
    Subscribe(MqttPacketRequest),
    Unsubscribe(MqttPacketRequest),
}

struct SessionCommand {
    kind: SessionCommandKind,
    response: Option<RuntimeSender<Result<SessionCommandResult, String>>>,
}

impl SessionCommand {
    fn with_response(
        mut self,
        response: RuntimeSender<Result<SessionCommandResult, String>>,
    ) -> Self {
        self.response = Some(response);
        self
    }
}

#[derive(Debug, Clone)]
enum SessionCommandResult {
    Done,
    Events(Vec<RoutedEvent>),
}

async fn broadcast_listener_status_to_subscribers(
    subscribers: &Shared<RuntimeMutex<Vec<RuntimeSender<MqttListenerStatus>>>>,
    status: MqttListenerStatus,
) {
    if let Ok(mut subscribers) = subscribers.lock().await {
        subscribers.retain(|subscriber| {
            !matches!(
                subscriber.try_send(status.clone()),
                Err(ChannelError::Closed)
            )
        });
    }
}

async fn process_session_command(
    runtime: &StackRuntime,
    session: &mut MqttClientSession,
    device_id: &str,
    auth_handler: &Shared<RuntimeMutex<Option<MqttAuthHandler>>>,
    command: SessionCommand,
) -> Result<(), String> {
    let response = match command.kind {
        SessionCommandKind::Publish(request) => {
            async {
                send_packet_request_async(runtime, session, device_id, request).await?;
                let _ = read_from_session_async(
                    runtime,
                    session,
                    device_id,
                    None,
                    Some(auth_handler),
                    Some(core::time::Duration::from_millis(MQTT_ACK_READ_TIMEOUT_MS)),
                )
                .await?;
                Ok::<SessionCommandResult, String>(SessionCommandResult::Done)
            }
            .await
        }
        SessionCommandKind::Subscribe(request) => {
            async {
                send_packet_request_async(runtime, session, device_id, request).await?;
                let messages = read_from_session_async(
                    runtime,
                    session,
                    device_id,
                    None,
                    Some(auth_handler),
                    Some(core::time::Duration::from_millis(MQTT_ACK_READ_TIMEOUT_MS)),
                )
                .await?;
                Ok::<SessionCommandResult, String>(SessionCommandResult::Events(
                    messages
                        .into_iter()
                        .filter_map(|message| match message {
                            RoutedMessage::Event(event) => Some(event),
                            _ => None,
                        })
                        .collect(),
                ))
            }
            .await
        }
        SessionCommandKind::Unsubscribe(request) => {
            async {
                send_packet_request_async(runtime, session, device_id, request).await?;
                let _ = read_from_session_async(
                    runtime,
                    session,
                    device_id,
                    None,
                    Some(auth_handler),
                    Some(core::time::Duration::from_millis(MQTT_ACK_READ_TIMEOUT_MS)),
                )
                .await?;
                Ok::<SessionCommandResult, String>(SessionCommandResult::Done)
            }
            .await
        }
    };

    if let Some(response_tx) = command.response {
        let _ = response_tx.send(response.clone()).await;
    }
    response.map(|_| ())
}

impl Lifecycle for MqttDriver {
    type Error = String;

    async fn start(&self) -> Result<(), Self::Error> {
        {
            return self.ensure_connected_async().await;
        }

        #[cfg(not(feature = "std"))]
        Err("MQTT runtime requires the \"std\" feature".to_string())
    }

    async fn stop(&self) -> Result<(), Self::Error> {
        {
            self.stop_listener()?;
            let maybe_session = {
                let mut guard = self
                    .session
                    .lock()
                    .await
                    .map_err(|_| "failed to lock MQTT session".to_string())?;
                guard.take()
            };
            if let Some(mut session) = maybe_session {
                let _ = disconnect_session(&self.runtime, &mut session).await;
            }
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
        {
            if self.listener_running.load(AtomicOrdering::SeqCst) {
                match self
                    .send_listener_command(SessionCommand {
                        kind: SessionCommandKind::Publish(request.clone()),
                        response: None,
                    })
                    .await
                {
                    Ok(SessionCommandResult::Done) => return Ok(()),
                    Ok(SessionCommandResult::Events(_)) => return Ok(()),
                    Err(error) => {
                        if self.enqueue_recovery_request(request).is_ok() {
                            return Ok(());
                        }
                        return Err(error);
                    }
                }
            }
            if let Err(error) = self.ensure_connected_async().await {
                return Err(error);
            }
            let mut session = self.take_session_wait_async().await?;
            let result = async {
                send_packet_request_async(
                    &self.runtime,
                    &mut session,
                    &self.dvc.id,
                    request.clone(),
                )
                .await?;
                let _ = read_from_session_async(
                    &self.runtime,
                    &mut session,
                    &self.dvc.id,
                    Some(&self.negotiated_connack),
                    Some(&self.auth_handler),
                    Some(core::time::Duration::from_millis(MQTT_ACK_READ_TIMEOUT_MS)),
                )
                .await?;
                Ok::<(), String>(())
            }
            .await;
            if result.is_ok() {
                self.restore_session(session).await?;
            } else {
                self.clear_session().await?;
                if self.listener_running.load(AtomicOrdering::SeqCst) {
                    let _ = self.enqueue_recovery_request(request);
                    return Ok(());
                }
            }
            result?;
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
        {
            self.track_subscription_intent(&subscription)?;
            if self.listener_running.load(AtomicOrdering::SeqCst) {
                match self
                    .send_listener_command(SessionCommand {
                        kind: SessionCommandKind::Subscribe(subscription.clone()),
                        response: None,
                    })
                    .await
                {
                    Ok(SessionCommandResult::Done) => return Ok(()),
                    Ok(SessionCommandResult::Events(events)) => {
                        let mut sink = sink;
                        for event in events {
                            sink.handle(event)
                                .map_err(|_| "failed to forward MQTT event to sink".to_string())?;
                        }
                        return Ok(());
                    }
                    Err(error) => {
                        if self.enqueue_recovery_request(subscription).is_ok() {
                            return Ok(());
                        }
                        return Err(error);
                    }
                }
            }
            if let Err(error) = self.ensure_connected_async().await {
                return Err(error);
            }
            let mut session = self.take_session_wait_async().await?;
            let result = async {
                send_packet_request_async(
                    &self.runtime,
                    &mut session,
                    &self.dvc.id,
                    subscription.clone(),
                )
                .await?;
                read_from_session_async(
                    &self.runtime,
                    &mut session,
                    &self.dvc.id,
                    Some(&self.negotiated_connack),
                    Some(&self.auth_handler),
                    Some(core::time::Duration::from_millis(MQTT_ACK_READ_TIMEOUT_MS)),
                )
                .await
            }
            .await;
            if result.is_ok() {
                self.restore_session(session).await?;
            } else {
                self.clear_session().await?;
                if self.listener_running.load(AtomicOrdering::SeqCst) {
                    let _ = self.enqueue_recovery_request(subscription);
                    return Ok(());
                }
            }
            let messages = result?;
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
        {
            self.track_subscription_intent(&subscription)?;
            if self.listener_running.load(AtomicOrdering::SeqCst) {
                match self
                    .send_listener_command(SessionCommand {
                        kind: SessionCommandKind::Unsubscribe(subscription.clone()),
                        response: None,
                    })
                    .await
                {
                    Ok(SessionCommandResult::Done) | Ok(SessionCommandResult::Events(_)) => {
                        return Ok(());
                    }
                    Err(error) => {
                        if self.enqueue_recovery_request(subscription).is_ok() {
                            return Ok(());
                        }
                        return Err(error);
                    }
                }
            }
            if let Err(error) = self.ensure_connected_async().await {
                return Err(error);
            }
            let mut session = self.take_session_wait_async().await?;
            let result = async {
                send_packet_request_async(
                    &self.runtime,
                    &mut session,
                    &self.dvc.id,
                    subscription.clone(),
                )
                .await?;
                let _ = read_from_session_async(
                    &self.runtime,
                    &mut session,
                    &self.dvc.id,
                    Some(&self.negotiated_connack),
                    Some(&self.auth_handler),
                    Some(core::time::Duration::from_millis(MQTT_ACK_READ_TIMEOUT_MS)),
                )
                .await?;
                Ok::<(), String>(())
            }
            .await;
            if result.is_ok() {
                self.restore_session(session).await?;
            } else {
                self.clear_session().await?;
                if self.listener_running.load(AtomicOrdering::SeqCst) {
                    let _ = self.enqueue_recovery_request(subscription);
                    return Ok(());
                }
            }
            result?;
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
        {
            self.ensure_connected_async().await?;
            if self.listener_running.swap(true, AtomicOrdering::SeqCst) {
                return Err("MQTT listener already running".to_string());
            }

            let session = Shared::clone(&self.session);
            let running = Shared::clone(&self.listener_running);
            let listener_error = Shared::clone(&self.listener_error);
            let status_subscribers = Shared::clone(&self.listener_status_subscribers);
            let listener_command_sender = Shared::clone(&self.listener_command_sender);
            let negotiated_connack = Shared::clone(&self.negotiated_connack);
            let auth_handler = Shared::clone(&self.auth_handler);
            let device_id = self.dvc.id.clone();
            let driver = self.clone();
            let runtime = self.runtime.clone();
            let reconnect = self.reconnect_config()?;
            let (command_tx, command_rx): (
                RuntimeSender<SessionCommand>,
                RuntimeReceiver<SessionCommand>,
            ) = runtime.channel(MQTT_LISTENER_COMMAND_CHANNEL_CAPACITY);
            self.clear_listener_error()?;
            self.broadcast_listener_status(MqttListenerStatus::Running)?;
            {
                let mut sender_guard = self
                    .listener_command_sender
                    .lock()
                    .await
                    .map_err(|_| "failed to lock MQTT listener command sender".to_string())?;
                *sender_guard = Some(command_tx);
            }
            let handle = runtime.clone().spawn(async move {
                let mut sink = sink;
                let mut reconnect_attempt = 0u32;
                let mut command_rx = command_rx;

                'listen: while running.load(AtomicOrdering::SeqCst) {
                    let mut session_value = match {
                        let mut guard = match session.lock().await {
                            Ok(guard) => guard,
                            Err(_) => {
                                if let Ok(mut error_guard) = listener_error.lock().await {
                                    *error_guard = Some(
                                        "failed to lock MQTT session while listening".to_string(),
                                    );
                                }
                                broadcast_listener_status_to_subscribers(
                                        &status_subscribers,
                                        MqttListenerStatus::Failed(
                                            "failed to lock MQTT session while listening".to_string(),
                                        ),
                                ).await;
                                break;
                            }
                        };
                        guard.take()
                    } {
                        Some(session) => session,
                        None => {
                            runtime
                                .sleep(core::time::Duration::from_millis(MQTT_SESSION_WAIT_POLL_MS))
                                .await;
                            continue;
                        }
                    };

                    loop {
                        match command_rx.try_recv() {
                            Ok(command) => {
                                if let Err(error) =
                                    process_session_command(
                                        &runtime,
                                        &mut session_value,
                                        &device_id,
                                        &auth_handler,
                                        command,
                                    )
                                        .await
                                {
                                    if let Ok(mut guard) = session.lock().await {
                                        *guard = None;
                                    }
                                    let mut error = error;
                                    while running.load(AtomicOrdering::SeqCst) {
                                        reconnect_attempt = reconnect_attempt.saturating_add(1);
                                        if !reconnect.allows_attempt(reconnect_attempt) {
                                            break;
                                        }

                                        runtime
                                            .sleep(core::time::Duration::from_millis(
                                                reconnect.delay_ms_for_attempt(reconnect_attempt),
                                            ))
                                            .await;

                                        if !running.load(AtomicOrdering::SeqCst) {
                                            break;
                                        }

                                        match driver.ensure_connected_async().await {
                                            Ok(()) => {
                                                let maybe_reconnected_session = {
                                                    let mut guard = match session.lock().await {
                                                        Ok(guard) => guard,
                                                        Err(_) => {
                                                            error = "failed to lock MQTT session after reconnect".to_string();
                                                            continue;
                                                        }
                                                    };
                                                    guard.take()
                                                };

                                                let Some(mut reconnected_session) = maybe_reconnected_session else {
                                                    error = "MQTT session missing after reconnect".to_string();
                                                    continue;
                                                };

                                                let recovered_events = match driver
                                                    .replay_recovery_state_on_session_async(
                                                        &mut reconnected_session,
                                                    )
                                                    .await
                                                {
                                                    Ok(recovered_events) => recovered_events,
                                                    Err(recovery_error) => {
                                                        if let Ok(mut guard) = session.lock().await {
                                                            *guard = None;
                                                        }
                                                        error = recovery_error;
                                                        continue;
                                                    }
                                                };

                                                if let Ok(mut guard) = session.lock().await {
                                                    *guard = Some(reconnected_session);
                                                }
                                                for event in recovered_events {
                                                    if sink.handle(event).is_err() {
                                                        if let Ok(mut error_guard) = listener_error.lock().await
                                                        {
                                                            *error_guard = Some(
                                                                "failed to forward MQTT event to sink"
                                                                    .to_string(),
                                                            );
                                                        }
                                                        broadcast_listener_status_to_subscribers(
                                                            &status_subscribers,
                                                            MqttListenerStatus::Failed(
                                                                "failed to forward MQTT event to sink"
                                                                    .to_string(),
                                                            ),
                                                        ).await;
                                                        running.store(false, AtomicOrdering::SeqCst);
                                                        break 'listen;
                                                    }
                                                }
                                                reconnect_attempt = 0;
                                                if let Ok(mut error_guard) = listener_error.lock().await {
                                                    *error_guard = None;
                                                }
                                                continue 'listen;
                                            }
                                            Err(connect_error) => {
                                                error = connect_error;
                                            }
                                        }
                                    }
                                    if !running.load(AtomicOrdering::SeqCst) {
                                        break 'listen;
                                    }
                                    if let Ok(mut error_guard) = listener_error.lock().await {
                                        *error_guard = Some(error.clone());
                                    }
                                    broadcast_listener_status_to_subscribers(
                                        &status_subscribers,
                                        MqttListenerStatus::Failed(error),
                                    ).await;
                                    running.store(false, AtomicOrdering::SeqCst);
                                    break 'listen;
                                }
                            }
                            Err(ChannelError::Empty) => break,
                            Err(ChannelError::Closed) => break,
                            Err(ChannelError::Full | ChannelError::RuntimeUnavailable) => {
                                break;
                            }
                        }
                    }

                    let messages = read_from_session_async(
                        &runtime,
                        &mut session_value,
                        &device_id,
                        Some(&negotiated_connack),
                        Some(&auth_handler),
                        Some(core::time::Duration::from_millis(MQTT_LISTENER_IDLE_POLL_MS)),
                    )
                    .await;

                    match messages {
                        Ok(messages) => {
                            reconnect_attempt = 0;
                            if let Ok(mut guard) = session.lock().await {
                                *guard = Some(session_value);
                            }
                            for message in messages {
                                if let RoutedMessage::Event(event) = message {
                                    if sink.handle(event).is_err() {
                                        if let Ok(mut error_guard) = listener_error.lock().await {
                                            *error_guard = Some(
                                                "failed to forward MQTT event to sink".to_string(),
                                            );
                                        }
                                        broadcast_listener_status_to_subscribers(
                                            &status_subscribers,
                                            MqttListenerStatus::Failed(
                                                "failed to forward MQTT event to sink".to_string(),
                                            ),
                                        ).await;
                                        running.store(false, AtomicOrdering::SeqCst);
                                        break;
                                    }
                                }
                            }
                        }
                        Err(mut error) => {
                            if let Ok(mut guard) = session.lock().await {
                                *guard = None;
                            }
                            while running.load(AtomicOrdering::SeqCst) {
                                reconnect_attempt = reconnect_attempt.saturating_add(1);
                                if !reconnect.allows_attempt(reconnect_attempt) {
                                    break;
                                }

                                runtime
                                    .sleep(core::time::Duration::from_millis(
                                        reconnect.delay_ms_for_attempt(reconnect_attempt),
                                    ))
                                    .await;

                                if !running.load(AtomicOrdering::SeqCst) {
                                    break;
                                }

                                match driver.ensure_connected_async().await {
                                    Ok(()) => {
                                        let maybe_reconnected_session = {
                                            let mut guard = match session.lock().await {
                                                Ok(guard) => guard,
                                                Err(_) => {
                                                    error =
                                                        "failed to lock MQTT session after reconnect"
                                                            .to_string();
                                                    continue;
                                                }
                                            };
                                            guard.take()
                                        };

                                        let Some(mut reconnected_session) = maybe_reconnected_session else {
                                            error =
                                                "MQTT session missing after reconnect".to_string();
                                            continue;
                                        };

                                        if let Err(recovery_error) = driver
                                            .replay_recovery_state_on_session_async(
                                                &mut reconnected_session,
                                            )
                                            .await
                                        {
                                            if let Ok(mut guard) = session.lock().await {
                                                *guard = None;
                                            }
                                            error = recovery_error;
                                            continue;
                                        }

                                        if let Ok(mut guard) = session.lock().await {
                                            *guard = Some(reconnected_session);
                                        }
                                        reconnect_attempt = 0;
                                        if let Ok(mut error_guard) = listener_error.lock().await {
                                            *error_guard = None;
                                        }
                                        continue 'listen;
                                    }
                                    Err(connect_error) => {
                                        error = connect_error;
                                    }
                                }
                            }
                            if reconnect_attempt == 0 && running.load(AtomicOrdering::SeqCst) {
                                continue;
                            }
                            if !running.load(AtomicOrdering::SeqCst) {
                                break;
                            }
                            if let Ok(mut error_guard) = listener_error.lock().await {
                                *error_guard = Some(error.clone());
                            }
                            broadcast_listener_status_to_subscribers(
                                &status_subscribers,
                                MqttListenerStatus::Failed(error),
                            ).await;
                            running.store(false, AtomicOrdering::SeqCst);
                            break;
                        }
                    }
                }
                if let Ok(mut sender_guard) = listener_command_sender.lock().await {
                    *sender_guard = None;
                }
                running.store(false, AtomicOrdering::SeqCst);
                let final_status = match listener_error.lock().await {
                    Ok(error_guard) => match error_guard.clone() {
                        Some(error) => MqttListenerStatus::Failed(error),
                        None => MqttListenerStatus::Stopped,
                    },
                    Err(_) => MqttListenerStatus::Failed(
                        "failed to lock MQTT listener error after listener exit".to_string(),
                    ),
                };
                broadcast_listener_status_to_subscribers(&status_subscribers, final_status).await;
            });

            let mut handle_guard = self
                .listener_handle
                .lock()
                .await
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
        {
            return self.stop_listener();
        }

        #[cfg(not(feature = "std"))]
        Err("MQTT runtime requires the \"std\" feature".to_string())
    }
}

fn planner_message_for_command(
    command: Command,
) -> Result<ferredge_bridge::BridgeMessage<'static>, MqttCommandConversionError> {
    match command.intent {
        Intent::Send { .. } | Intent::Subscribe { .. } | Intent::Unsubscribe { .. } => {
            planner::command_to_messaging(command).map_err(Into::into)
        }
        _ => Err(MqttCommandConversionError::UnsupportedIntent),
    }
}

impl BridgeAdapter for MqttBridgeAdapter {
    type Error = MqttCommandConversionError;

    fn command_to_bridge(&self, command: Command) -> Result<BridgeMessage<'static>, Self::Error> {
        planner_message_for_command(command)
    }

    fn event_to_bridge(&self, event: RoutedEvent) -> Result<BridgeMessage<'static>, Self::Error> {
        Ok(planner::routed_event_to_bridge(event))
    }

    fn result_to_bridge(
        &self,
        result: RoutedResult,
    ) -> Result<BridgeMessage<'static>, Self::Error> {
        Ok(planner::routed_result_to_bridge(result))
    }
}
