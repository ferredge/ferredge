use alloc::borrow::Cow;

use ferredge_core::prelude::{DeliveryState, DeviceId, EndpointRef};
use serde::{Deserialize, Serialize};

use crate::{
    capability::BridgeCapability,
    fault::BridgeFault,
    meta::{BridgeCorrelation, BridgeHeaders, BridgeRoute, BridgeTransportMeta},
    op::BridgeOp,
    payload::BridgePayload,
};

/// Top-level bridge message envelope shared across protocols.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum BridgeMessage<'a> {
    /// Outbound bridge command.
    Command(BridgeCommand<'a>),
    /// Inbound bridge event.
    Event(BridgeEvent<'a>),
    /// Inbound bridge result or completion.
    Result(BridgeResult<'a>),
    /// Inbound or synthesized bridge fault.
    Fault(BridgeFaultMessage<'a>),
}

/// Normalized outbound bridge command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BridgeCommand<'a> {
    /// Stable command identifier.
    pub id: Cow<'a, str>,
    /// Optional originating device.
    pub source_device_id: Option<DeviceId>,
    /// Target device receiving the command.
    pub target_device_id: DeviceId,
    /// Capability family required by the command.
    pub capability: BridgeCapability,
    /// Semantic operation to perform.
    pub operation: BridgeOp,
    /// Optional payload carried by the command.
    pub payload: Option<BridgePayload<'a>>,
    /// Logical routing information required by the transport codec.
    pub route: BridgeRoute<'a>,
    /// Typed transport metadata preserved separately from arbitrary headers.
    pub transport: Option<BridgeTransportMeta<'a>>,
    /// Arbitrary protocol headers or properties.
    pub headers: Option<BridgeHeaders<'a>>,
    /// Optional correlation metadata.
    pub correlation: Option<BridgeCorrelation<'a>>,
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
    pub payload: BridgePayload<'a>,
    /// Logical routing information preserved for the event.
    pub route: BridgeRoute<'a>,
    /// Typed transport metadata preserved separately from arbitrary headers.
    pub transport: Option<BridgeTransportMeta<'a>>,
    /// Arbitrary protocol headers or properties.
    pub headers: Option<BridgeHeaders<'a>>,
    /// Optional correlation metadata.
    pub correlation: Option<BridgeCorrelation<'a>>,
}

/// Normalized bridge result or completion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum BridgeResult<'a> {
    /// Delivery-progress update that has not completed yet.
    Progress {
        /// Source endpoint that produced the result.
        source: EndpointRef,
        /// Identifier of the command this result belongs to.
        command_id: Cow<'a, str>,
        /// Delivery state represented by this progress update.
        state: DeliveryState,
        /// Optional capability context for the result.
        capability: Option<BridgeCapability>,
        /// Optional operation context for the result.
        operation: Option<BridgeOp>,
        /// Optional logical route associated with the result.
        route: Option<BridgeRoute<'a>>,
        /// Optional typed transport metadata.
        transport: Option<BridgeTransportMeta<'a>>,
        /// Optional arbitrary headers or properties.
        headers: Option<BridgeHeaders<'a>>,
        /// Optional correlation metadata.
        correlation: Option<BridgeCorrelation<'a>>,
    },
    /// Successful completed command result.
    Success {
        /// Source endpoint that produced the result.
        source: EndpointRef,
        /// Identifier of the command this result belongs to.
        command_id: Cow<'a, str>,
        /// Optional capability context for the result.
        capability: Option<BridgeCapability>,
        /// Optional operation context for the result.
        operation: Option<BridgeOp>,
        /// Optional result payload.
        payload: Option<BridgePayload<'a>>,
        /// Optional logical route associated with the result.
        route: Option<BridgeRoute<'a>>,
        /// Optional typed transport metadata.
        transport: Option<BridgeTransportMeta<'a>>,
        /// Optional arbitrary headers or properties.
        headers: Option<BridgeHeaders<'a>>,
        /// Optional correlation metadata.
        correlation: Option<BridgeCorrelation<'a>>,
    },
    /// Failed command result with normalized fault details.
    Failure {
        /// Source endpoint that produced the result.
        source: EndpointRef,
        /// Identifier of the command this result belongs to.
        command_id: Cow<'a, str>,
        /// Failure state represented by this result.
        state: DeliveryState,
        /// Optional capability context for the result.
        capability: Option<BridgeCapability>,
        /// Optional operation context for the result.
        operation: Option<BridgeOp>,
        /// Optional result payload.
        payload: Option<BridgePayload<'a>>,
        /// Optional human-readable error detail.
        error: Option<Cow<'a, str>>,
        /// Optional logical route associated with the result.
        route: Option<BridgeRoute<'a>>,
        /// Optional typed transport metadata.
        transport: Option<BridgeTransportMeta<'a>>,
        /// Optional arbitrary headers or properties.
        headers: Option<BridgeHeaders<'a>>,
        /// Optional correlation metadata.
        correlation: Option<BridgeCorrelation<'a>>,
        /// Normalized fault metadata for the failure.
        fault: BridgeFault,
    },
}

/// Bridge fault envelope with optional source and command context.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BridgeFaultMessage<'a> {
    /// Optional source endpoint where the fault originated.
    pub source: Option<EndpointRef>,
    /// Optional related command identifier.
    pub command_id: Option<Cow<'a, str>>,
    /// Optional correlation metadata.
    pub correlation: Option<BridgeCorrelation<'a>>,
    /// Optional route metadata associated with the fault.
    pub route: Option<BridgeRoute<'a>>,
    /// Optional typed transport metadata associated with the fault.
    pub transport: Option<BridgeTransportMeta<'a>>,
    /// Optional arbitrary headers or properties associated with the fault.
    pub headers: Option<BridgeHeaders<'a>>,
    /// Fault payload.
    pub fault: crate::fault::BridgeFault,
}

