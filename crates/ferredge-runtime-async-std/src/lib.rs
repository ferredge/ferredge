#[cfg(feature = "serial")]
use std::sync::{Arc as StdArc, Mutex as StdMutex};
#[cfg(any(feature = "runtime", feature = "net"))]
use std::time::Duration;

#[cfg(feature = "net")]
use async_std::{
    io::{ReadExt, WriteExt},
    net::{TcpListener, TcpStream, UdpSocket},
};
#[cfg(feature = "runtime")]
use async_std::{
    sync::{Mutex, MutexGuard},
    task::{self, JoinHandle},
};
#[cfg(any(feature = "runtime", feature = "net", feature = "serial"))]
use ferredge_core::prelude::*;
#[cfg(feature = "runtime")]
#[derive(Debug, Clone, Default)]
pub struct AsyncStdRuntime;

#[cfg(feature = "net")]
#[derive(Debug, Clone, Default)]
pub struct AsyncStdNet;

#[cfg(feature = "serial")]
#[derive(Debug, Clone, Default)]
pub struct AsyncStdSerial;

#[cfg(feature = "runtime")]
pub struct AsyncStdTask<T> {
    handle: JoinHandle<T>,
}

#[cfg(feature = "runtime")]
pub struct AsyncStdSender<T> {
    inner: async_std::channel::Sender<T>,
}

#[cfg(feature = "runtime")]
pub struct AsyncStdReceiver<T> {
    inner: async_std::channel::Receiver<T>,
}

#[cfg(feature = "runtime")]
pub struct AsyncStdMutex<T> {
    inner: Mutex<T>,
}

#[cfg(feature = "runtime")]
pub struct AsyncStdMutexGuard<'a, T> {
    inner: MutexGuard<'a, T>,
}

#[cfg(feature = "runtime")]
#[derive(Clone)]
pub struct AsyncStdInstant {
    inner: std::time::Instant,
}

#[cfg(feature = "runtime")]
impl<T> Clone for AsyncStdSender<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

#[cfg(feature = "net")]
pub struct AsyncStdSocket {
    stream: TcpStream,
    read_timeout: Option<Duration>,
    write_timeout: Option<Duration>,
}

#[cfg(feature = "net")]
pub struct AsyncStdListener {
    listener: TcpListener,
}

#[cfg(feature = "net")]
pub struct AsyncStdDatagramSocket {
    socket: UdpSocket,
    read_timeout: Option<Duration>,
}

#[cfg(feature = "serial")]
pub struct AsyncStdSerialPort {
    port: StdArc<StdMutex<Box<dyn serialport::SerialPort>>>,
}

#[cfg(feature = "runtime")]
pub fn block_on<F: core::future::Future>(future: F) -> F::Output {
    task::block_on(future)
}

#[cfg(feature = "runtime")]
impl AsyncStdRuntime {
    pub fn block_on<F: core::future::Future>(&self, future: F) -> F::Output {
        task::block_on(future)
    }
}

#[cfg(feature = "runtime")]
impl<T> TaskHandle<T> for AsyncStdTask<T>
where
    T: Send + 'static,
{
    async fn join(&mut self) -> Result<T, TaskJoinError> {
        Ok((&mut self.handle).await)
    }

    fn abort(&self) {}

    fn is_finished(&self) -> bool {
        false
    }
}

#[cfg(feature = "runtime")]
impl<T> ChannelSender<T> for AsyncStdSender<T>
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
            Err(async_std::channel::TrySendError::Full(_)) => Err(ChannelError::Full),
            Err(async_std::channel::TrySendError::Closed(_)) => Err(ChannelError::Closed),
        }
    }
}

#[cfg(feature = "runtime")]
impl<T> ChannelReceiver<T> for AsyncStdReceiver<T>
where
    T: Send + 'static,
{
    async fn recv(&mut self) -> Result<T, ChannelError> {
        self.inner.recv().await.map_err(|_| ChannelError::Closed)
    }

    fn try_recv(&mut self) -> Result<T, ChannelError> {
        match self.inner.try_recv() {
            Ok(item) => Ok(item),
            Err(async_std::channel::TryRecvError::Empty) => Err(ChannelError::Empty),
            Err(async_std::channel::TryRecvError::Closed) => Err(ChannelError::Closed),
        }
    }
}

#[cfg(feature = "runtime")]
impl<T> core::ops::Deref for AsyncStdMutexGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

#[cfg(feature = "runtime")]
impl<T> core::ops::DerefMut for AsyncStdMutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

