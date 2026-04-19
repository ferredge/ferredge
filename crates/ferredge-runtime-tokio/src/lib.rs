#[cfg(feature = "runtime")]
use std::sync::Arc;
#[cfg(any(feature = "runtime", feature = "net"))]
use std::time::Duration;

#[cfg(any(feature = "runtime", feature = "net", feature = "serial"))]
use ferredge_core::prelude::*;
#[cfg(any(feature = "net", feature = "serial"))]
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[cfg(feature = "net")]
use tokio::net::{TcpListener, TcpStream, UdpSocket};
#[cfg(feature = "runtime")]
use tokio::{
    runtime::{Handle, Runtime},
    sync::{Mutex, MutexGuard, mpsc},
    task::JoinHandle,
};
#[cfg(feature = "serial")]
use tokio_serial::{SerialPortBuilderExt, SerialStream};

#[cfg(feature = "runtime")]
#[derive(Clone)]
pub struct TokioRuntime {
    runtime: Arc<Runtime>,
}

#[cfg(feature = "net")]
#[derive(Clone, Default)]
pub struct TokioNet;

#[cfg(feature = "serial")]
#[derive(Clone, Default)]
pub struct TokioSerial;

#[cfg(feature = "runtime")]
pub struct TokioTask<T> {
    handle: JoinHandle<T>,
}

#[cfg(feature = "runtime")]
pub struct TokioSender<T> {
    inner: mpsc::Sender<T>,
}

#[cfg(feature = "runtime")]
pub struct TokioReceiver<T> {
    inner: mpsc::Receiver<T>,
}

#[cfg(feature = "runtime")]
pub struct TokioMutex<T> {
    inner: Mutex<T>,
}

#[cfg(feature = "runtime")]
pub struct TokioMutexGuard<'a, T> {
    inner: MutexGuard<'a, T>,
}

#[cfg(feature = "runtime")]
#[derive(Clone)]
pub struct TokioInstant {
    inner: std::time::Instant,
}

#[cfg(feature = "runtime")]
impl<T> Clone for TokioSender<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

#[cfg(feature = "net")]
pub struct TokioSocket {
    stream: TcpStream,
    read_timeout: Option<Duration>,
    write_timeout: Option<Duration>,
}

#[cfg(feature = "net")]
pub struct TokioListener {
    listener: TcpListener,
}

#[cfg(feature = "net")]
pub struct TokioDatagramSocket {
    socket: UdpSocket,
    read_timeout: Option<Duration>,
}

#[cfg(feature = "serial")]
pub struct TokioSerialPort {
    stream: SerialStream,
}

#[cfg(feature = "runtime")]
impl Default for TokioRuntime {
    fn default() -> Self {
        Self {
            runtime: Arc::new(
                Runtime::new().expect("tokio runtime should initialize for ferredge adapter"),
            ),
        }
    }
}

#[cfg(feature = "runtime")]
impl TokioRuntime {
    pub fn block_on<F: core::future::Future>(&self, future: F) -> F::Output {
        if Handle::try_current().is_ok() {
            tokio::task::block_in_place(|| Handle::current().block_on(future))
        } else {
            self.runtime.block_on(future)
        }
    }
}

#[cfg(feature = "runtime")]
pub fn block_on<F: core::future::Future>(future: F) -> F::Output {
    TokioRuntime::default().block_on(future)
}

#[cfg(feature = "runtime")]
impl<T> TaskHandle<T> for TokioTask<T>
where
    T: Send + 'static,
{
    async fn join(&mut self) -> Result<T, TaskJoinError> {
        (&mut self.handle)
            .await
            .map_err(|_| TaskJoinError::Cancelled)
    }

    fn abort(&self) {
        self.handle.abort();
    }

    fn is_finished(&self) -> bool {
        self.handle.is_finished()
    }
}

#[cfg(feature = "runtime")]
impl<T> ChannelSender<T> for TokioSender<T>
where
    T: Send + 'static,
{
    async fn send(&self, item: T) -> Result<(), ChannelError> {
        self.inner
            .send(item)
            .await
            .map_err(|_| ChannelError::Closed)
    }

    fn try_send(&self, item: T) -> Result<(), ChannelError> {
        match self.inner.try_send(item) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(_)) => Err(ChannelError::Full),
            Err(mpsc::error::TrySendError::Closed(_)) => Err(ChannelError::Closed),
        }
    }
}

#[cfg(feature = "runtime")]
impl<T> ChannelReceiver<T> for TokioReceiver<T>
where
    T: Send + 'static,
{
    async fn recv(&mut self) -> Result<T, ChannelError> {
        self.inner.recv().await.ok_or(ChannelError::Closed)
    }

    fn try_recv(&mut self) -> Result<T, ChannelError> {
        match self.inner.try_recv() {
            Ok(item) => Ok(item),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => Err(ChannelError::Empty),
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => Err(ChannelError::Closed),
        }
    }
}