impl BridgeMessage<'_> {
    pub fn into_owned(self) -> BridgeMessage<'static> {
        match self {
            BridgeMessage::Command(command) => BridgeMessage::Command(command.into_owned()),
            BridgeMessage::Event(event) => BridgeMessage::Event(event.into_owned()),
            BridgeMessage::Result(result) => BridgeMessage::Result(result.into_owned()),
            BridgeMessage::Fault(fault) => BridgeMessage::Fault(fault.into_owned()),
        }
    }
}

impl BridgeCommand<'_> {
    pub fn into_owned(self) -> BridgeCommand<'static> {
        BridgeCommand {
            id: Cow::Owned(self.id.into_owned()),
            source_device_id: self.source_device_id,
            target_device_id: self.target_device_id,
            capability: self.capability,
            operation: self.operation,
            payload: self.payload.map(BridgePayload::into_owned),
            route: self.route.into_owned(),
            transport: self.transport.map(BridgeTransportMeta::into_owned),
            headers: self.headers.map(BridgeHeaders::into_owned),
            correlation: self.correlation.map(BridgeCorrelation::into_owned),
        }
    }
}

impl BridgeEvent<'_> {
    pub fn into_owned(self) -> BridgeEvent<'static> {
        BridgeEvent {
            source: self.source,
            capability: self.capability,
            operation: self.operation,
            payload: self.payload.into_owned(),
            route: self.route.into_owned(),
            transport: self.transport.map(BridgeTransportMeta::into_owned),
            headers: self.headers.map(BridgeHeaders::into_owned),
            correlation: self.correlation.map(BridgeCorrelation::into_owned),
        }
    }
}

impl BridgeResult<'_> {
    pub fn into_owned(self) -> BridgeResult<'static> {
        match self {
            BridgeResult::Progress {
                source,
                command_id,
                state,
                capability,
                operation,
                route,
                transport,
                headers,
                correlation,
            } => BridgeResult::Progress {
                source,
                command_id: Cow::Owned(command_id.into_owned()),
                state,
                capability,
                operation,
                route: route.map(BridgeRoute::into_owned),
                transport: transport.map(BridgeTransportMeta::into_owned),
                headers: headers.map(BridgeHeaders::into_owned),
                correlation: correlation.map(BridgeCorrelation::into_owned),
            },
            BridgeResult::Success {
                source,
                command_id,
                capability,
                operation,
                payload,
                route,
                transport,
                headers,
                correlation,
            } => BridgeResult::Success {
                source,
                command_id: Cow::Owned(command_id.into_owned()),
                capability,
                operation,
                payload: payload.map(BridgePayload::into_owned),
                route: route.map(BridgeRoute::into_owned),
                transport: transport.map(BridgeTransportMeta::into_owned),
                headers: headers.map(BridgeHeaders::into_owned),
                correlation: correlation.map(BridgeCorrelation::into_owned),
            },
            BridgeResult::Failure {
                source,
                command_id,
                state,
                capability,
                operation,
                payload,
                error,
                route,
                transport,
                headers,
                correlation,
                fault,
            } => BridgeResult::Failure {
                source,
                command_id: Cow::Owned(command_id.into_owned()),
                state,
                capability,
                operation,
                payload: payload.map(BridgePayload::into_owned),
                error: error.map(|value| Cow::Owned(value.into_owned())),
                route: route.map(BridgeRoute::into_owned),
                transport: transport.map(BridgeTransportMeta::into_owned),
                headers: headers.map(BridgeHeaders::into_owned),
                correlation: correlation.map(BridgeCorrelation::into_owned),
                fault,
            },
        }
    }
}

impl BridgeFaultMessage<'_> {
    pub fn into_owned(self) -> BridgeFaultMessage<'static> {
        BridgeFaultMessage {
            source: self.source,
            command_id: self.command_id.map(|value| Cow::Owned(value.into_owned())),
            correlation: self.correlation.map(BridgeCorrelation::into_owned),
            route: self.route.map(BridgeRoute::into_owned),
            transport: self.transport.map(BridgeTransportMeta::into_owned),
            headers: self.headers.map(BridgeHeaders::into_owned),
            fault: self.fault,
        }
    }
}
