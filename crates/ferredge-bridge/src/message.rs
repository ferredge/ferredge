use alloc::string::String;

use ferredge_core::prelude::{
    CommandId, Correlation, DeliveryState, DeviceId, EndpointRef, Headers, IntoOwned, PayloadValue,
    Route, TransportMeta,
};
use serde::{Deserialize, Serialize};

use crate::{capability::BridgeCapability, fault::BridgeFault, op::BridgeOp};

/// Top-level bridge message envelope shared across protocols.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum BridgeMessage<'a> {
    /// Outbound bridge command.
    Command(BridgeCommand<'a>),
    /// Inbound bridge event.
    Event(BridgeEvent<'a>),
    /// Inbound bridge result or completion.
    Result(BridgeResult<'a>),
}

/// Normalized outbound bridge command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BridgeCommand<'a> {
    /// Stable command identifier.
    pub id: CommandId,
    /// Optional originating device.
    pub source_device_id: Option<DeviceId>,
    /// Target device receiving the command.
    pub target_device_id: DeviceId,
    /// Capability family required by the command.
    pub capability: BridgeCapability,
    /// Semantic operation to perform.
    pub operation: BridgeOp,
    /// Optional payload carried by the command.
    pub payload: Option<PayloadValue<'a>>,
    /// Logical routing information required by the transport codec.
    pub route: Route<'a>,
    /// Typed transport metadata preserved separately from arbitrary headers.
    pub transport: Option<TransportMeta<'a>>,
    /// Arbitrary protocol headers or properties.
    pub headers: Option<Headers<'a>>,
    /// Optional correlation metadata.
    pub correlation: Option<Correlation<'a>>,
}

/// Normalized inbound bridge event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BridgeEvent<'a> {
    /// Source endpoint that emitted the event.
    pub source: EndpointRef,
    /// Capability family associated with the event.
    pub capability: BridgeCapability,
    /// Semantic operation represented by the event.
    pub operation: BridgeOp,
    /// Event payload.
    pub payload: PayloadValue<'a>,
    /// Logical routing information preserved for the event.
    pub route: Route<'a>,
    /// Typed transport metadata preserved separately from arbitrary headers.
    pub transport: Option<TransportMeta<'a>>,
    /// Arbitrary protocol headers or properties.
    pub headers: Option<Headers<'a>>,
    /// Optional correlation metadata.
    pub correlation: Option<Correlation<'a>>,
}

/// Shared context metadata carried by every bridge result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BridgeMessageMeta<'a> {
    /// Source endpoint that produced the result.
    pub source: EndpointRef,
    /// Identifier of the command this result belongs to.
    pub command_id: CommandId,
    /// Optional capability context for the result.
    pub capability: Option<BridgeCapability>,
    /// Optional operation context for the result.
    pub operation: Option<BridgeOp>,
    /// Optional typed transport metadata.
    pub transport: Option<TransportMeta<'a>>,
    /// Optional arbitrary headers or properties.
    pub headers: Option<Headers<'a>>,
    /// Optional correlation metadata.
    pub correlation: Option<Correlation<'a>>,
}

/// Normalized bridge result or completion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BridgeResult<'a> {
    /// Shared result context metadata.
    pub meta: BridgeMessageMeta<'a>,
    /// Lifecycle outcome represented by this result.
    pub outcome: BridgeOutcome<'a>,
}

/// Lifecycle outcome carried by a bridge result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum BridgeOutcome<'a> {
    /// Delivery-progress update that has not completed yet.
    Progress {
        /// Delivery state represented by this progress update.
        state: DeliveryState,
    },
    /// Successful completed command result.
    Success {
        /// Optional result payload.
        payload: Option<PayloadValue<'a>>,
    },
    /// Failed command result with normalized fault details.
    Failure {
        /// Failure state represented by this result.
        state: DeliveryState,
        /// Optional result payload.
        payload: Option<PayloadValue<'a>>,
        /// Optional human-readable error detail.
        error: Option<String>,
        /// Normalized fault metadata for the failure.
        fault: BridgeFault,
    },
}

