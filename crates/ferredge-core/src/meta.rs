extern crate alloc;

use alloc::{borrow::Cow, vec::Vec};
use core::fmt;

use serde::{Deserialize, Serialize};

use crate::headers::Headers;
use crate::owned::IntoOwned;

/// Open protocol identifier shared by routing, capture, and fault metadata.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProtocolId(pub Cow<'static, str>);

impl ProtocolId {
    /// MQTT protocol identifier.
    pub const MQTT: ProtocolId = ProtocolId(Cow::Borrowed("mqtt"));
    /// HTTP protocol identifier.
    pub const HTTP: ProtocolId = ProtocolId(Cow::Borrowed("http"));
    /// Modbus protocol identifier.
    pub const MODBUS: ProtocolId = ProtocolId(Cow::Borrowed("modbus"));
    /// CoAP protocol identifier.
    pub const COAP: ProtocolId = ProtocolId(Cow::Borrowed("coap"));

    /// Returns the identifier as a string slice.
    pub fn as_str(&self) -> &str {
        self.0.as_ref()
    }
}

impl fmt::Display for ProtocolId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Typed value stored in one open transport metadata entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MetaValue<'a> {
    /// Boolean flag value.
    Bool(bool),
    /// Unsigned integer value.
    U64(u64),
    /// Signed integer value.
    I64(i64),
    /// UTF-8 text value.
    Str(Cow<'a, str>),
    /// Opaque binary value.
    Bytes(Cow<'a, [u8]>),
    /// List of unsigned integers.
    U64List(Vec<u64>),
    /// List of UTF-8 text values.
    StrList(Vec<Cow<'a, str>>),
    /// Ordered name/value string pairs.
    Pairs(Vec<(Cow<'a, str>, Cow<'a, str>)>),
}

impl MetaValue<'_> {
    /// Re-borrows this value without forcing ownership changes.
    pub fn as_borrowed(&self) -> MetaValue<'_> {
        match self {
            Self::Bool(value) => MetaValue::Bool(*value),
            Self::U64(value) => MetaValue::U64(*value),
            Self::I64(value) => MetaValue::I64(*value),
            Self::Str(value) => MetaValue::Str(Cow::Borrowed(value.as_ref())),
            Self::Bytes(value) => MetaValue::Bytes(Cow::Borrowed(value.as_ref())),
            Self::U64List(values) => MetaValue::U64List(values.clone()),
            Self::StrList(values) => MetaValue::StrList(
                values
                    .iter()
                    .map(|value| Cow::Borrowed(value.as_ref()))
                    .collect(),
            ),
            Self::Pairs(pairs) => MetaValue::Pairs(
                pairs
                    .iter()
                    .map(|(key, value)| {
                        (Cow::Borrowed(key.as_ref()), Cow::Borrowed(value.as_ref()))
                    })
                    .collect(),
            ),
        }
    }

    /// Materializes an owned value for long-lived boundaries.
    pub fn into_owned(self) -> MetaValue<'static> {
        IntoOwned::into_owned(self)
    }
}

impl IntoOwned for MetaValue<'_> {
    type Owned = MetaValue<'static>;
    fn into_owned(self) -> Self::Owned {
        match self {
            Self::Bool(value) => MetaValue::Bool(value),
            Self::U64(value) => MetaValue::U64(value),
            Self::I64(value) => MetaValue::I64(value),
            Self::Str(value) => MetaValue::Str(IntoOwned::into_owned(value)),
            Self::Bytes(value) => MetaValue::Bytes(IntoOwned::into_owned(value)),
            Self::U64List(values) => MetaValue::U64List(values),
            Self::StrList(values) => MetaValue::StrList(IntoOwned::into_owned(values)),
            Self::Pairs(pairs) => MetaValue::Pairs(IntoOwned::into_owned(pairs)),
        }
    }
}

/// Open protocol-keyed transport metadata capture carried by envelopes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransportMeta<'a> {
    /// Protocol whose adapter produced or consumes this capture.
    pub protocol: ProtocolId,
    /// Wire-level name/value header pairs carried alongside the capture.
    pub headers: Option<Headers<'a>>,
    /// Extension SPI: raw entries behind the typed protocol views; application code should use those views.
    entries: Vec<(Cow<'static, str>, MetaValue<'a>)>,
}

impl<'a> TransportMeta<'a> {
    /// Creates an empty capture for the given protocol.
    pub fn new(protocol: ProtocolId) -> Self {
        Self {
            protocol,
            headers: None,
            entries: Vec::new(),
        }
    }

    /// Consumes the capture, yielding its raw entries for typed decoding.
    pub fn into_entries(self) -> Vec<(Cow<'static, str>, MetaValue<'a>)> {
        self.entries
    }

