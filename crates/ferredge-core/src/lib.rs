//! # Ferredge Core
//!
//! Core library for the ferredge edge device aggregator.
//!
//! This crate provides the fundamental abstractions for:
//! - Device representation and lifecycle management
//! - Message/event routing between components
//! - Storage backend abstraction
//! - Driver interface for protocol implementations
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                         ferredge-core                           │
//! ├─────────────────────────────────────────────────────────────────┤
//! │                                                                 │
//! │  ┌─────────┐    ┌─────────────┐    ┌─────────────────────────┐ │
//! │  │ Devices │◄──►│   Router    │◄──►│  Handlers               │ │
//! │  └─────────┘    └─────────────┘    │  - Storage              │ │
//! │                        ▲           │  - Drivers              │ │
//! │                        │           │  - Other Devices        │ │
//! │                        ▼           │  - SDK/FFI              │ │
//! │                 ┌─────────────┐    └─────────────────────────┘ │
//! │                 │   Events    │                                 │
//! │                 └─────────────┘                                 │
//! └─────────────────────────────────────────────────────────────────┘
//! ```

pub mod device;
pub mod driver;
pub mod error;
pub mod event;
pub mod message;
pub mod router;
pub mod storage;
pub mod types;

// Re-export commonly used types
pub use device::{Device, DeviceId, DeviceInfo, DeviceState, DeviceProfile};
pub use driver::{Driver, DriverId, DriverInfo};
pub use error::{Error, Result};
pub use event::{Event, EventKind};
pub use message::{Command, CommandResponse, Message, Reading};
pub use router::{Router, RouteTarget};
pub use storage::StorageBackend;
pub use types::Value;
