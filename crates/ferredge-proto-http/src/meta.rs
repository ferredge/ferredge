extern crate alloc;

use alloc::{borrow::Cow, vec::Vec};

use ferredge_core::prelude::{
    Headers, IntoOwned, MetaValue, ProtocolId, ProtocolMeta, TransportMeta,
};
use serde::{Deserialize, Serialize};

use crate::HttpCommandConversionError;

/// Capture key for the HTTP request method.
pub const KEY_METHOD: &str = "method";
/// Capture key for the HTTP request path.
pub const KEY_PATH: &str = "path";
/// Capture key for the HTTP response status code; values above `u16::MAX` mark the capture malformed.
pub const KEY_STATUS_CODE: &str = "status-code";
/// Capture key for the HTTP content type.
pub const KEY_CONTENT_TYPE: &str = "content-type";

/// Typed HTTP view over the open transport metadata capture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct HttpMeta<'a> {
    /// HTTP method used for the exchange.
    pub method: Option<Cow<'a, str>>,
    /// Request path used for the exchange.
    pub path: Option<Cow<'a, str>>,
    /// Response status code when available.
    pub status_code: Option<u16>,
    /// Content type of the carried body.
    pub content_type: Option<Cow<'a, str>>,
    /// HTTP headers carried as first-class capture headers.
    pub headers: Option<Headers<'a>>,
}

impl<'a> HttpMeta<'a> {
    /// Decodes an HTTP capture by move, rejecting foreign protocols, malformed entries, and wrong-typed known keys; duplicate keys decode first-occurrence-wins.
    pub fn try_from_transport_meta(
        mut meta: TransportMeta<'a>,
    ) -> Result<Self, HttpCommandConversionError> {
        if meta.protocol != ProtocolId::HTTP {
            return Err(HttpCommandConversionError::ForeignExtensions(meta.protocol));
        }
        let mut view = HttpMeta {
            headers: meta.headers.take(),
            ..HttpMeta::default()
        };
        let mut seen: Vec<Cow<'static, str>> = Vec::new();
        for (key, value) in meta.into_entries() {
            if seen.iter().any(|entry| entry == &key) {
                continue;
            }
            seen.push(key.clone());
            match key.as_ref() {
                KEY_METHOD => view.method = Some(expect_str(KEY_METHOD, value)?),
                KEY_PATH => view.path = Some(expect_str(KEY_PATH, value)?),
                KEY_STATUS_CODE => {
                    let value = expect_u64(KEY_STATUS_CODE, value)?;
                    view.status_code = Some(u16::try_from(value).map_err(|_| {
                        HttpCommandConversionError::InvalidExtensionValue {
                            key: KEY_STATUS_CODE,
                            value,
                        }
                    })?);
                }
                KEY_CONTENT_TYPE => view.content_type = Some(expect_str(KEY_CONTENT_TYPE, value)?),
                _ => {}
            }
        }
        Ok(view)
    }
}

/// Requires one capture value to be UTF-8 text.
fn expect_str<'a>(
    key: &'static str,
    value: MetaValue<'a>,
) -> Result<Cow<'a, str>, HttpCommandConversionError> {
    match value {
        MetaValue::Str(value) => Ok(value),
        _ => Err(HttpCommandConversionError::InvalidExtensionType { key }),
    }
}

/// Requires one capture value to be an unsigned integer.
fn expect_u64(key: &'static str, value: MetaValue<'_>) -> Result<u64, HttpCommandConversionError> {
    match value {
        MetaValue::U64(value) => Ok(value),
        _ => Err(HttpCommandConversionError::InvalidExtensionType { key }),
    }
}

impl<'a> ProtocolMeta<'a> for HttpMeta<'a> {
    const PROTOCOL: ProtocolId = ProtocolId::HTTP;

    /// Encodes this typed view as an open HTTP transport capture without copying values.
    fn into_transport_meta(self) -> TransportMeta<'a> {
        let mut meta = TransportMeta::new(ProtocolId::HTTP);
        if let Some(method) = self.method {
            meta.set(KEY_METHOD, MetaValue::Str(method));
        }
        if let Some(path) = self.path {
            meta.set(KEY_PATH, MetaValue::Str(path));
        }
        if let Some(status_code) = self.status_code {
            meta.set(KEY_STATUS_CODE, MetaValue::U64(u64::from(status_code)));
        }
        if let Some(content_type) = self.content_type {
            meta.set(KEY_CONTENT_TYPE, MetaValue::Str(content_type));
        }
        meta.headers = self.headers;
        meta
    }

    /// Decodes an HTTP transport capture into a typed view by move; foreign or malformed captures yield `None`.
    fn from_transport_meta(meta: TransportMeta<'a>) -> Option<Self> {
        Self::try_from_transport_meta(meta).ok()
    }

    /// Borrows an HTTP transport capture as a typed view; foreign or malformed captures yield `None`.
    fn view(meta: &'a TransportMeta<'a>) -> Option<HttpMeta<'a>> {
        Self::try_from_transport_meta(meta.as_borrowed()).ok()
    }
}

impl HttpMeta<'_> {
    /// Materializes an owned typed view for long-lived boundaries.
    pub fn into_owned(self) -> HttpMeta<'static> {
        IntoOwned::into_owned(self)
    }
}

impl IntoOwned for HttpMeta<'_> {
    type Owned = HttpMeta<'static>;
    fn into_owned(self) -> Self::Owned {
        HttpMeta {
            method: self.method.into_owned(),
            path: self.path.into_owned(),
            status_code: self.status_code,
            content_type: self.content_type.into_owned(),
            headers: self.headers.into_owned(),
        }
    }
}
