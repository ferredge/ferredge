use crate::maybe::{MaybeSend, MaybeSync};
use crate::routed::RoutedEvent;

/// Receives protocol-native or routed events emitted by drivers.
pub trait EventSink: MaybeSend {
    /// Event type accepted by this sink.
    type Event;
    /// Error returned when sink processing fails.
    type Error;

    /// Handles one event emitted from driver ingress.
    fn handle(&mut self, event: Self::Event) -> Result<(), Self::Error>;
}

/// Lifecycle hooks shared by all protocol drivers.
pub trait Lifecycle: MaybeSend + MaybeSync {
    /// Driver-specific startup or shutdown error.
    type Error;

    /// Starts protocol resources such as connections, polling loops, or subscriptions.
    fn start(&self) -> impl Future<Output = Result<(), Self::Error>> + MaybeSend;
    /// Stops protocol resources and performs cleanup.
    fn stop(&self) -> impl Future<Output = Result<(), Self::Error>> + MaybeSend;
}

/// Capability for request/response style protocols such as HTTP or Modbus.
pub trait RequestResponse: MaybeSend + MaybeSync {
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
    ) -> impl Future<Output = Result<Self::Response, Self::Error>> + MaybeSend;
}

/// Capability for drivers that can emit unsolicited inbound events.
pub trait EventSource: MaybeSend + MaybeSync {
    /// Event emitted by protocol ingress.
    type Event;
    /// Driver-specific listening error.
    type Error;

    /// Starts listening and forwards inbound events into provided sink.
    fn start_listening<S>(
        &self,
        sink: S,
    ) -> impl Future<Output = Result<(), Self::Error>> + MaybeSend
    where
        S: EventSink<Event = Self::Event> + MaybeSend + 'static;

    /// Stops active listening loop without requiring full driver shutdown.
    fn stop_listening(&self) -> impl Future<Output = Result<(), Self::Error>> + MaybeSend;
}

/// Capability for publish/subscribe protocols such as MQTT.
pub trait PubSub: MaybeSend + MaybeSync {
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
    ) -> impl Future<Output = Result<(), Self::Error>> + MaybeSend;

    /// Registers one subscription and forwards matching messages into provided sink.
    fn subscribe<S>(
        &self,
        subscription: Self::Subscription,
        sink: S,
    ) -> impl Future<Output = Result<(), Self::Error>> + MaybeSend
    where
        S: EventSink<Event = RoutedEvent<'static>> + MaybeSend;

    /// Removes one subscription from transport.
    fn unsubscribe(
        &self,
        subscription: Self::Subscription,
    ) -> impl Future<Output = Result<(), Self::Error>> + MaybeSend;
}
