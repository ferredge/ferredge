#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(feature = "std")]
use std::{string::String, vec::Vec};

#[cfg(not(feature = "std"))]
use alloc::{string::String, vec::Vec};

use ferredge_core::prelude::*;

pub mod attributes;
mod handler;

/// Native outbound HTTP request used by the HTTP protocol adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    /// HTTP method sent to target endpoint.
    pub method: String,
    /// Request path or slug.
    pub path: String,
    /// Optional request body.
    pub body: Option<Vec<u8>>,
    /// Additional request headers.
    pub headers: Vec<(String, String)>,
}

/// Native HTTP response returned by the HTTP protocol adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    /// Raw response bytes returned from server.
    pub body: Vec<u8>,
}

/// Conversion error raised when routed command cannot be represented as HTTP request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HttpCommandConversionError {
    /// Routed command intent cannot be executed via HTTP request/response semantics.
    UnsupportedIntent,
    /// Target resource does not exist on concrete device definition.
    UnknownResource(String),
}

impl core::fmt::Display for HttpCommandConversionError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnsupportedIntent => write!(f, "unsupported intent for HTTP driver"),
            Self::UnknownResource(resource) => {
                write!(f, "resource {resource} not found for HTTP driver")
            }
        }
    }
}

/// HTTP protocol adapter implementing lifecycle and request/response capabilities.
#[derive(Debug, Clone)]
pub struct HttpDriver {
    /// Device metadata and resource map served by this driver.
    pub dvc: Device<attributes::HttpResourceAttributes>,
}

impl HttpDriver {
    /// Builds a concrete HTTP request from routed command and device resource metadata.
    pub fn try_request_from_command(
        &self,
        command: &Command,
    ) -> Result<HttpRequest, HttpCommandConversionError> {
        match &command.intent {
            Intent::Read { resource } | Intent::Invoke { operation: resource, .. } => self
                .dvc
                .resources
                .get(resource)
                .map(|resource| HttpRequest {
                    method: resource.resource_attributes.method.clone(),
                    path: resource.resource_attributes.slug.clone(),
                    body: None,
                    headers: resource
                        .resource_attributes
                        .headers
                        .clone()
                        .unwrap_or_default(),
                })
                .ok_or_else(|| HttpCommandConversionError::UnknownResource(resource.clone())),
            Intent::Write { resource, payload } => self
                .dvc
                .resources
                .get(resource)
                .map(|resource| HttpRequest {
                    method: resource.resource_attributes.method.clone(),
                    path: resource.resource_attributes.slug.clone(),
                    body: Some(payload.clone()),
                    headers: resource
                        .resource_attributes
                        .headers
                        .clone()
                        .unwrap_or_default(),
                })
                .ok_or_else(|| HttpCommandConversionError::UnknownResource(resource.clone())),
            Intent::Send { .. } | Intent::Subscribe { .. } | Intent::Unsubscribe { .. } => {
                Err(HttpCommandConversionError::UnsupportedIntent)
            }
        }
    }
}

impl Lifecycle for HttpDriver {
    type Error = String;

    async fn start(&self) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn stop(&self) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl RequestResponse for HttpDriver {
    type Request = HttpRequest;
    type Response = HttpResponse;
    type Error = String;

    #[cfg(feature = "std")]
    async fn execute(&self, request: Self::Request) -> Result<Self::Response, Self::Error> {
        let endpoint = match &self.dvc.endpoint {
            DeviceEndpoint::Http(config) => config.url.as_str(),
            _ => return Err("device endpoint is not HTTP".to_string()),
        };

        handler::send_request(endpoint, &request)
            .map(|body| HttpResponse { body })
            .map_err(|e| e.to_string())
    }

    #[cfg(not(feature = "std"))]
    async fn execute(&self, _request: Self::Request) -> Result<Self::Response, Self::Error> {
        Err("HTTP driver not implemented for no_std environment".to_string())
    }
}
