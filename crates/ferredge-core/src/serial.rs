use core::future::Future;

use crate::device::SerialPortConfig;

/// Error returned by abstract async serial operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SerialError {
    /// Remote endpoint or device closed the serial session.
    Closed,
    /// Operation timed out before completion.
    TimedOut,
    /// Requested operation is unsupported by this transport.
    Unsupported,
    /// Underlying runtime or driver is unavailable.
    RuntimeUnavailable,
    /// Transport-specific failure string preserved by adapter.
    Other(&'static str),
}

/// Async byte-stream serial port used by protocol adapters such as Modbus RTU or ASCII.
pub trait AsyncSerialPort: Send + Sync + 'static {
    /// Reads bytes into the provided buffer and returns the number of bytes read.
    fn read(&mut self, buf: &mut [u8]) -> impl Future<Output = Result<usize, SerialError>> + Send;

    /// Writes bytes from the provided buffer and returns the number of bytes written.
    fn write(&mut self, buf: &[u8]) -> impl Future<Output = Result<usize, SerialError>> + Send;

    /// Flushes buffered outbound bytes when the transport supports it.
    fn flush(&mut self) -> impl Future<Output = Result<(), SerialError>> + Send;

    /// Closes the port gracefully when the transport supports it.
    fn close(&mut self) -> impl Future<Output = Result<(), SerialError>> + Send;
}

/// Async serial transport factory shared by protocol adapters.
pub trait AsyncSerial: Clone + Send + Sync + 'static {
    /// Open serial port type returned by `open`.
    type Port: AsyncSerialPort;

    /// Opens one configured serial port.
    fn open(
        &self,
        config: &SerialPortConfig,
    ) -> impl Future<Output = Result<Self::Port, SerialError>> + Send;
}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate alloc;
    use alloc::string::ToString;
    use core::{
        pin::Pin,
        task::{Context, Poll, Waker},
    };

    #[derive(Default)]
    struct MockSerialPort {
        read_data: Option<&'static [u8]>,
        write_count: usize,
        closed: bool,
    }

    impl AsyncSerialPort for MockSerialPort {
        async fn read(&mut self, buf: &mut [u8]) -> Result<usize, SerialError> {
            let Some(read_data) = self.read_data.take() else {
                return Err(SerialError::Closed);
            };
            let len = read_data.len().min(buf.len());
            buf[..len].copy_from_slice(&read_data[..len]);
            Ok(len)
        }

        async fn write(&mut self, buf: &[u8]) -> Result<usize, SerialError> {
            self.write_count += buf.len();
            Ok(buf.len())
        }

        async fn flush(&mut self) -> Result<(), SerialError> {
            Ok(())
        }

        async fn close(&mut self) -> Result<(), SerialError> {
            self.closed = true;
            Ok(())
        }
    }

    #[derive(Clone, Default)]
    struct MockSerial;

    impl AsyncSerial for MockSerial {
        type Port = MockSerialPort;

        async fn open(&self, _config: &SerialPortConfig) -> Result<Self::Port, SerialError> {
            Ok(MockSerialPort {
                read_data: Some(b"rtu"),
                write_count: 0,
                closed: false,
            })
        }
    }

    fn block_on<F: Future>(future: F) -> F::Output {
        let waker = Waker::noop();
        let mut context = Context::from_waker(waker);
        let mut future = Pin::from(Box::new(future));
        loop {
            match future.as_mut().poll(&mut context) {
                Poll::Ready(output) => return output,
                Poll::Pending => core::hint::spin_loop(),
            }
        }
    }

    #[test]
    fn mock_async_serial_contract_is_usable() {
        let serial = MockSerial;
        let config = SerialPortConfig {
            path: "/dev/ttyUSB0".to_string(),
            ..SerialPortConfig::default()
        };
        let mut port = block_on(serial.open(&config)).expect("open should succeed");
        let mut buf = [0u8; 8];
        let n = block_on(port.read(&mut buf)).expect("read should succeed");
        assert_eq!(&buf[..n], b"rtu");
        assert_eq!(block_on(port.write(b"abc")), Ok(3));
        assert_eq!(block_on(port.flush()), Ok(()));
        assert_eq!(block_on(port.close()), Ok(()));
    }
}
