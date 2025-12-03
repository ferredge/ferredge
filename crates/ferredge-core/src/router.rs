//! Core router for ferredge.
//!
//! The router is the central hub that:
//! - Routes commands to appropriate devices/drivers
//! - Dispatches events to registered handlers
//! - Manages message flow between components
//!
//! Architecture:
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                          Router                                  │
//! │                                                                  │
//! │  Commands ──►  ┌──────────────┐  ──► Devices                    │
//! │                │   Dispatch   │  ──► Drivers                    │
//! │  Events   ──►  │    Logic     │  ──► Storage                    │
//! │                └──────────────┘  ──► SDK/FFI                    │
//! │                       ▲                                          │
//! │                       │                                          │
//! │               Route Configuration                                │
//! └─────────────────────────────────────────────────────────────────┘
//! ```

use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::Arc;

use tokio::sync::{RwLock, broadcast, mpsc};
use tracing::{debug, error, info, warn};

use crate::device::{Device, DeviceId};
use crate::driver::{DriverId, DriverRegistry};
use crate::error::{DeviceError, Result, RoutingError};
use crate::event::{Event, EventHandler, EventKind};
use crate::message::{Command, CommandOperation, CommandResponse, Message, Reading};
use crate::storage::DynStorageBackend;
use crate::{Driver, StorageBackend};

/// Target for routing a command or event.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RouteTarget {
    /// Route to a specific device
    Device(DeviceId),
    /// Route to a driver
    Driver(DriverId),
    /// Route to storage
    Storage,
    /// Broadcast to all event handlers
    Broadcast,
    /// Route to SDK/FFI layer
    External,
}

/// Configuration for a route.
#[derive(Debug, Clone)]
pub struct RouteConfig {
    /// The target of this route
    pub target: RouteTarget,
    /// Whether this route is enabled
    pub enabled: bool,
    /// Optional filter expression
    pub filter: Option<String>,
    /// Priority (lower = higher priority)
    pub priority: u32,
}

impl RouteConfig {
    /// Creates a new route configuration.
    pub fn new(target: RouteTarget) -> Self {
        Self {
            target,
            enabled: true,
            filter: None,
            priority: 100,
        }
    }

    /// Sets the priority.
    pub fn with_priority(mut self, priority: u32) -> Self {
        self.priority = priority;
        self
    }
}

/// The central router for ferredge.
pub struct Router<
    D: Device + 'static,
    E: EventHandler + 'static,
    S: StorageBackend + 'static,
    Dr: Driver<D> + 'static,
> {
    /// Registered devices
    devices: RwLock<HashMap<DeviceId, D>>,
    /// Driver registry
    drivers: RwLock<DriverRegistry<D, Dr>>,
    /// Storage backend
    storage: RwLock<Option<Arc<S>>>,
    /// Event handlers
    event_handlers: RwLock<Vec<Arc<E>>>,
    /// Event broadcast channel
    event_tx: broadcast::Sender<Event>,
    /// Command channel
    command_tx: mpsc::Sender<(Command, mpsc::Sender<CommandResponse>)>,
    /// Reading channel (for storage)
    reading_tx: mpsc::Sender<Reading>,
    /// Shutdown signal
    shutdown_tx: broadcast::Sender<()>,

    /// Marker for driver type
    __driver__: PhantomData<Dr>,
}

impl<
    D: Device + 'static,
    E: EventHandler + 'static,
    S: StorageBackend + 'static,
    Dr: Driver<D> + 'static,
