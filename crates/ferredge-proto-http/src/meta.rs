extern crate alloc;

use alloc::borrow::Cow;

use ferredge_core::prelude::{IntoOwned, MetaValue, ProtocolId, TransportMeta};
use serde::{Deserialize, Serialize};

/// Capture key for the HTTP request method.
pub const KEY_METHOD: &str = "method";
/// Capture key for the HTTP request path.
pub const KEY_PATH: &str = "path";
/// Capture key for the HTTP response status code.
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
}

impl<'a> HttpMeta<'a> {
    /// Encodes this typed view as an open HTTP transport capture without copying values.
    pub fn into_transport_meta(self) -> TransportMeta<'a> {
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
        meta
    }

    /// Decodes an HTTP transport capture into a typed view by move.
    pub fn from_transport_meta(meta: TransportMeta<'a>) -> Option<Self> {
        if meta.protocol != ProtocolId::HTTP {
            return None;
        }
        let mut meta = meta;
        let mut view = HttpMeta::default();
        for (key, value) in meta.entries.drain(..) {
            match (key.as_ref(), value) {
                (KEY_METHOD, MetaValue::Str(value)) => view.method = Some(value),
                (KEY_PATH, MetaValue::Str(value)) => view.path = Some(value),
                (KEY_STATUS_CODE, MetaValue::U64(value)) => {
                    view.status_code = Some(value as u16);
                }
                (KEY_CONTENT_TYPE, MetaValue::Str(value)) => view.content_type = Some(value),
                _ => {}
            }
        }
        Some(view)
    }

    /// Borrows an HTTP transport capture as a typed view without consuming it.
    pub fn view(meta: &'a TransportMeta<'a>) -> Option<HttpMeta<'a>> {
        if meta.protocol != ProtocolId::HTTP {
            return None;
        }
        Some(HttpMeta {
            method: meta.get_str(KEY_METHOD).map(Cow::Borrowed),
            path: meta.get_str(KEY_PATH).map(Cow::Borrowed),
            status_code: meta.get_u64(KEY_STATUS_CODE).map(|value| value as u16),
            content_type: meta.get_str(KEY_CONTENT_TYPE).map(Cow::Borrowed),
        })
    }

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
        }
    }
}
