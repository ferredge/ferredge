extern crate alloc;

use alloc::{borrow::Cow, string::String, vec::Vec};

use serde::{Deserialize, Serialize};

use crate::owned::IntoOwned;

/// Single protocol header or property pair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Header<'a> {
    /// Header or property name.
    pub key: Cow<'a, str>,
    /// Header or property value.
    pub value: Cow<'a, str>,
}

impl Header<'_> {
    /// Re-borrows this header without forcing ownership changes.
    pub fn as_borrowed(&self) -> Header<'_> {
        Header {
            key: Cow::Borrowed(self.key.as_ref()),
            value: Cow::Borrowed(self.value.as_ref()),
        }
    }

    /// Materializes an owned header for long-lived boundaries.
    pub fn into_owned(self) -> Header<'static> {
        IntoOwned::into_owned(self)
    }
}

impl IntoOwned for Header<'_> {
    type Owned = Header<'static>;
    fn into_owned(self) -> Self::Owned {
        Header {
            key: IntoOwned::into_owned(self.key),
            value: IntoOwned::into_owned(self.value),
        }
    }
}

impl From<(String, String)> for Header<'static> {
    fn from((key, value): (String, String)) -> Self {
        Header {
            key: Cow::Owned(key),
            value: Cow::Owned(value),
        }
    }
}

impl<'a> From<(Cow<'a, str>, Cow<'a, str>)> for Header<'a> {
    fn from((key, value): (Cow<'a, str>, Cow<'a, str>)) -> Self {
        Header { key, value }
    }
}

/// Ordered protocol header or property collection preserved across routing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(transparent)]
pub struct Headers<'a>(pub Vec<Header<'a>>);

impl<'a> Headers<'a> {
    /// Returns the first value whose key matches case-insensitively.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.iter().find_map(|header| {
            header
                .key
                .eq_ignore_ascii_case(name)
                .then_some(header.value.as_ref())
        })
    }

    /// Appends one header pair.
    pub fn push(&mut self, key: impl Into<Cow<'a, str>>, value: impl Into<Cow<'a, str>>) {
        self.0.push(Header {
            key: key.into(),
            value: value.into(),
        });
    }

    /// Iterates headers in preserved order.
    pub fn iter(&self) -> impl Iterator<Item = &Header<'a>> {
        self.0.iter()
    }

    /// Returns true when no headers are present.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Returns the number of headers.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Re-borrows these headers without forcing ownership changes.
    pub fn as_borrowed(&self) -> Headers<'_> {
        Headers(self.0.iter().map(Header::as_borrowed).collect())
    }

    /// Materializes owned headers for long-lived boundaries.
    pub fn into_owned(self) -> Headers<'static> {
        IntoOwned::into_owned(self)
    }
}

impl IntoOwned for Headers<'_> {
    type Owned = Headers<'static>;
    fn into_owned(self) -> Self::Owned {
        Headers(self.0.into_owned())
    }
}

impl From<Vec<(String, String)>> for Headers<'static> {
    fn from(value: Vec<(String, String)>) -> Self {
        Headers(value.into_iter().map(Header::from).collect())
    }
}

impl<'a> From<Vec<(Cow<'a, str>, Cow<'a, str>)>> for Headers<'a> {
    fn from(value: Vec<(Cow<'a, str>, Cow<'a, str>)>) -> Self {
        Headers(value.into_iter().map(Header::from).collect())
    }
}
