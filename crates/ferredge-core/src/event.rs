//! Event system for ferredge.
//!
//! Events represent significant occurrences in the system that
//! other components may want to react to.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::device::{DeviceId, DeviceState};
use crate::message::{MessageId, Reading};

/// An event that occurred in the system.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    /// Unique event identifier
    pub id: MessageId,
    /// Event timestamp
    pub timestamp: DateTime<Utc>,
    /// The kind of event
    pub kind: EventKind,
    /// Optional source device
    pub source_device: Option<DeviceId>,
    /// Optional correlation ID
    pub correlation_id: Option<MessageId>,
}

impl Event {
    /// Creates a new event.
    pub fn new(kind: EventKind) -> Self {
        Self {
            id: MessageId::generate(),
            timestamp: Utc::now(),
            kind,
            source_device: None,
            correlation_id: None,
        }
    }

    /// Sets the source device.
    pub fn with_source(mut self, device_id: DeviceId) -> Self {
        self.source_device = Some(device_id);
        self
    }

    /// Sets the correlation ID.
    pub fn with_correlation(mut self, correlation_id: MessageId) -> Self {
        self.correlation_id = Some(correlation_id);
        self
    }

    /// Creates a device state changed event.
    pub fn device_state_changed(device_id: DeviceId, old_state: DeviceState, new_state: DeviceState) -> Self {
        Self::new(EventKind::DeviceStateChanged {
            device_id: device_id.clone(),
            old_state,
            new_state,
        })
        .with_source(device_id)
    }

    /// Creates a new reading event.
    pub fn reading(reading: Reading) -> Self {
        let device_id = reading.device_id.clone();
        Self::new(EventKind::Reading(reading)).with_source(device_id)
    }

    /// Creates an error event.
    pub fn error(message: impl Into<String>, device_id: Option<DeviceId>) -> Self {
        let mut event = Self::new(EventKind::Error {
            message: message.into(),
        });
        if let Some(id) = device_id {
            event = event.with_source(id);
        }
        event
    }
}

/// The kind of event that occurred.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventKind {
    /// A device's state changed
    DeviceStateChanged {
        device_id: DeviceId,
        old_state: DeviceState,
        new_state: DeviceState,
    },

    /// A device was registered
    DeviceRegistered {
        device_id: DeviceId,
    },

    /// A device was unregistered
    DeviceUnregistered {
        device_id: DeviceId,
    },

    /// A new reading was received
    Reading(Reading),

    /// An error occurred
    Error {
        message: String,
    },

    /// A command was executed
    CommandExecuted {
        device_id: DeviceId,
        command: String,
        success: bool,
    },

    /// Driver started
    DriverStarted {
        driver_id: String,
    },

    /// Driver stopped
    DriverStopped {
        driver_id: String,
    },

    /// System startup complete
    SystemStarted,

    /// System shutdown initiated
    SystemShutdown,

    /// Custom event for extensibility
    Custom {
        name: String,
        data: serde_json::Value,
    },
}

/// Trait for types that can handle events.
pub trait EventHandler: Send + Sync {
    /// Handles an event. Returns Ok(true) if the event was handled,
    /// Ok(false) if it should be passed to other handlers.
    fn handle(&self, event: &Event) -> impl std::future::Future<Output = crate::Result<bool>> + Send;

    /// Returns the event kinds this handler is interested in.
    /// If None, the handler receives all events.
    fn filter(&self) -> Option<Vec<EventKindFilter>> {
        None
    }
}

/// Filter for event kinds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventKindFilter {
    DeviceStateChanged,
    DeviceRegistered,
    DeviceUnregistered,
    Reading,
    Error,
    CommandExecuted,
    DriverStarted,
    DriverStopped,
    SystemStarted,
    SystemShutdown,
    Custom(String),
}

impl EventKindFilter {
    /// Checks if this filter matches the given event kind.
    pub fn matches(&self, kind: &EventKind) -> bool {
        match (self, kind) {
            (EventKindFilter::DeviceStateChanged, EventKind::DeviceStateChanged { .. }) => true,
            (EventKindFilter::DeviceRegistered, EventKind::DeviceRegistered { .. }) => true,
            (EventKindFilter::DeviceUnregistered, EventKind::DeviceUnregistered { .. }) => true,
            (EventKindFilter::Reading, EventKind::Reading(_)) => true,
            (EventKindFilter::Error, EventKind::Error { .. }) => true,
            (EventKindFilter::CommandExecuted, EventKind::CommandExecuted { .. }) => true,
            (EventKindFilter::DriverStarted, EventKind::DriverStarted { .. }) => true,
            (EventKindFilter::DriverStopped, EventKind::DriverStopped { .. }) => true,
            (EventKindFilter::SystemStarted, EventKind::SystemStarted) => true,
            (EventKindFilter::SystemShutdown, EventKind::SystemShutdown) => true,
            (EventKindFilter::Custom(name), EventKind::Custom { name: event_name, .. }) => name == event_name,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Value;

    #[test]
    fn test_event_creation() {
        let device_id = DeviceId::new("device-1");
        let event = Event::device_state_changed(
            device_id.clone(),
            DeviceState::Offline,
            DeviceState::Online,
        );

        assert_eq!(event.source_device, Some(device_id));
        assert!(matches!(
            event.kind,
            EventKind::DeviceStateChanged { .. }
        ));
    }

    #[test]
    fn test_reading_event() {
        let reading = Reading::new(DeviceId::new("sensor-1"), "temp", Value::Float(25.0));
        let event = Event::reading(reading);

        assert!(matches!(event.kind, EventKind::Reading(_)));
    }

    #[test]
    fn test_event_filter() {
        let filter = EventKindFilter::Reading;
        let reading = Reading::new(DeviceId::new("sensor-1"), "temp", Value::Float(25.0));

        assert!(filter.matches(&EventKind::Reading(reading)));
        assert!(!filter.matches(&EventKind::SystemStarted));
    }
}
