//! Core value types for ferredge.
//!
//! This module provides protocol-agnostic value representations
//! that can be used across different device protocols.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A protocol-agnostic value type that can represent data from any device.
///
/// This is the fundamental data unit that flows through ferredge.
/// It supports common data types found across industrial protocols.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Value {
    /// Null/empty value
    Null,
    /// Boolean value
    Bool(bool),
    /// Signed 64-bit integer
    Int(i64),
    /// Unsigned 64-bit integer
    UInt(u64),
    /// 64-bit floating point
    Float(f64),
    /// UTF-8 string
    String(String),
    /// Raw binary data
    Binary(Vec<u8>),
    /// Array of values
    Array(Vec<Value>),
    /// Key-value map
    Object(HashMap<String, Value>),
}

impl Value {
    /// Returns true if the value is null.
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    /// Attempts to get the value as a boolean.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// Attempts to get the value as an i64.
    pub fn as_int(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            Value::UInt(u) => (*u).try_into().ok(),
            _ => None,
        }
    }

    /// Attempts to get the value as a u64.
    pub fn as_uint(&self) -> Option<u64> {
        match self {
            Value::UInt(u) => Some(*u),
            Value::Int(i) => (*i).try_into().ok(),
            _ => None,
        }
    }

    /// Attempts to get the value as an f64.
    pub fn as_float(&self) -> Option<f64> {
        match self {
            Value::Float(f) => Some(*f),
            Value::Int(i) => Some(*i as f64),
            Value::UInt(u) => Some(*u as f64),
            _ => None,
        }
    }

    /// Attempts to get the value as a string slice.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    /// Attempts to get the value as a byte slice.
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Value::Binary(b) => Some(b),
            _ => None,
        }
    }

    /// Attempts to get the value as an array.
    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(a) => Some(a),
            _ => None,
        }
    }

    /// Attempts to get the value as an object.
    pub fn as_object(&self) -> Option<&HashMap<String, Value>> {
        match self {
            Value::Object(o) => Some(o),
            _ => None,
        }
    }
}

impl Default for Value {
    fn default() -> Self {
        Value::Null
    }
}

impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Value::Bool(v)
    }
}

impl From<i64> for Value {
    fn from(v: i64) -> Self {
        Value::Int(v)
    }
}

impl From<i32> for Value {
    fn from(v: i32) -> Self {
        Value::Int(v as i64)
    }
}

impl From<u64> for Value {
    fn from(v: u64) -> Self {
        Value::UInt(v)
    }
}

impl From<u32> for Value {
    fn from(v: u32) -> Self {
        Value::UInt(v as u64)
    }
}

impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Value::Float(v)
    }
}

impl From<f32> for Value {
    fn from(v: f32) -> Self {
        Value::Float(v as f64)
    }
}

impl From<String> for Value {
    fn from(v: String) -> Self {
        Value::String(v)
    }
}

impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Value::String(v.to_owned())
    }
}

impl From<Vec<u8>> for Value {
    fn from(v: Vec<u8>) -> Self {
        Value::Binary(v)
    }
}

impl<T: Into<Value>> From<Vec<T>> for Value {
    fn from(v: Vec<T>) -> Self {
        Value::Array(v.into_iter().map(Into::into).collect())
    }
}

impl<T: Into<Value>> From<HashMap<String, T>> for Value {
    fn from(v: HashMap<String, T>) -> Self {
        Value::Object(v.into_iter().map(|(k, v)| (k, v.into())).collect())
    }
}

/// Data type enumeration for describing expected value types.
///
/// Used in device profiles and resource definitions to specify
/// the expected data type for a resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DataType {
    Bool,
    Int8,
    Int16,
    Int32,
    Int64,
    UInt8,
    UInt16,
    UInt32,
    UInt64,
    Float32,
    Float64,
    String,
    Binary,
    Array,
    Object,
}

impl DataType {
    /// Returns the size in bytes for fixed-size types, None for variable types.
    pub fn size(&self) -> Option<usize> {
        match self {
            DataType::Bool => Some(1),
            DataType::Int8 | DataType::UInt8 => Some(1),
            DataType::Int16 | DataType::UInt16 => Some(2),
            DataType::Int32 | DataType::UInt32 | DataType::Float32 => Some(4),
            DataType::Int64 | DataType::UInt64 | DataType::Float64 => Some(8),
            DataType::String | DataType::Binary | DataType::Array | DataType::Object => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_value_conversions() {
        assert_eq!(Value::from(42i32).as_int(), Some(42));
        assert_eq!(Value::from(3.14f64).as_float(), Some(3.14));
        assert_eq!(Value::from("hello").as_str(), Some("hello"));
        assert_eq!(Value::from(true).as_bool(), Some(true));
    }

    #[test]
    fn test_value_serialization() {
        let value = Value::Object(HashMap::from([
            ("temperature".to_string(), Value::Float(25.5)),
            ("humidity".to_string(), Value::Int(60)),
        ]));

        let json = serde_json::to_string(&value).unwrap();
        let parsed: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value, parsed);
    }
}
