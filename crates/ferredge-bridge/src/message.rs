use alloc::string::String;

use ferredge_core::prelude::{Correlation, DeliveryState, DeviceId, EndpointRef};
use serde::{Deserialize, Serialize};

use crate::{
    capability::BridgeCapability, fault::BridgeFault, meta::BridgeMeta, op::BridgeOp,
    payload::BridgePayload,
};

/// Top-level bridge message envelope shared across protocols.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum BridgeMessage {
    /// Outbound bridge command.
    Command(BridgeCommand),
    /// Inbound bridge event.
    Event(BridgeEvent),
    /// Inbound bridge result or completion.
    Result(BridgeResult),
    /// Inbound or synthesized bridge fault.
    Fault(BridgeFaultMessage),
}

/// Normalized outbound bridge command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BridgeCommand {
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
    /// Typed metadata associated with the command.
    pub meta: BridgeMeta,
    /// Optional correlation metadata.
    pub correlation: Option<Correlation>,
}

/// Normalized inbound bridge event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BridgeEvent {
    /// Source endpoint that emitted the event.
    pub source: EndpointRef,
    /// Capability family associated with the event.
    pub capability: BridgeCapability,
    /// Semantic operation represented by the event.
    pub operation: BridgeOp,
    /// Event payload.
    pub payload: BridgePayload,
    /// Typed metadata associated with the event.
    pub meta: BridgeMeta,
    /// Optional correlation metadata.
    pub correlation: Option<Correlation>,
}

/// Normalized bridge result or completion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum BridgeResult {
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
        /// Typed metadata associated with the result.
        meta: BridgeMeta,
        /// Optional correlation metadata.
        correlation: Option<Correlation>,
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
        /// Typed metadata associated with the result.
        meta: BridgeMeta,
        /// Optional correlation metadata.
        correlation: Option<Correlation>,
    },
    /// Failed command result with normalized bridge fault details.
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
        /// Typed metadata associated with the result.
        meta: BridgeMeta,
        /// Optional correlation metadata.
        correlation: Option<Correlation>,
        /// Normalized fault metadata for the failure.
        fault: BridgeFault,
    },
}

/// Bridge fault envelope with optional source and command context.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BridgeFaultMessage {
    /// Optional source endpoint where the fault originated.
    pub source: Option<EndpointRef>,
    /// Optional related command identifier.
    pub command_id: Option<String>,
    /// Optional correlation metadata.
    pub correlation: Option<Correlation>,
    /// Typed metadata associated with the fault.
    pub meta: BridgeMeta,
    /// Fault payload.
    pub fault: crate::fault::BridgeFault,
}
