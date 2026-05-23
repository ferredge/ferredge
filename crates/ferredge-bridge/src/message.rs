use alloc::string::String;

use ferredge_core::prelude::{Correlation, DeliveryState, DeviceId, EndpointRef};
use serde::{Deserialize, Serialize};

use crate::{capability::BridgeCapability, meta::BridgeMeta, op::BridgeOp, payload::BridgePayload};

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
pub struct BridgeResult {
    /// Source endpoint that produced the result.
    pub source: EndpointRef,
    /// Identifier of the command this result belongs to.
    pub command_id: String,
    /// Delivery or completion state.
    pub state: DeliveryState,
    /// Optional capability context for the result.
    pub capability: Option<BridgeCapability>,
    /// Optional operation context for the result.
    pub operation: Option<BridgeOp>,
    /// Optional result payload.
    pub payload: Option<BridgePayload>,
    /// Typed metadata associated with the result.
    pub meta: BridgeMeta,
    /// Optional correlation metadata.
    pub correlation: Option<Correlation>,
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
