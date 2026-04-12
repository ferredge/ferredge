use crate::command::{Command, CommandResult};
use crate::device::{Device, DeviceResourceAttributes};
use crate::routed::{RoutedEvent, RoutedMessage, RoutedResult};

/// Receives protocol-native or routed events emitted by drivers.
pub trait EventSink: Send {
    /// Event type accepted by this sink.
    type Event;
    /// Error returned when sink processing fails.
    type Error;

    /// Handles one event emitted from driver ingress.
    fn handle(&mut self, event: Self::Event) -> Result<(), Self::Error>;
}

/// Lifecycle hooks shared by all protocol drivers.
pub trait Lifecycle: Send + Sync {
    /// Driver-specific startup or shutdown error.
    type Error;

    /// Starts protocol resources such as connections, polling loops, or subscriptions.
    fn start(&self) -> impl Future<Output = Result<(), Self::Error>> + Send;
    /// Stops protocol resources and performs cleanup.
    fn stop(&self) -> impl Future<Output = Result<(), Self::Error>> + Send;
}

/// Capability for request/response style protocols such as HTTP or Modbus.
pub trait RequestResponse: Send + Sync {
    /// Native outbound request type understood by concrete driver.
    type Request;
    /// Native response type returned by concrete driver.
    type Response;
    /// Driver-specific request execution error.
    type Error;

    /// Executes one native request against underlying transport.
    fn execute(
        &self,
        request: Self::Request,
    ) -> impl Future<Output = Result<Self::Response, Self::Error>> + Send;
}

/// Capability for drivers that can emit unsolicited inbound events.
pub trait EventSource: Send + Sync {
    /// Event emitted by protocol ingress.
    type Event;
    /// Driver-specific listening error.
    type Error;

    /// Starts listening and forwards inbound events into provided sink.
    fn start_listening<S>(&self, sink: S) -> impl Future<Output = Result<(), Self::Error>> + Send
    where
        S: EventSink<Event = Self::Event> + Send + 'static;

    /// Stops active listening loop without requiring full driver shutdown.
    fn stop_listening(&self) -> impl Future<Output = Result<(), Self::Error>> + Send;
}

/// Capability for publish/subscribe protocols such as MQTT.
pub trait PubSub: Send + Sync {
    /// Native publish request type.
    type PublishRequest;
    /// Native subscription descriptor type.
    type Subscription;
    /// Driver-specific pub/sub error.
    type Error;

    /// Publishes one message to transport.
    fn publish(
        &self,
        request: Self::PublishRequest,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    /// Registers one subscription and forwards matching messages into provided sink.
    fn subscribe<S>(
        &self,
        subscription: Self::Subscription,
        sink: S,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send
    where
        S: EventSink<Event = RoutedEvent> + Send;

    /// Removes one subscription from transport.
    fn unsubscribe(
        &self,
        subscription: Self::Subscription,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;
}

/// Protocol-neutral router for commands, events, and results.
pub trait Router: Send + Sync {
    /// Router-specific error type.
    type Error;

    /// Registers one device and its metadata with router state.
    fn register_device<T>(
        &self,
        device: Device<T>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send
    where
        T: DeviceResourceAttributes;

    /// Routes already-normalized message through router pipeline.
    fn route_message(
        &self,
        message: RoutedMessage,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    /// Routes one command and returns command-level result state.
    fn route_command(
        &self,
        command: Command,
    ) -> impl Future<Output = Result<CommandResult, Self::Error>> + Send;

    /// Handles one routed event emitted from protocol ingress.
    fn handle_event(
        &self,
        event: RoutedEvent,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    /// Handles one routed result or completion update.
    fn handle_result(
        &self,
        result: RoutedResult,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;
}
