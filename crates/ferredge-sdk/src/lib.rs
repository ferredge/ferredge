//! # Ferredge SDK
//!
//! SDK for interacting with the ferredge core from external applications.
//!
//! This crate provides a high-level API for:
//! - Connecting to a ferredge instance
//! - Reading from and writing to devices
//! - Subscribing to events
//! - Managing device configurations
//!
//! ## Example
//!
//! ```rust,ignore
//! use ferredge_sdk::Client;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Connect to ferredge
//!     let client = Client::connect("localhost:8080").await?;
//!
//!     // Read from a device
//!     let reading = client.read("sensor-1", "temperature").await?;
//!     println!("Temperature: {:?}", reading.value);
//!
//!     // Write to a device
//!     client.write("actuator-1", "setpoint", 25.0.into()).await?;
//!
//!     Ok(())
//! }
//! ```

pub mod client;
pub mod error;

// Re-export core types that SDK users need
pub use ferredge_core::{
    device::{DeviceId, DeviceInfo, DeviceState},
    event::{Event, EventKind},
    message::{Command, CommandResponse, Reading},
    types::Value,
};

pub use client::Client;
pub use error::{SdkError, SdkResult};