#[cfg(feature = "runtime")]
impl<T> core::ops::Deref for TokioMutexGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

#[cfg(feature = "runtime")]
impl<T> core::ops::DerefMut for TokioMutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

#[cfg(feature = "runtime")]
impl<T> AsyncMutex<T> for TokioMutex<T>
where
    T: Send + 'static,
{
    type Guard<'a>
        = TokioMutexGuard<'a, T>
    where
        T: 'a;

    async fn lock(&self) -> Result<Self::Guard<'_>, MutexError> {
        Ok(TokioMutexGuard {
            inner: self.inner.lock().await,
        })
    }

    fn try_lock(&self) -> Result<Self::Guard<'_>, MutexError> {
        self.inner
            .try_lock()
            .map(|inner| TokioMutexGuard { inner })
            .map_err(|_| MutexError::Busy)
    }
}

#[cfg(feature = "runtime")]
impl RuntimeInstant for TokioInstant {
    fn elapsed(&self) -> Duration {
        self.inner.elapsed()
    }
}

#[cfg(feature = "runtime")]
impl AsyncRuntime for TokioRuntime {
    type Task<T>
        = TokioTask<T>
    where
        T: Send + 'static;
    type Sender<T>
        = TokioSender<T>
    where
        T: Send + 'static;
    type Receiver<T>
        = TokioReceiver<T>
    where
        T: Send + 'static;
    type Mutex<T>
        = TokioMutex<T>
    where
        T: Send + 'static;
    type Instant = TokioInstant;

    fn spawn<F>(&self, future: F) -> Self::Task<F::Output>
    where
        F: core::future::Future + Send + 'static,
        F::Output: Send + 'static,
    {
        TokioTask {
            handle: self.runtime.spawn(future),
        }
    }

    fn channel<T>(&self, capacity: usize) -> (Self::Sender<T>, Self::Receiver<T>)
    where
        T: Send + 'static,
    {
        let (tx, rx) = mpsc::channel(capacity);
        (TokioSender { inner: tx }, TokioReceiver { inner: rx })
    }

    fn mutex<T>(&self, value: T) -> Self::Mutex<T>
    where
        T: Send + 'static,
    {
        TokioMutex {
            inner: Mutex::new(value),
        }
    }

    fn now(&self) -> Self::Instant {
        TokioInstant {
            inner: std::time::Instant::now(),
        }
    }

    async fn sleep(&self, duration: Duration) {
        tokio::time::sleep(duration).await;
    }
}

#[cfg(feature = "net")]
impl TokioSocket {
    pub fn set_read_timeout(&mut self, timeout: Option<Duration>) -> Result<(), NetError> {
        self.read_timeout = timeout;
        Ok(())
    }

    pub fn set_write_timeout(&mut self, timeout: Option<Duration>) -> Result<(), NetError> {
        self.write_timeout = timeout;
        Ok(())
    }
}

#[cfg(feature = "net")]
impl AsyncSocket for TokioSocket {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, NetError> {
        match self.read_timeout {
            Some(timeout) => tokio::time::timeout(timeout, self.stream.read(buf))
                .await
                .map_err(|_| NetError::TimedOut)?
                .map_err(map_io_error),
            None => self.stream.read(buf).await.map_err(map_io_error),
        }
    }

    async fn write(&mut self, buf: &[u8]) -> Result<usize, NetError> {
        match self.write_timeout {
            Some(timeout) => tokio::time::timeout(timeout, self.stream.write(buf))
                .await
                .map_err(|_| NetError::TimedOut)?
                .map_err(map_io_error),
            None => self.stream.write(buf).await.map_err(map_io_error),
        }
    }

    async fn flush(&mut self) -> Result<(), NetError> {
        match self.write_timeout {
            Some(timeout) => tokio::time::timeout(timeout, self.stream.flush())
                .await
                .map_err(|_| NetError::TimedOut)?
                .map_err(map_io_error),
            None => self.stream.flush().await.map_err(map_io_error),
        }
    }

    async fn close(&mut self) -> Result<(), NetError> {
        self.stream.shutdown().await.map_err(map_io_error)
    }
}

#[cfg(feature = "net")]
impl AsyncListener for TokioListener {
    type Socket = TokioSocket;

    async fn accept(&mut self) -> Result<Self::Socket, NetError> {
        let (stream, _) = self.listener.accept().await.map_err(map_io_error)?;
        Ok(TokioSocket {
            stream,
            read_timeout: None,
            write_timeout: None,
        })
    }

    async fn close(&mut self) -> Result<(), NetError> {
        Ok(())
    }
}

