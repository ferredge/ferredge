use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use core::cell::RefCell;
use core::future::Future;
use core::pin::Pin;

use embassy_sync::blocking_mutex::Mutex as BlockingMutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embedded_io_async::ErrorKind;
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

/// Object-safe erasure of an `embedded-io-async` byte stream.
///
/// `embedded_io_async::Read`/`Write` use async trait methods, so they have no `dyn` form;
/// this boxes the futures (alloc) to recover one. Errors collapse to their [`ErrorKind`].
trait ErasedSerialIo {
    fn read<'a>(
        &'a mut self,
        buf: &'a mut [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, ErrorKind>> + 'a>>;

    fn write<'a>(
        &'a mut self,
        buf: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, ErrorKind>> + 'a>>;

    fn flush<'a>(&'a mut self) -> Pin<Box<dyn Future<Output = Result<(), ErrorKind>> + 'a>>;
}

impl<T> ErasedSerialIo for T
where
    T: embedded_io_async::Read + embedded_io_async::Write,
{
    fn read<'a>(
        &'a mut self,
        buf: &'a mut [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, ErrorKind>> + 'a>> {
        Box::pin(async move {
            embedded_io_async::Read::read(self, buf)
                .await
                .map_err(|error| embedded_io_async::Error::kind(&error))
        })
    }

    fn write<'a>(
        &'a mut self,
        buf: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, ErrorKind>> + 'a>> {
        Box::pin(async move {
            embedded_io_async::Write::write(self, buf)
                .await
                .map_err(|error| embedded_io_async::Error::kind(&error))
        })
    }

    fn flush<'a>(&'a mut self) -> Pin<Box<dyn Future<Output = Result<(), ErrorKind>> + 'a>> {
        Box::pin(async move {
            embedded_io_async::Write::flush(self)
                .await
                .map_err(|error| embedded_io_async::Error::kind(&error))
        })
    }
}

/// Boxed, type-erased serial device.
///
/// Lets heterogeneous HAL ports live behind one concrete type, which protocol crates need
/// for their non-generic `StackSerial`/`StackSerialPort` aliases. Costs one heap-boxed
/// future per operation; use [`EmbassySerialPort`] directly when the device type is known.
pub struct EmbassyDynSerialPort {
    inner: Box<dyn ErasedSerialIo>,
}

impl EmbassyDynSerialPort {
    pub fn new(inner: impl embedded_io_async::Read + embedded_io_async::Write + 'static) -> Self {
        Self {
            inner: Box::new(inner),
        }
    }
}

impl embedded_io_async::ErrorType for EmbassyDynSerialPort {
    type Error = ErrorKind;
}

impl embedded_io_async::Read for EmbassyDynSerialPort {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        self.inner.read(buf).await
    }
}

impl embedded_io_async::Write for EmbassyDynSerialPort {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        self.inner.write(buf).await
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        self.inner.flush().await
    }
}

/// [`EmbassySerial`] registry over type-erased ports — the concrete factory protocol
/// crates alias as `StackSerial`.
pub type EmbassyDynSerial = EmbassySerial<EmbassyDynSerialPort>;

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
