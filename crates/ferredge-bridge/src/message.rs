use alloc::string::String;

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
    pub id: String,
    /// Optional originating device.
    pub source_device_id: Option<DeviceId>,
    /// Target device receiving the command.
    pub target_device_id: DeviceId,
    /// Capability family required by the command.
    pub capability: BridgeCapability,
    /// Semantic operation to perform.
    pub operation: BridgeOp,
    /// Optional payload carried by the command.
    pub payload: Option<BridgePayload>,
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
    pub payload: BridgePayload,
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
        command_id: String,
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
        command_id: String,
        /// Optional capability context for the result.
        capability: Option<BridgeCapability>,
        /// Optional operation context for the result.
        operation: Option<BridgeOp>,
        /// Optional result payload.
        payload: Option<BridgePayload>,
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
        command_id: String,
        /// Failure state represented by this result.
        state: DeliveryState,
        /// Optional capability context for the result.
        capability: Option<BridgeCapability>,
        /// Optional operation context for the result.
        operation: Option<BridgeOp>,
        /// Optional result payload.
        payload: Option<BridgePayload>,
        /// Optional human-readable error detail.
        error: Option<String>,
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
    pub command_id: Option<String>,
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