#[cfg(feature = "net")]
impl TokioDatagramSocket {
    pub fn set_read_timeout(&mut self, timeout: Option<Duration>) -> Result<(), NetError> {
        self.read_timeout = timeout;
        Ok(())
    }
}

#[cfg(feature = "net")]
impl AsyncDatagramSocket for TokioDatagramSocket {
    async fn recv_from(&mut self, buf: &mut [u8]) -> Result<(usize, String), NetError> {
        let (size, address) = match self.read_timeout {
            Some(timeout) => tokio::time::timeout(timeout, self.socket.recv_from(buf))
                .await
                .map_err(|_| NetError::TimedOut)?
                .map_err(map_io_error)?,
            None => self.socket.recv_from(buf).await.map_err(map_io_error)?,
        };
        Ok((size, address.to_string()))
    }

    async fn send_to(&mut self, buf: &[u8], address: &str) -> Result<usize, NetError> {
        self.socket
            .send_to(buf, address)
            .await
            .map_err(map_io_error)
    }

    async fn close(&mut self) -> Result<(), NetError> {
        Ok(())
    }
}

#[cfg(feature = "net")]
impl AsyncNet for TokioNet {
    type Socket = TokioSocket;
    type Listener = TokioListener;

    async fn connect(&self, address: &str) -> Result<Self::Socket, NetError> {
        let stream = TcpStream::connect(address).await.map_err(map_io_error)?;
        Ok(TokioSocket {
            stream,
            read_timeout: None,
            write_timeout: None,
        })
    }

    async fn bind(&self, address: &str) -> Result<Self::Listener, NetError> {
        let listener = TcpListener::bind(address).await.map_err(map_io_error)?;
        Ok(TokioListener { listener })
    }
}

#[cfg(feature = "net")]
impl AsyncDatagramNet for TokioNet {
    type DatagramSocket = TokioDatagramSocket;

    async fn bind_datagram(&self, address: &str) -> Result<Self::DatagramSocket, NetError> {
        let socket = UdpSocket::bind(address).await.map_err(map_io_error)?;
        Ok(TokioDatagramSocket {
            socket,
            read_timeout: None,
        })
    }
}

#[cfg(feature = "serial")]
impl AsyncSerialPort for TokioSerialPort {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, SerialError> {
        self.stream.read(buf).await.map_err(map_serial_io_error)
    }

    async fn write(&mut self, buf: &[u8]) -> Result<usize, SerialError> {
        self.stream.write(buf).await.map_err(map_serial_io_error)
    }

    async fn flush(&mut self) -> Result<(), SerialError> {
        self.stream.flush().await.map_err(map_serial_io_error)
    }

    async fn close(&mut self) -> Result<(), SerialError> {
        self.stream.shutdown().await.map_err(map_serial_io_error)
    }
}

#[cfg(feature = "serial")]
impl AsyncSerial for TokioSerial {
    type Port = TokioSerialPort;

    async fn open(&self, config: &SerialPortConfig) -> Result<Self::Port, SerialError> {
        let mut builder = serialport::new(config.path.clone(), config.baudrate)
            .data_bits(map_data_bits(config.data_bits))
            .parity(map_parity(config.parity))
            .stop_bits(map_stop_bits(config.stop_bits))
            .flow_control(map_flow_control(config.flow_control));
        if let Some(timeout) = config.read_timeout.or(config.write_timeout) {
            builder = builder.timeout(timeout);
        }

        let stream = builder.open_native_async().map_err(map_serial_error)?;
        Ok(TokioSerialPort { stream })
    }
}

#[cfg(feature = "net")]
fn map_io_error(error: std::io::Error) -> NetError {
    use std::io::ErrorKind;

    match error.kind() {
        ErrorKind::BrokenPipe
        | ErrorKind::ConnectionAborted
        | ErrorKind::ConnectionReset
        | ErrorKind::UnexpectedEof
        | ErrorKind::NotConnected => NetError::Closed,
        ErrorKind::TimedOut | ErrorKind::WouldBlock => NetError::TimedOut,
        ErrorKind::AddrInUse
        | ErrorKind::AddrNotAvailable
        | ErrorKind::ConnectionRefused
        | ErrorKind::HostUnreachable
        | ErrorKind::NetworkUnreachable => NetError::Unreachable,
        ErrorKind::Unsupported => NetError::Unsupported,
        _ => NetError::Other("tokio io error"),
    }
}

#[cfg(feature = "serial")]
fn map_serial_io_error(error: std::io::Error) -> SerialError {
    use std::io::ErrorKind;

    match error.kind() {
        ErrorKind::BrokenPipe
        | ErrorKind::ConnectionAborted
        | ErrorKind::ConnectionReset
        | ErrorKind::UnexpectedEof
        | ErrorKind::NotConnected => SerialError::Closed,
        ErrorKind::TimedOut | ErrorKind::WouldBlock => SerialError::TimedOut,
        ErrorKind::Unsupported => SerialError::Unsupported,
        _ => SerialError::Other("tokio serial io error"),
    }
}

