use alloc::format;
use alloc::string::String;
use core::cell::RefCell;

use embassy_sync::blocking_mutex::Mutex as BlockingMutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use ferredge_core::prelude::*;

/// [`AsyncSerialPort`] adapter over any [`embedded_io_async`] byte stream, e.g. a HAL
/// UART or an embassy-usb CDC-ACM class.
pub struct EmbassySerialPort<T> {
    inner: T,
}

impl<T> EmbassySerialPort<T> {
    pub fn new(inner: T) -> Self {
        Self { inner }
    }

    pub fn into_inner(self) -> T {
        self.inner
    }
}

impl<T> AsyncSerialPort for EmbassySerialPort<T>
where
    T: embedded_io_async::Read + embedded_io_async::Write + 'static,
{
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, SerialError> {
        self.inner.read(buf).await.map_err(map_embedded_io_error)
    }

    async fn write(&mut self, buf: &[u8]) -> Result<usize, SerialError> {
        self.inner.write(buf).await.map_err(map_embedded_io_error)
    }

    async fn flush(&mut self) -> Result<(), SerialError> {
        self.inner.flush().await.map_err(map_embedded_io_error)
    }

    async fn close(&mut self) -> Result<(), SerialError> {
        // embedded-io streams have no close notion; drain buffered output instead.
        self.inner.flush().await.map_err(map_embedded_io_error)
    }
}

/// [`AsyncSerial`] factory backed by ports registered up front.
///
/// Serial hardware on embassy targets is HAL-specific and configured (baudrate, framing,
/// pins) when the peripheral is initialized, so this factory cannot open ports by path.
/// Register each configured port under the path protocol code will ask for; `open` hands
/// the port out once and ignores the [`SerialPortConfig`] line settings.
pub struct EmbassySerial<T> {
    ports: Shared<BlockingMutex<CriticalSectionRawMutex, RefCell<Map<String, T>>>>,
}

impl<T> Clone for EmbassySerial<T> {
    fn clone(&self) -> Self {
        Self {
            ports: self.ports.clone(),
        }
    }
}

impl<T> Default for EmbassySerial<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> EmbassySerial<T> {
    pub fn new() -> Self {
        Self {
            ports: Shared::new(BlockingMutex::new(RefCell::new(Map::new()))),
        }
    }

    pub fn register(&self, path: impl Into<String>, port: T) {
        self.ports
            .lock(|ports| ports.borrow_mut().insert(path.into(), port));
    }
}

impl<T> AsyncSerial for EmbassySerial<T>
where
    T: embedded_io_async::Read + embedded_io_async::Write + 'static,
{
    type Port = EmbassySerialPort<T>;

    async fn open(&self, config: &SerialPortConfig) -> Result<Self::Port, SerialError> {
        self.ports
            .lock(|ports| ports.borrow_mut().remove(&config.path))
            .map(EmbassySerialPort::new)
            .ok_or_else(|| {
                SerialError::Other(format!(
                    "no embassy serial port registered as {}",
                    config.path
                ))
            })
    }
}

fn map_embedded_io_error<E: embedded_io_async::Error>(error: E) -> SerialError {
    use embedded_io_async::ErrorKind;

    match error.kind() {
        ErrorKind::BrokenPipe
        | ErrorKind::ConnectionAborted
        | ErrorKind::ConnectionReset
        | ErrorKind::NotConnected => SerialError::Closed,
        ErrorKind::TimedOut => SerialError::TimedOut,
        ErrorKind::Unsupported => SerialError::Unsupported,
        _ => SerialError::Other("embassy serial io error".into()),
    }
}
