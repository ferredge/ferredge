extern crate alloc;

use serde::{Deserialize, Serialize};

use crate::{
    command::{Command, CommandResult, Correlation, PayloadValue},
    id::DeviceId,
    meta::{ProtocolId, TransportMeta},
    owned::IntoOwned,
    route::Route,
};

/// Identifies concrete source or target transport endpoint inside router graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct EndpointRef {
    /// Device identifier registered with router.
    pub device_id: DeviceId,
    /// Transport protocol used by device endpoint.
    pub protocol: ProtocolId,
}

impl EndpointRef {
    /// Creates an endpoint reference for the given device and protocol.
    pub fn new(device_id: DeviceId, protocol: ProtocolId) -> Self {
        Self {
            device_id,
            protocol,
        }
    }
}

/// Normalized inbound event emitted by one protocol adapter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RoutedEvent<'a> {
    /// Source endpoint that emitted this event.
    pub source: EndpointRef,
    /// Logical route targeted by event, such as resource or topic.
    pub route: Route<'a>,
    /// Typed event payload preserved across routing.
    pub payload: PayloadValue<'a>,
    /// Optional request correlation for reply-topic or async workflows.
    pub correlation: Option<Correlation<'a>>,
    /// Optional transport metadata capture preserved from source protocol.
    pub transport: Option<TransportMeta<'a>>,
}

impl<'a> RoutedEvent<'a> {
    /// Creates a routed event from the given source over the given route.
    pub fn new(
        source: EndpointRef,
        route: Route<'a>,
        payload: PayloadValue<'a>,
        correlation: Option<Correlation<'a>>,
        transport: Option<TransportMeta<'a>>,
    ) -> Self {
        Self {
            source,
            route,
            payload,
            correlation,
            transport,
        }
    }

    /// Materializes an owned routed event for queues or persistence.
    pub fn into_owned(self) -> RoutedEvent<'static> {
        IntoOwned::into_owned(self)
    }
}

impl IntoOwned for RoutedEvent<'_> {
    type Owned = RoutedEvent<'static>;
    fn into_owned(self) -> Self::Owned {
        RoutedEvent {
            source: self.source,
            route: self.route.into_owned(),
            payload: self.payload.into_owned(),
            correlation: IntoOwned::into_owned(self.correlation),
            transport: IntoOwned::into_owned(self.transport),
        }
    }
}

/// Normalized command result or transport completion emitted by one adapter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RoutedResult<'a> {
    /// Source endpoint that produced this result.
    pub source: EndpointRef,
    /// Protocol-neutral command result payload.
    pub result: CommandResult<'a>,
    /// Optional transport metadata capture preserved from source protocol.
    pub transport: Option<TransportMeta<'a>>,
}

impl<'a> RoutedResult<'a> {
    /// Creates a routed result from the given source.
    pub fn new(
        source: EndpointRef,
        result: CommandResult<'a>,
        transport: Option<TransportMeta<'a>>,
    ) -> Self {
        Self {
            source,
            result,
            transport,
        }
    }

    /// Materializes an owned routed result for queues or persistence.
    pub fn into_owned(self) -> RoutedResult<'static> {
        IntoOwned::into_owned(self)
    }
}

impl IntoOwned for RoutedResult<'_> {
    type Owned = RoutedResult<'static>;
    fn into_owned(self) -> Self::Owned {
        RoutedResult {
            source: self.source,
            result: self.result.into_owned(),
            transport: IntoOwned::into_owned(self.transport),
        }
    }
}

/// Unified message envelope accepted by router and bridge layers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum RoutedMessage<'a> {
    /// Routed command envelope.
    Command(Command<'a>),
    /// Routed event envelope.
    Event(RoutedEvent<'a>),
    /// Routed result envelope.
    Result(RoutedResult<'a>),
}

impl RoutedMessage<'_> {
    /// Materializes an owned routed message for async boundaries.
    pub fn into_owned(self) -> RoutedMessage<'static> {
        IntoOwned::into_owned(self)
    }
}

impl IntoOwned for RoutedMessage<'_> {
    type Owned = RoutedMessage<'static>;
    fn into_owned(self) -> Self::Owned {
        match self {
            Self::Command(command) => RoutedMessage::Command(command.into_owned()),
            Self::Event(event) => RoutedMessage::Event(event.into_owned()),
            Self::Result(result) => RoutedMessage::Result(result.into_owned()),
        }
    }
}
