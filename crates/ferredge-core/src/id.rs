extern crate alloc;

use alloc::borrow::Cow;
use alloc::string::String;
use core::fmt;

use serde::{Deserialize, Serialize};

/// Declares an owned, transparent string newtype for one identifier domain.
///
/// Distinct types keep command, device, resource, operation, and channel
/// identifiers from being swapped at call sites; `From` impls keep literal
/// and `format!` construction ergonomic; `#[serde(transparent)]` keeps the
/// wire format identical to a plain string.
macro_rules! string_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Creates the identifier from any string-like value.
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            /// Returns the identifier as a string slice.
            pub fn as_str(&self) -> &str {
                &self.0
            }

            /// Consumes the identifier, yielding the inner string.
            pub fn into_string(self) -> String {
                self.0
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(String::from(value))
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl From<$name> for Cow<'static, str> {
            fn from(value: $name) -> Self {
                Cow::Owned(value.0)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl PartialEq<str> for $name {
            fn eq(&self, other: &str) -> bool {
                self.0 == other
            }
        }

        impl PartialEq<&str> for $name {
            fn eq(&self, other: &&str) -> bool {
                self.0 == *other
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

string_id! {
    /// Unique identifier for command lifecycle and correlation.
    CommandId
}

string_id! {
    /// Stable identifier for a registered device.
    DeviceId
}

string_id! {
    /// Logical resource name targeted by request/response and addressed-access intents.
    ResourceName
}

string_id! {
    /// Named operation invoked on a device or service.
    OperationName
}

string_id! {
    /// Broker channel name such as a topic, queue, subject, or stream.
    ChannelName
}
