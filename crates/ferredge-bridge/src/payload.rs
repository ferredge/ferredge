use alloc::{borrow::Cow, string::String, vec::Vec};

use ferredge_core::prelude::PayloadValue;
use serde::{Deserialize, Serialize};

/// Efficient protocol-neutral payload carrier for bridge messages.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum BridgePayload<'a> {
    /// Explicitly empty payload.
    Empty,
    /// Scalar payload.
    Scalar(BridgeScalar),
    /// UTF-8 text payload.
    Text(Cow<'a, str>),
    /// Opaque binary payload.
    Binary(Cow<'a, [u8]>),
    /// Ordered heterogeneous payload sequence.
    Sequence(Vec<BridgePayload<'a>>),
    /// Ordered object payload.
    Object(Vec<(Cow<'a, str>, BridgePayload<'a>)>),
}

impl BridgePayload<'_> {
    /// Re-borrows this payload without forcing ownership changes.
    pub fn as_borrowed(&self) -> BridgePayload<'_> {
        match self {
            Self::Empty => BridgePayload::Empty,
            Self::Scalar(value) => BridgePayload::Scalar(value.clone()),
            Self::Text(value) => BridgePayload::Text(Cow::Borrowed(value.as_ref())),
            Self::Binary(value) => BridgePayload::Binary(Cow::Borrowed(value.as_ref())),
            Self::Sequence(values) => {
                BridgePayload::Sequence(values.iter().map(BridgePayload::as_borrowed).collect())
            }
            Self::Object(values) => BridgePayload::Object(
                values
                    .iter()
                    .map(|(key, value)| (Cow::Borrowed(key.as_ref()), value.as_borrowed()))
                    .collect(),
            ),
        }
    }

    /// Materializes an owned bridge payload for async boundaries.
    pub fn into_owned(self) -> BridgePayload<'static> {
        match self {
            Self::Empty => BridgePayload::Empty,
            Self::Scalar(value) => BridgePayload::Scalar(value),
            Self::Text(value) => BridgePayload::Text(Cow::Owned(value.into_owned())),
            Self::Binary(value) => BridgePayload::Binary(Cow::Owned(value.into_owned())),
            Self::Sequence(values) => {
                BridgePayload::Sequence(values.into_iter().map(BridgePayload::into_owned).collect())
            }
            Self::Object(values) => BridgePayload::Object(
                values
                    .into_iter()
                    .map(|(key, value)| (Cow::Owned(key.into_owned()), value.into_owned()))
                    .collect(),
            ),
        }
    }
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

impl<'a> From<PayloadValue<'a>> for BridgePayload<'a> {
    fn from(value: PayloadValue<'a>) -> Self {
        match value {
            PayloadValue::Null => Self::Empty,
            PayloadValue::Bool(value) => Self::Scalar(BridgeScalar::Bool(value)),
            PayloadValue::I64(value) => Self::Scalar(BridgeScalar::I64(value)),
            PayloadValue::U64(value) => Self::Scalar(BridgeScalar::U64(value)),
            PayloadValue::F64(value) => Self::Scalar(BridgeScalar::F64(value)),
            PayloadValue::String(value) => Self::Text(value),
            PayloadValue::Bytes(value) => Self::Binary(value),
            PayloadValue::List(values) => {
                Self::Sequence(values.into_owned().into_iter().map(Self::from).collect())
            }
            PayloadValue::Map(values) => Self::Object(
                values
                    .into_owned()
                    .into_iter()
                    .map(|(key, value)| (key, Self::from(value)))
                    .collect(),
            ),
        }
    }
}

impl<'a> From<BridgePayload<'a>> for PayloadValue<'a> {
    fn from(value: BridgePayload<'a>) -> Self {
        match value {
            BridgePayload::Empty => Self::Null,
            BridgePayload::Scalar(BridgeScalar::Bool(value)) => Self::Bool(value),
            BridgePayload::Scalar(BridgeScalar::I64(value)) => Self::I64(value),
            BridgePayload::Scalar(BridgeScalar::U64(value)) => Self::U64(value),
            BridgePayload::Scalar(BridgeScalar::F64(value)) => Self::F64(value),
            BridgePayload::Text(value) => Self::String(value),
            BridgePayload::Binary(value) => Self::Bytes(value),
            BridgePayload::Sequence(values) => Self::List(Cow::Owned(
                values.into_iter().map(PayloadValue::from).collect(),
            )),
            BridgePayload::Object(values) => Self::Map(Cow::Owned(
                values
                    .into_iter()
                    .map(|(key, value)| (key, PayloadValue::from(value)))
                    .collect(),
            )),
        }
    }
}

impl From<String> for BridgePayload<'static> {
    fn from(value: String) -> Self {
        BridgePayload::Text(Cow::Owned(value))
    }
}

impl<'a> From<&'a str> for BridgePayload<'a> {
    fn from(value: &'a str) -> Self {
        BridgePayload::Text(Cow::Borrowed(value))
    }
}

impl From<Vec<u8>> for BridgePayload<'static> {
    fn from(value: Vec<u8>) -> Self {
        BridgePayload::Binary(Cow::Owned(value))
    }
}

impl<'a> From<&'a [u8]> for BridgePayload<'a> {
    fn from(value: &'a [u8]) -> Self {
        BridgePayload::Binary(Cow::Borrowed(value))
    }
}