#[cfg(feature = "runtime")]
impl<T> AsyncMutex<T> for AsyncStdMutex<T>
where
    T: Send + 'static,
{
    type Guard<'a>
        = AsyncStdMutexGuard<'a, T>
    where
        T: 'a;

    async fn lock(&self) -> Result<Self::Guard<'_>, MutexError> {
        Ok(AsyncStdMutexGuard {
            inner: self.inner.lock().await,
        })
    }

    fn try_lock(&self) -> Result<Self::Guard<'_>, MutexError> {
        self.inner
            .try_lock()
            .map(|inner| AsyncStdMutexGuard { inner })
            .ok_or(MutexError::Busy)
    }
}

#[cfg(feature = "runtime")]
impl RuntimeInstant for AsyncStdInstant {
    fn elapsed(&self) -> Duration {
        self.inner.elapsed()
    }
}

#[cfg(feature = "runtime")]
impl AsyncRuntime for AsyncStdRuntime {
    type Task<T>
        = AsyncStdTask<T>
    where
        T: Send + 'static;
    type Sender<T>
        = AsyncStdSender<T>
    where
        T: Send + 'static;
    type Receiver<T>
        = AsyncStdReceiver<T>
    where
        T: Send + 'static;
    type Mutex<T>
        = AsyncStdMutex<T>
    where
        T: Send + 'static;
    type Instant = AsyncStdInstant;

    fn spawn<F>(&self, future: F) -> Self::Task<F::Output>
    where
        F: core::future::Future + Send + 'static,
        F::Output: Send + 'static,
    {
        AsyncStdTask {
            handle: task::spawn(future),
        }
    }

    fn channel<T>(&self, capacity: usize) -> (Self::Sender<T>, Self::Receiver<T>)
    where
        T: Send + 'static,
    {
        let (tx, rx) = async_std::channel::bounded(capacity);
        (AsyncStdSender { inner: tx }, AsyncStdReceiver { inner: rx })
    }

    fn mutex<T>(&self, value: T) -> Self::Mutex<T>
    where
        T: Send + 'static,
    {
        AsyncStdMutex {
            inner: Mutex::new(value),
        }
    }

    fn now(&self) -> Self::Instant {
        AsyncStdInstant {
            inner: std::time::Instant::now(),
        }
    }

    async fn sleep(&self, duration: Duration) {
        task::sleep(duration).await;
    }
}

#[cfg(feature = "net")]
impl AsyncStdSocket {
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
impl AsyncSocket for AsyncStdSocket {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, NetError> {
        match self.read_timeout {
            Some(timeout) => async_std::future::timeout(timeout, self.stream.read(buf))
                .await
                .map_err(|_| NetError::TimedOut)?
                .map_err(map_io_error),
            None => self.stream.read(buf).await.map_err(map_io_error),
        }
    }

    async fn write(&mut self, buf: &[u8]) -> Result<usize, NetError> {
        match self.write_timeout {
            Some(timeout) => async_std::future::timeout(timeout, self.stream.write(buf))
                .await
                .map_err(|_| NetError::TimedOut)?
                .map_err(map_io_error),
            None => self.stream.write(buf).await.map_err(map_io_error),
        }
    }

    async fn flush(&mut self) -> Result<(), NetError> {
        match self.write_timeout {
            Some(timeout) => async_std::future::timeout(timeout, self.stream.flush())
                .await
                .map_err(|_| NetError::TimedOut)?
                .map_err(map_io_error),
            None => self.stream.flush().await.map_err(map_io_error),
        }
    }

    async fn close(&mut self) -> Result<(), NetError> {
        Ok(())
    }
}

#[cfg(feature = "net")]
impl AsyncListener for AsyncStdListener {
    type Socket = AsyncStdSocket;