> Router<D, E, S, Dr>
{
    /// Creates a new router with the given channel capacity.
    pub fn new(capacity: usize) -> (Self, RouterHandle<D, E, S, Dr>) {
        let (event_tx, _) = broadcast::channel(capacity);
        let (command_tx, command_rx) = mpsc::channel(capacity);
        let (reading_tx, reading_rx) = mpsc::channel(capacity);
        let (shutdown_tx, _) = broadcast::channel(1);

        let router = Self {
            devices: RwLock::new(HashMap::new()),
            drivers: RwLock::new(DriverRegistry::new()),
            storage: RwLock::new(None),
            event_handlers: RwLock::new(Vec::new()),
            event_tx,
            command_tx: command_tx.clone(),
            reading_tx: reading_tx.clone(),
            shutdown_tx,
            __driver__: PhantomData,
        };

        let handle = RouterHandle {
            command_rx,
            reading_rx,
            _device: PhantomData,
            _driver: PhantomData,
            _event: PhantomData,
            _storage: PhantomData,
        };

        (router, handle)
    }

    /// Registers a device with the router.
    pub async fn register_device(&self, device: D) -> Result<()> {
        let id = device.info().id.clone();
        info!(device_id = %id, "Registering device");

        let mut devices = self.devices.write().await;
        if devices.contains_key(&id) {
            return Err(DeviceError::AlreadyExists(id.to_string()).into());
        }

        devices.insert(id.clone(), device);

        // Emit registration event
        let event = Event::new(EventKind::DeviceRegistered { device_id: id });
        self.emit_event(event).await;

        Ok(())
    }

    /// Unregisters a device.
    pub async fn unregister_device(&self, device_id: &DeviceId) -> Result<()> {
        let mut devices = self.devices.write().await;
        if devices.remove(device_id).is_some() {
            info!(device_id = %device_id, "Unregistered device");

            let event = Event::new(EventKind::DeviceUnregistered {
                device_id: device_id.clone(),
            });
            self.emit_event(event).await;

            Ok(())
        } else {
            Err(DeviceError::NotFound(device_id.to_string()).into())
        }
    }

    /// Gets a device by ID.
    pub async fn get_device(&self, device_id: &DeviceId) -> Option<D> {
        self.devices.read().await.get(device_id).cloned()
    }

    /// Returns all registered devices.
    pub async fn devices(&self) -> Vec<D> {
        self.devices.read().await.values().cloned().collect()
    }

    /// Registers a driver.
    pub async fn register_driver(&self, driver: Arc<Dr>) {
        let id = driver.info().id.clone();
        info!(driver_id = %id, "Registering driver");

        self.drivers.write().await.register(driver);

        let event = Event::new(EventKind::DriverStarted {
            driver_id: id.to_string(),
        });
        self.emit_event(event).await;
    }

    /// Gets a driver by ID.
    pub async fn get_driver(&self, driver_id: &DriverId) -> Option<Arc<Dr>> {
        self.drivers.read().await.get(driver_id).cloned()
    }

    /// Sets the storage backend.
    pub async fn set_storage(&self, storage: Arc<S>) {
        info!(backend = storage.name(), "Setting storage backend");
        *self.storage.write().await = Some(storage);
    }

    /// Gets the storage backend.
    pub async fn storage(&self) -> Option<Arc<S>> {
        self.storage.read().await.clone()
    }

    /// Registers an event handler.
    pub async fn register_event_handler(&self, handler: Arc<E>) {
        self.event_handlers.write().await.push(handler);
    }

    /// Subscribes to events.
    pub fn subscribe_events(&self) -> broadcast::Receiver<Event> {
        self.event_tx.subscribe()
    }

    /// Emits an event to all handlers.
    pub async fn emit_event(&self, event: Event) {
        debug!(event_id = %event.id.0, "Emitting event");

        // Send to broadcast channel
        if let Err(e) = self.event_tx.send(event.clone()) {
            debug!("No event subscribers: {}", e);
        }

        // Call registered handlers
        let handlers = self.event_handlers.read().await;
        for handler in handlers.iter() {
            if let Some(filters) = handler.filter() {
                if !filters.iter().any(|f| f.matches(&event.kind)) {
                    continue;
                }
            }

            match handler.handle(&event).await {
                Ok(handled) => {
                    if handled {
                        debug!("Event handled");
                    }
                }
                Err(e) => {
                    error!(error = %e, "Event handler error");
                }
            }
        }
    }

    /// Sends a command and waits for a response.
    pub async fn send_command(&self, command: Command) -> Result<CommandResponse> {
        let device_id = command.device_id.clone();
        debug!(device_id = %device_id, "Sending command");

        // Get the device
        let device = self
            .get_device(&device_id)
            .await
            .ok_or_else(|| DeviceError::NotFound(device_id.to_string()))?;

        // Execute the command
        let response = match &command.operation {
            CommandOperation::Read { resource } => {
                let reading = device.read(resource).await?;
                CommandResponse::ok(reading.value)
            }
            CommandOperation::Write { resource, value } => {
                device.write(resource, value.clone()).await?;
                CommandResponse::ok_empty()
            }
            CommandOperation::Execute { .. } => device.execute(command.clone()).await?,
        };

        // Emit command executed event
        let success = response.success;
        let event = Event::new(EventKind::CommandExecuted {
            device_id: device_id.clone(),
            command: format!("{:?}", command.operation),
            success,
        });
        self.emit_event(event).await;

        Ok(response)
    }

    /// Stores a reading.
    pub async fn store_reading(&self, reading: Reading) -> Result<()> {
        // Emit reading event
        self.emit_event(Event::reading(reading.clone())).await;

        // Store in backend if available
        if let Some(storage) = self.storage().await {
            storage.store(reading).await?;
        } else {
            warn!("No storage backend configured, reading not persisted");
        }

        Ok(())
    }

    /// Returns a sender for submitting readings.
    pub fn reading_sender(&self) -> mpsc::Sender<Reading> {
        self.reading_tx.clone()
    }

    /// Returns a sender for submitting commands.
    pub fn command_sender(&self) -> mpsc::Sender<(Command, mpsc::Sender<CommandResponse>)> {
        self.command_tx.clone()
    }

    /// Initiates shutdown.
    pub async fn shutdown(&self) {
        info!("Router shutdown initiated");
        self.emit_event(Event::new(EventKind::SystemShutdown)).await;
        let _ = self.shutdown_tx.send(());
    }

    /// Subscribes to shutdown signal.
    pub fn subscribe_shutdown(&self) -> broadcast::Receiver<()> {
        self.shutdown_tx.subscribe()
    }
}

