//! SDK client for interacting with ferredge.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{broadcast, mpsc, oneshot};
use tracing::{debug, info};

use ferredge_core::{
    device::DeviceId,
    event::Event,
    message::{Command, CommandResponse, Reading},
    router::Router,
    types::Value,
};

use crate::error::{SdkError, SdkResult};

/// Client for interacting with ferredge.
///
/// The client can be used in two modes:
/// 1. Embedded mode: Direct reference to a Router instance
/// 2. Remote mode: Connection over network (future implementation)
pub struct Client {
    inner: ClientInner,
    /// Default timeout for operations
    timeout: Duration,
}

enum ClientInner {
    /// Direct connection to an embedded router
    Embedded(Arc<Router>),
    // Future: Remote connection
    // Remote { ... }
}

impl Client {
    /// Creates a client connected to an embedded router.
    pub fn embedded(router: Arc<Router>) -> Self {
        Self {
            inner: ClientInner::Embedded(router),
            timeout: Duration::from_secs(30),
        }
    }

    /// Sets the default timeout for operations.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Reads a value from a device resource.
    pub async fn read(&self, device_id: impl Into<DeviceId>, resource: &str) -> SdkResult<Reading> {
        let device_id = device_id.into();
        debug!(device_id = %device_id, resource = %resource, "Reading from device");

        let command = Command::read(device_id.clone(), resource);
        let response = self.send_command(command).await?;

        if response.success {
            Ok(Reading::new(
                device_id,
                resource,
                response.result.unwrap_or(Value::Null),
            ))
        } else {
            Err(SdkError::CommandFailed(
                response.error.unwrap_or_else(|| "Unknown error".to_string()),
            ))
        }
    }

    /// Writes a value to a device resource.
    pub async fn write(
        &self,
        device_id: impl Into<DeviceId>,
        resource: &str,
        value: Value,
    ) -> SdkResult<()> {
        let device_id = device_id.into();
        debug!(device_id = %device_id, resource = %resource, "Writing to device");

        let command = Command::write(device_id, resource, value);
        let response = self.send_command(command).await?;

        if response.success {
            Ok(())
        } else {
            Err(SdkError::CommandFailed(
                response.error.unwrap_or_else(|| "Unknown error".to_string()),
            ))
        }
    }

    /// Executes a command on a device.
    pub async fn execute(
        &self,
        device_id: impl Into<DeviceId>,
        command_name: &str,
        args: serde_json::Value,
    ) -> SdkResult<CommandResponse> {
        let device_id = device_id.into();
        debug!(device_id = %device_id, command = %command_name, "Executing command");

        let command = Command::execute(device_id, command_name, args);
        let response = self.send_command(command).await?;

        if response.success {
            Ok(response)
        } else {
            Err(SdkError::CommandFailed(
                response.error.unwrap_or_else(|| "Unknown error".to_string()),
            ))
        }
    }

    /// Sends a command and waits for a response.
    async fn send_command(&self, command: Command) -> SdkResult<CommandResponse> {
        match &self.inner {
            ClientInner::Embedded(router) => {
                let response = tokio::time::timeout(self.timeout, router.send_command(command))
                    .await
                    .map_err(|_| SdkError::Timeout)?
                    .map_err(SdkError::Core)?;
                Ok(response)
            }
        }
    }

    /// Subscribes to events.
    pub fn subscribe_events(&self) -> SdkResult<broadcast::Receiver<Event>> {
        match &self.inner {
            ClientInner::Embedded(router) => Ok(router.subscribe_events()),
        }
    }

    /// Gets device information.
    pub async fn get_device(&self, device_id: impl Into<DeviceId>) -> SdkResult<Option<ferredge_core::DeviceInfo>> {
        let device_id = device_id.into();
        match &self.inner {
            ClientInner::Embedded(router) => {
                Ok(router.get_device(&device_id).await.map(|d| d.info().clone()))
            }
        }
    }

    /// Lists all devices.
    pub async fn list_devices(&self) -> SdkResult<Vec<ferredge_core::DeviceInfo>> {
        match &self.inner {
            ClientInner::Embedded(router) => {
                let devices = router.devices().await;
                Ok(devices.iter().map(|d| d.info().clone()).collect())
            }
        }
    }
}

/// Builder for creating event subscriptions with filters.
pub struct EventSubscription {
    receiver: broadcast::Receiver<Event>,
}

impl EventSubscription {
    /// Creates a new subscription from a receiver.
    pub fn new(receiver: broadcast::Receiver<Event>) -> Self {
        Self { receiver }
    }

    /// Receives the next event.
    pub async fn recv(&mut self) -> Option<Event> {
        self.receiver.recv().await.ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferredge_core::device::{DeviceInfo, DeviceState};
    use ferredge_core::router::Router;

    struct MockDevice {
        info: DeviceInfo,
    }

    impl ferredge_core::Device for MockDevice {
        fn info(&self) -> &DeviceInfo {
            &self.info
        }

        fn state(&self) -> DeviceState {
            DeviceState::Online
        }

        async fn read(&self, resource: &str) -> ferredge_core::Result<Reading> {
            Ok(Reading::new(self.info.id.clone(), resource, Value::Float(42.0)))
        }

        async fn write(&self, _resource: &str, _value: Value) -> ferredge_core::Result<()> {
            Ok(())
        }

        async fn execute(
            &self,
            _command: Command,
        ) -> ferredge_core::Result<CommandResponse> {
            Ok(CommandResponse::ok_empty())
        }

        async fn connect(&self) -> ferredge_core::Result<()> {
            Ok(())
        }

        async fn disconnect(&self) -> ferredge_core::Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_embedded_client() {
        let (router, _handle) = Router::new(16);
        let router = Arc::new(router);

        let info = DeviceInfo::new("test-device", "mock", "test-profile");
        let device = Arc::new(MockDevice { info });

        router.register_device(device).await.unwrap();

        let client = Client::embedded(router);
        let reading = client.read("test-device", "temperature").await.unwrap();

        assert_eq!(reading.value.as_float(), Some(42.0));
    }
}
