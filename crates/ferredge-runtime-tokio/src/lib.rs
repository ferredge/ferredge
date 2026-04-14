use std::{sync::Arc, time::Duration};

use ferredge_core::prelude::*;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    runtime::{Handle, Runtime},
    sync::{Mutex, MutexGuard, mpsc},
    task::JoinHandle,
};

#[derive(Clone)]
pub struct TokioRuntime {
    runtime: Arc<Runtime>,
}

#[derive(Clone, Default)]
pub struct TokioNet;

pub struct TokioTask<T> {
    handle: JoinHandle<T>,
}

pub struct TokioSender<T> {
    inner: mpsc::Sender<T>,
}

pub struct TokioReceiver<T> {
    inner: mpsc::Receiver<T>,
}

pub struct TokioMutex<T> {
    inner: Mutex<T>,
}

pub struct TokioMutexGuard<'a, T> {
    inner: MutexGuard<'a, T>,
}

#[derive(Clone)]
pub struct TokioInstant {
    inner: std::time::Instant,
}

impl<T> Clone for TokioSender<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

pub struct TokioSocket {
    stream: TcpStream,
    read_timeout: Option<Duration>,
    write_timeout: Option<Duration>,
}

pub struct TokioListener {
    listener: TcpListener,
}

impl Default for TokioRuntime {
    fn default() -> Self {
        Self {
            runtime: Arc::new(
                Runtime::new().expect("tokio runtime should initialize for ferredge adapter"),
            ),
        }
    }
}

impl TokioRuntime {
    pub fn block_on<F: core::future::Future>(&self, future: F) -> F::Output {
        if Handle::try_current().is_ok() {
            tokio::task::block_in_place(|| Handle::current().block_on(future))
        } else {
            self.runtime.block_on(future)
        }
    }
}

pub fn block_on<F: core::future::Future>(future: F) -> F::Output {
    TokioRuntime::default().block_on(future)
}

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

impl<T> core::ops::Deref for TokioMutexGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<T> core::ops::DerefMut for TokioMutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

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

impl RuntimeInstant for TokioInstant {
    fn elapsed(&self) -> Duration {
        self.inner.elapsed()
    }
}

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