    async fn accept(&mut self) -> Result<Self::Socket, NetError> {
        let (stream, _) = self.listener.accept().await.map_err(map_io_error)?;
        Ok(AsyncStdSocket {
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
impl AsyncStdDatagramSocket {
    pub fn set_read_timeout(&mut self, timeout: Option<Duration>) -> Result<(), NetError> {
        self.read_timeout = timeout;
        Ok(())
    }
}

#[cfg(feature = "net")]
impl AsyncDatagramSocket for AsyncStdDatagramSocket {
    async fn recv_from(&mut self, buf: &mut [u8]) -> Result<(usize, String), NetError> {
        let (size, address) = match self.read_timeout {
            Some(timeout) => async_std::future::timeout(timeout, self.socket.recv_from(buf))
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
impl AsyncNet for AsyncStdNet {
    type Socket = AsyncStdSocket;
    type Listener = AsyncStdListener;

    async fn connect(&self, address: &str) -> Result<Self::Socket, NetError> {
        let stream = TcpStream::connect(address).await.map_err(map_io_error)?;
        Ok(AsyncStdSocket {
            stream,
            read_timeout: None,
            write_timeout: None,
        })
    }

    async fn bind(&self, address: &str) -> Result<Self::Listener, NetError> {
        let listener = TcpListener::bind(address).await.map_err(map_io_error)?;
        Ok(AsyncStdListener { listener })
    }
}

#[cfg(feature = "net")]
impl AsyncDatagramNet for AsyncStdNet {
    type DatagramSocket = AsyncStdDatagramSocket;

    async fn bind_datagram(&self, address: &str) -> Result<Self::DatagramSocket, NetError> {
        let socket = UdpSocket::bind(address).await.map_err(map_io_error)?;
        Ok(AsyncStdDatagramSocket {
            socket,
            read_timeout: None,
        })
    }
}

#[cfg(feature = "serial")]
impl AsyncSerialPort for AsyncStdSerialPort {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, SerialError> {
        let port = self.port.clone();
        let len = buf.len();
        let (n, read_buf) = async_std::task::spawn_blocking(move || {
            let mut local_buf = vec![0u8; len];
            let mut guard = port.lock().map_err(|_| SerialError::RuntimeUnavailable)?;
            let n = guard.read(&mut local_buf).map_err(map_serial_io_error)?;
            Ok::<_, SerialError>((n, local_buf))
        })
        .await?;
        buf[..n].copy_from_slice(&read_buf[..n]);
        Ok(n)
    }

    async fn write(&mut self, buf: &[u8]) -> Result<usize, SerialError> {
        let port = self.port.clone();
        let payload = buf.to_vec();
        async_std::task::spawn_blocking(move || {
            let mut guard = port.lock().map_err(|_| SerialError::RuntimeUnavailable)?;
            guard.write(&payload).map_err(map_serial_io_error)
        })
        .await
    }

    async fn flush(&mut self) -> Result<(), SerialError> {
        let port = self.port.clone();
        async_std::task::spawn_blocking(move || {
            let mut guard = port.lock().map_err(|_| SerialError::RuntimeUnavailable)?;
            guard.flush().map_err(map_serial_io_error)
        })
        .await
    }

    async fn close(&mut self) -> Result<(), SerialError> {
        Ok(())
    }
}

#[cfg(feature = "serial")]
impl AsyncSerial for AsyncStdSerial {
    type Port = AsyncStdSerialPort;

    async fn open(&self, config: &SerialPortConfig) -> Result<Self::Port, SerialError> {
        let path = config.path.clone();
        let baudrate = config.baudrate;
        let data_bits = config.data_bits;
        let parity = config.parity;
        let stop_bits = config.stop_bits;
        let flow_control = config.flow_control;
        let timeout = config.read_timeout.or(config.write_timeout);
        let port = async_std::task::spawn_blocking(move || {
            let mut builder = serialport::new(path, baudrate)
                .data_bits(map_data_bits(data_bits))
                .parity(map_parity(parity))
                .stop_bits(map_stop_bits(stop_bits))
                .flow_control(map_flow_control(flow_control));
            if let Some(timeout) = timeout {
                builder = builder.timeout(timeout);
            }
            builder.open().map_err(map_serial_error)
        })
        .await?;
        Ok(AsyncStdSerialPort {
            port: StdArc::new(StdMutex::new(port)),
        })
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
        _ => NetError::Other("async-std io error"),
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
        _ => SerialError::Other("async-std serial io error"),
    }
}

#[cfg(feature = "serial")]
fn map_serial_error(error: serialport::Error) -> SerialError {
    use serialport::ErrorKind;

    match error.kind() {
        ErrorKind::NoDevice => SerialError::Closed,
        ErrorKind::InvalidInput => SerialError::Unsupported,
        ErrorKind::Io(std::io::ErrorKind::TimedOut) => SerialError::TimedOut,
        _ => SerialError::Other("async-std serial error"),
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
#[allow(unused)]
mod tests {
    use super::*;

    #[cfg(feature = "runtime")]
    #[test]
    fn runtime_spawn_channel_and_mutex_work() {
        let runtime = AsyncStdRuntime;
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
        task::block_on(async {
            let net = AsyncStdNet;
            let mut listener = net.bind("127.0.0.1:0").await.expect("bind should succeed");
            let address = listener
                .listener
                .local_addr()
                .expect("local addr should exist")
                .to_string();
            let connect_address = address.clone();
            let connect_task = task::spawn(async move {
                net.connect(&connect_address)
                    .await
                    .expect("connect should succeed")
            });
            let mut server = listener.accept().await.expect("accept should succeed");
            let mut client = connect_task.await;

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
        task::block_on(async {
            let net = AsyncStdNet;
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
        task::block_on(async {
            let serial = AsyncStdSerial;
            let config = SerialPortConfig {
                path: "/dev/ferredge-missing-serial".to_string(),
                ..SerialPortConfig::default()
            };
            let result = serial.open(&config).await;
            assert!(result.is_err());
        });
    }
}
