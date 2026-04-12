use crate::{
    command::Command,
    routed::{RoutedEvent, RoutedMessage, RoutedResult},
};

/// Emits routed actions produced by bridge logic without requiring allocation.
pub trait ActionEmitter {
    /// Error returned if consumer cannot accept one emitted action.
    type Error;

    /// Emits one routed action produced by bridge evaluation.
    fn emit(&mut self, action: RoutedMessage) -> Result<(), Self::Error>;
}

/// Translates routed commands, events, and results across protocol boundaries.
pub trait ProtocolBridge: Send + Sync {
    /// Bridge-specific translation error.
    type Error;

    /// Translates one routed command into zero or more routed actions.
    fn bridge_command<E>(
        &self,
        command: &Command,
        emitter: &mut E,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send
    where
        E: ActionEmitter + Send;

    /// Translates one routed event into zero or more routed actions.
    fn bridge_event<E>(
        &self,
        event: &RoutedEvent,
        emitter: &mut E,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send
    where
        E: ActionEmitter + Send;

    /// Translates one routed result into zero or more routed actions.
    fn bridge_result<E>(
        &self,
        result: &RoutedResult,
        emitter: &mut E,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send
    where
        E: ActionEmitter + Send;
}