#[cfg(feature = "serial")]
fn map_serial_error(error: serialport::Error) -> SerialError {
    use serialport::ErrorKind;

    match error.kind() {
        ErrorKind::NoDevice => SerialError::Closed,
        ErrorKind::InvalidInput => SerialError::Unsupported,
        ErrorKind::Io(std::io::ErrorKind::TimedOut) => SerialError::TimedOut,
        _ => SerialError::Other("tokio serial error"),
    }
}

#[cfg(feature = "serial")]
fn map_data_bits(bits: SerialDataBits) -> serialport::DataBits {
    match bits {
        SerialDataBits::Five => serialport::DataBits::Five,
        SerialDataBits::Six => serialport::DataBits::Six,
        SerialDataBits::Seven => serialport::DataBits::Seven,
        SerialDataBits::Eight => serialport::DataBits::Eight,
    }
}

#[cfg(feature = "serial")]
fn map_parity(parity: SerialParity) -> serialport::Parity {
    match parity {
        SerialParity::None => serialport::Parity::None,
        SerialParity::Even => serialport::Parity::Even,
        SerialParity::Odd => serialport::Parity::Odd,
    }
}

#[cfg(feature = "serial")]
fn map_stop_bits(bits: SerialStopBits) -> serialport::StopBits {
    match bits {
        SerialStopBits::One => serialport::StopBits::One,
        SerialStopBits::Two => serialport::StopBits::Two,
    }
}

#[cfg(feature = "serial")]
fn map_flow_control(control: SerialFlowControl) -> serialport::FlowControl {
    match control {
        SerialFlowControl::None => serialport::FlowControl::None,
        SerialFlowControl::Software => serialport::FlowControl::Software,
        SerialFlowControl::Hardware => serialport::FlowControl::Hardware,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "runtime")]
    #[test]
    fn runtime_spawn_channel_and_mutex_work() {
        let runtime = TokioRuntime::default();
        runtime.block_on(async {
            let (tx, mut rx) = runtime.channel(2);
            tx.send(7u8).await.expect("send should succeed");
            assert_eq!(rx.recv().await, Ok(7));

            let mutex = runtime.mutex(11u8);
            {
                let mut guard = mutex.lock().await.expect("lock should succeed");
                *guard = 13;
            }
            let mut task = runtime.spawn(async { 29u8 });
            assert_eq!(task.join().await, Ok(29));
        });
    }

    #[cfg(feature = "net")]
    #[test]
    fn net_tcp_loopback_works() {
        let runtime = TokioRuntime::default();
        runtime.block_on(async {
            let net = TokioNet;
            let mut listener = net.bind("127.0.0.1:0").await.expect("bind should succeed");
            let address = listener
                .listener
                .local_addr()
                .expect("local addr should exist")
                .to_string();
            let connect_address = address.clone();
            let connect_task = tokio::spawn(async move {
                net.connect(&connect_address)
                    .await
                    .expect("connect should succeed")
            });
            let mut server = listener.accept().await.expect("accept should succeed");
            let mut client = connect_task.await.expect("join should succeed");

            client.write(b"ping").await.expect("write should succeed");
            client.flush().await.expect("flush should succeed");
            let mut buf = [0u8; 8];
            let n = server.read(&mut buf).await.expect("read should succeed");
            assert_eq!(&buf[..n], b"ping");
        });
    }

    #[cfg(feature = "net")]
    #[test]
    fn net_udp_loopback_works() {
        let runtime = TokioRuntime::default();
        runtime.block_on(async {
            let net = TokioNet;
            let mut server = net
                .bind_datagram("127.0.0.1:0")
                .await
                .expect("server bind should succeed");
            let mut client = net
                .bind_datagram("127.0.0.1:0")
                .await
                .expect("client bind should succeed");
            let server_addr = server
                .socket
                .local_addr()
                .expect("local addr should exist")
                .to_string();

            client
                .send_to(b"udp", &server_addr)
                .await
                .expect("send_to should succeed");
            let mut buf = [0u8; 8];
            let (n, _) = server
                .recv_from(&mut buf)
                .await
                .expect("recv should succeed");
            assert_eq!(&buf[..n], b"udp");
        });
    }

    #[cfg(feature = "serial")]
    #[test]
    fn serial_open_nonexistent_path_fails() {
        let runtime = TokioRuntime::default();
        runtime.block_on(async {
            let serial = TokioSerial;
            let config = SerialPortConfig {
                path: "/dev/ferredge-missing-serial".to_string(),
                ..SerialPortConfig::default()
            };
            let result = serial.open(&config).await;
            assert!(result.is_err());
        });
    }
}