    /// Returns the value stored under the given key, if any.
    pub fn get(&self, key: &str) -> Option<&MetaValue<'a>> {
        self.entries
            .iter()
            .find(|(entry_key, _)| entry_key.as_ref() == key)
            .map(|(_, value)| value)
    }

    /// Stores the value under the given key, replacing an existing entry.
    pub fn set(&mut self, key: impl Into<Cow<'static, str>>, value: MetaValue<'a>) {
        let key = key.into();
        match self
            .entries
            .iter_mut()
            .find(|(entry_key, _)| entry_key.as_ref() == key.as_ref())
        {
            Some((_, existing)) => *existing = value,
            None => self.entries.push((key, value)),
        }
    }

    /// Removes every entry stored under the given key, returning the first value, if any.
    pub fn take(&mut self, key: &str) -> Option<MetaValue<'a>> {
        let index = self
            .entries
            .iter()
            .position(|(entry_key, _)| entry_key.as_ref() == key)?;
        let value = self.entries.remove(index).1;
        self.entries
            .retain(|(entry_key, _)| entry_key.as_ref() != key);
        Some(value)
    }

    /// Returns the string value stored under the given key, if any.
    pub fn get_str(&self, key: &str) -> Option<&str> {
        match self.get(key) {
            Some(MetaValue::Str(value)) => Some(value.as_ref()),
            _ => None,
        }
    }

    /// Returns the unsigned integer value stored under the given key, if any.
    pub fn get_u64(&self, key: &str) -> Option<u64> {
        match self.get(key) {
            Some(MetaValue::U64(value)) => Some(*value),
            _ => None,
        }
    }

    /// Returns the signed integer value stored under the given key, if any.
    pub fn get_i64(&self, key: &str) -> Option<i64> {
        match self.get(key) {
            Some(MetaValue::I64(value)) => Some(*value),
            _ => None,
        }
    }

    /// Returns the boolean value stored under the given key, if any.
    pub fn get_bool(&self, key: &str) -> Option<bool> {
        match self.get(key) {
            Some(MetaValue::Bool(value)) => Some(*value),
            _ => None,
        }
    }

    /// Returns the binary value stored under the given key, if any.
    pub fn get_bytes(&self, key: &str) -> Option<&[u8]> {
        match self.get(key) {
            Some(MetaValue::Bytes(value)) => Some(value.as_ref()),
            _ => None,
        }
    }

    /// Returns the unsigned integer list stored under the given key, if any.
    pub fn get_u64_list(&self, key: &str) -> Option<&[u64]> {
        match self.get(key) {
            Some(MetaValue::U64List(values)) => Some(values.as_slice()),
            _ => None,
        }
    }

    /// Returns the string list stored under the given key, if any.
    pub fn get_str_list(&self, key: &str) -> Option<&[Cow<'a, str>]> {
        match self.get(key) {
            Some(MetaValue::StrList(values)) => Some(values.as_slice()),
            _ => None,
        }
    }

    /// Returns the name/value pair list stored under the given key, if any.
    pub fn get_pairs(&self, key: &str) -> Option<&[(Cow<'a, str>, Cow<'a, str>)]> {
        match self.get(key) {
            Some(MetaValue::Pairs(pairs)) => Some(pairs.as_slice()),
            _ => None,
        }
    }

    /// Re-borrows this capture without forcing ownership changes.
    pub fn as_borrowed(&self) -> TransportMeta<'_> {
        TransportMeta {
            protocol: self.protocol.clone(),
            headers: self.headers.as_ref().map(Headers::as_borrowed),
            entries: self
                .entries
                .iter()
                .map(|(key, value)| (key.clone(), value.as_borrowed()))
                .collect(),
        }
    }

    /// Materializes an owned capture for long-lived boundaries.
    pub fn into_owned(self) -> TransportMeta<'static> {
        IntoOwned::into_owned(self)
    }
}

impl IntoOwned for TransportMeta<'_> {
    type Owned = TransportMeta<'static>;
    fn into_owned(self) -> Self::Owned {
        TransportMeta {
            protocol: self.protocol,
            headers: self.headers.map(IntoOwned::into_owned),
            entries: self
                .entries
                .into_iter()
                .map(|(key, value)| (key, IntoOwned::into_owned(value)))
                .collect(),
        }
    }
}

/// Typed protocol view over one transport metadata capture.
pub trait ProtocolMeta<'a>: Sized {
    /// Protocol identity this view projects.
    const PROTOCOL: ProtocolId;
    /// Moves this view into a transport capture (zero-copy: Cows move).
    fn into_transport_meta(self) -> TransportMeta<'a>;
    /// Moves a capture into this view; `None` covers protocol mismatch and malformed captures alike (a view's fallible decoder distinguishes them).
    fn from_transport_meta(meta: TransportMeta<'a>) -> Option<Self>;
    /// Borrows this view from a capture; `None` covers protocol mismatch and malformed captures alike (a view's fallible decoder distinguishes them).
    fn view(meta: &'a TransportMeta<'a>) -> Option<Self>;
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;

    #[test]
    fn take_removes_all_duplicate_entries_returning_the_first() {
        let mut meta = TransportMeta::new(ProtocolId::MQTT);
        meta.entries = vec![
            (
                Cow::Borrowed("topic"),
                MetaValue::Str(Cow::Borrowed("first")),
            ),
            (Cow::Borrowed("qos"), MetaValue::U64(1)),
            (
                Cow::Borrowed("topic"),
                MetaValue::Str(Cow::Borrowed("second")),
            ),
        ];

        assert_eq!(
            meta.take("topic"),
            Some(MetaValue::Str(Cow::Borrowed("first")))
        );
        assert_eq!(meta.take("topic"), None);
        assert_eq!(meta.get_u64("qos"), Some(1));
    }
}