/// Handle for running router background tasks.
pub struct RouterHandle<D: Device, E: EventHandler, S: StorageBackend, Dr: Driver<D>> {
    /// Receiver for commands
    pub command_rx: mpsc::Receiver<(Command, mpsc::Sender<CommandResponse>)>,
    /// Receiver for readings
    pub reading_rx: mpsc::Receiver<Reading>,

    _device: PhantomData<D>,
    _driver: PhantomData<Dr>,
    _event: PhantomData<E>,
    _storage: PhantomData<S>,
}

impl<D: Device, E: EventHandler, S: StorageBackend, Dr: Driver<D>> RouterHandle<D, E, S, Dr> {
    /// Runs the router's background processing loop.
    pub async fn run(mut self, router: Arc<Router<D, E, S, Dr>>) {
        let mut shutdown_rx = router.subscribe_shutdown();

        loop {
            tokio::select! {
                // Process incoming commands
                Some((command, response_tx)) = self.command_rx.recv() => {
                    let router = router.clone();
                    tokio::spawn(async move {
                        let response = match router.send_command(command).await {
                            Ok(resp) => resp,
                            Err(e) => CommandResponse::err(e.to_string()),
                        };
                        let _ = response_tx.send(response).await;
                    });
                }

                // Process incoming readings
                Some(reading) = self.reading_rx.recv() => {
                    let router = router.clone();
                    tokio::spawn(async move {
                        if let Err(e) = router.store_reading(reading).await {
                            error!(error = %e, "Failed to store reading");
                        }
                    });
                }

                // Handle shutdown
                _ = shutdown_rx.recv() => {
                    info!("Router handle shutting down");
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::{DeviceInfo, DeviceState};
    use crate::types::Value;

    // Mock device for testing
    #[derive(Clone)]
    struct MockDevice {
        info: DeviceInfo,
    }

    struct EventHandlerMock;

    impl EventHandler for EventHandlerMock {
        fn filter(&self) -> Option<Vec<crate::event::EventKindFilter>> {
            None
        }

        async fn handle(&self, _event: &Event) -> Result<bool> {
            Ok(true)
        }
    }

    struct MockDriver<D: Device> {
        info: crate::driver::DriverInfo,
        device: Arc<D>,
    }

    impl<D: Device> Driver<D> for MockDriver<D> {
        fn info(&self) -> &crate::driver::DriverInfo {
            &self.info
        }

        async fn init(&self) -> Result<()> {
            Ok(())
        }

        async fn shutdown(&self) -> Result<()> {
            Ok(())
        }

        async fn create_device(
            &self,
            info: DeviceInfo,
            profile: crate::DeviceProfile,
            config: serde_json::Value,
        ) -> Result<Arc<D>> {
            Ok(self.device.clone())
        }

        fn validate_config(&self, _config: &serde_json::Value) -> Result<()> {
            Ok(())
        }
    }

    struct MockStorageBackend;

    impl StorageBackend for MockStorageBackend {
        fn name(&self) -> &str {
            "MockStorage"
        }

        async fn init(&self) -> Result<()> {
            Ok(())
        }

        async fn close(&self) -> Result<()> {
            Ok(())
        }

        async fn store(&self, _reading: Reading) -> Result<()> {
            Ok(())
        }

        async fn store_batch(&self, _readings: Vec<Reading>) -> Result<()> {
            Ok(())
        }

        async fn query(
            &self,
            _query: crate::storage::ReadingQuery,
        ) -> Result<crate::storage::QueryResult> {
            Ok(crate::storage::QueryResult {
                readings: Vec::new(),
                total: Some(0),
                has_more: false,
            })
        }

        async fn latest(&self, _device_id: &DeviceId, _resource: &str) -> Result<Option<Reading>> {
            Ok(None)
        }

        async fn delete(&self, _query: crate::storage::ReadingQuery) -> Result<usize> {
            Ok(0)
        }
    }

    impl Device for MockDevice {
        fn info(&self) -> &DeviceInfo {
            &self.info
        }

        fn state(&self) -> DeviceState {
            DeviceState::Online
        }

        async fn read(&self, resource: &str) -> Result<Reading> {
            Ok(Reading::new(
                self.info.id.clone(),
                resource,
                Value::Float(25.0),
            ))
        }

        async fn write(&self, _resource: &str, _value: Value) -> Result<()> {
            Ok(())
        }

        async fn execute(&self, _command: Command) -> Result<CommandResponse> {
            Ok(CommandResponse::ok_empty())
        }

        async fn connect(&self) -> Result<()> {
            Ok(())
        }

        async fn disconnect(&self) -> Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_device_registration() {
        let device_info = DeviceInfo::new("test-device", "mock-driver", "test-profile");
        let event_handler = Arc::new(EventHandlerMock);
        let storage_backend = Arc::new(MockStorageBackend);
        let device = Arc::new(MockDevice { info: device_info });
        let device_driver = Arc::new(MockDriver {
            info: crate::driver::DriverInfo::new("mock-driver", "Mock Driver", "mock-protocol"),
            device: device.clone(),
        });
        let (router, _handle) = Router::new(16);

        // router.register_device(device).await.unwrap();
        router.register_event_handler(event_handler).await;
        router.register_driver(device_driver).await;
        router.set_storage(storage_backend).await;

        let devices = router.devices().await;
        assert_eq!(devices.len(), 1);
    }

    // #[tokio::test]
    // async fn test_command_routing() {
    //     let (router, _handle) = Router::new(16);

    //     let info = DeviceInfo::new("test-device", "mock-driver", "test-profile");
    //     let device_id = info.id.clone();
    //     let device = Arc::new(MockDevice { info });

    //     router.register_device(device).await.unwrap();

    //     let command = Command::read(device_id, "temperature");
    //     let response = router.send_command(command).await.unwrap();

    //     assert!(response.success);
    //     assert!(response.result.is_some());
    // }
}