impl BridgeMessage<'_> {
    /// Materializes an owned bridge message for async boundaries.
    pub fn into_owned(self) -> BridgeMessage<'static> {
        IntoOwned::into_owned(self)
    }
}

impl IntoOwned for BridgeMessage<'_> {
    type Owned = BridgeMessage<'static>;
    fn into_owned(self) -> Self::Owned {
        match self {
            BridgeMessage::Command(command) => BridgeMessage::Command(command.into_owned()),
            BridgeMessage::Event(event) => BridgeMessage::Event(event.into_owned()),
            BridgeMessage::Result(result) => BridgeMessage::Result(result.into_owned()),
        }
    }
}

impl BridgeCommand<'_> {
    /// Materializes an owned bridge command for async boundaries.
    pub fn into_owned(self) -> BridgeCommand<'static> {
        IntoOwned::into_owned(self)
    }
}

impl IntoOwned for BridgeCommand<'_> {
    type Owned = BridgeCommand<'static>;
    fn into_owned(self) -> Self::Owned {
        BridgeCommand {
            id: self.id,
            source_device_id: self.source_device_id,
            target_device_id: self.target_device_id,
            capability: self.capability,
            operation: self.operation,
            payload: self.payload.into_owned(),
            route: self.route.into_owned(),
            transport: self.transport.into_owned(),
            headers: self.headers.into_owned(),
            correlation: self.correlation.into_owned(),
        }
    }
}

impl BridgeEvent<'_> {
    /// Materializes an owned bridge event for async boundaries.
    pub fn into_owned(self) -> BridgeEvent<'static> {
        IntoOwned::into_owned(self)
    }
}

impl IntoOwned for BridgeEvent<'_> {
    type Owned = BridgeEvent<'static>;
    fn into_owned(self) -> Self::Owned {
        BridgeEvent {
            source: self.source,
            capability: self.capability,
            operation: self.operation,
            payload: self.payload.into_owned(),
            route: self.route.into_owned(),
            transport: self.transport.into_owned(),
            headers: self.headers.into_owned(),
            correlation: self.correlation.into_owned(),
        }
    }
}

impl BridgeMessageMeta<'_> {
    /// Materializes owned bridge result metadata for async boundaries.
    pub fn into_owned(self) -> BridgeMessageMeta<'static> {
        IntoOwned::into_owned(self)
    }
}

impl IntoOwned for BridgeMessageMeta<'_> {
    type Owned = BridgeMessageMeta<'static>;
    fn into_owned(self) -> Self::Owned {
        BridgeMessageMeta {
            source: self.source,
            command_id: self.command_id,
            capability: self.capability,
            operation: self.operation,
            transport: self.transport.into_owned(),
            headers: self.headers.into_owned(),
            correlation: self.correlation.into_owned(),
        }
    }
}

impl BridgeResult<'_> {
    /// Materializes an owned bridge result for async boundaries.
    pub fn into_owned(self) -> BridgeResult<'static> {
        IntoOwned::into_owned(self)
    }
}

impl IntoOwned for BridgeResult<'_> {
    type Owned = BridgeResult<'static>;
    fn into_owned(self) -> Self::Owned {
        BridgeResult {
            meta: self.meta.into_owned(),
            outcome: self.outcome.into_owned(),
        }
    }
}

impl BridgeOutcome<'_> {
    /// Materializes an owned bridge outcome for async boundaries.
    pub fn into_owned(self) -> BridgeOutcome<'static> {
        IntoOwned::into_owned(self)
    }
}

impl IntoOwned for BridgeOutcome<'_> {
    type Owned = BridgeOutcome<'static>;
    fn into_owned(self) -> Self::Owned {
        match self {
            BridgeOutcome::Progress { state } => BridgeOutcome::Progress { state },
            BridgeOutcome::Success { payload } => BridgeOutcome::Success {
                payload: payload.into_owned(),
            },
            BridgeOutcome::Failure {
                state,
                payload,
                error,
                fault,
            } => BridgeOutcome::Failure {
                state,
                payload: payload.into_owned(),
                error,
                fault,
            },
        }
    }
}
