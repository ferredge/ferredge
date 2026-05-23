use alloc::{string::String, vec::Vec};

use ferredge_core::prelude::PayloadValue;
use serde::{Deserialize, Serialize};

/// Efficient protocol-neutral payload carrier for bridge messages.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum BridgePayload {
    /// Explicitly empty payload.
    Empty,
    /// Scalar payload.
    Scalar(BridgeScalar),
    /// UTF-8 text payload.
    Text(String),
    /// Opaque binary payload.
    Binary(Vec<u8>),
    /// Ordered heterogeneous payload sequence.
    Sequence(Vec<BridgePayload>),
    /// Ordered object payload.
    Object(Vec<(String, BridgePayload)>),
}

/// Scalar payload supported by the bridge layer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum BridgeScalar {
    /// Boolean scalar.
    Bool(bool),
    /// Signed integer scalar.
    I64(i64),
    /// Unsigned integer scalar.
    U64(u64),
    /// Floating-point scalar.
    F64(f64),
}

impl From<PayloadValue> for BridgePayload {
    fn from(value: PayloadValue) -> Self {
        match value {
            PayloadValue::Null => Self::Empty,
            PayloadValue::Bool(value) => Self::Scalar(BridgeScalar::Bool(value)),
            PayloadValue::I64(value) => Self::Scalar(BridgeScalar::I64(value)),
            PayloadValue::U64(value) => Self::Scalar(BridgeScalar::U64(value)),
            PayloadValue::F64(value) => Self::Scalar(BridgeScalar::F64(value)),
            PayloadValue::String(value) => Self::Text(value),
            PayloadValue::Bytes(value) => Self::Binary(value),
            PayloadValue::List(values) => {
                Self::Sequence(values.into_iter().map(Self::from).collect())
            }
            PayloadValue::Map(values) => Self::Object(
                values
                    .into_iter()
                    .map(|(key, value)| (key, Self::from(value)))
                    .collect(),
            ),
        }
    }
}

impl From<BridgePayload> for PayloadValue {
    fn from(value: BridgePayload) -> Self {
        match value {
            BridgePayload::Empty => Self::Null,
            BridgePayload::Scalar(BridgeScalar::Bool(value)) => Self::Bool(value),
            BridgePayload::Scalar(BridgeScalar::I64(value)) => Self::I64(value),
            BridgePayload::Scalar(BridgeScalar::U64(value)) => Self::U64(value),
            BridgePayload::Scalar(BridgeScalar::F64(value)) => Self::F64(value),
            BridgePayload::Text(value) => Self::String(value),
            BridgePayload::Binary(value) => Self::Bytes(value),
            BridgePayload::Sequence(values) => {
                Self::List(values.into_iter().map(PayloadValue::from).collect())
            }
            BridgePayload::Object(values) => Self::Map(
                values
                    .into_iter()
                    .map(|(key, value)| (key, PayloadValue::from(value)))
                    .collect(),
            ),
        }
    }
}
