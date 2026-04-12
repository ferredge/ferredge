use std::time::Duration;

use async_std::{
    io::{ReadExt, WriteExt},
    net::{TcpListener, TcpStream},
    task::{self, JoinHandle},
};
use ferredge_core::prelude::*;

#[derive(Debug, Clone, Default)]
pub struct AsyncStdRuntime;

#[derive(Debug, Clone, Default)]
pub struct AsyncStdNet;

pub struct AsyncStdTask<T> {
    handle: JoinHandle<T>,
}

pub struct AsyncStdSender<T> {
    inner: async_std::channel::Sender<T>,
}

pub struct AsyncStdReceiver<T> {
    inner: async_std::channel::Receiver<T>,
}

impl<T> Clone for AsyncStdSender<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

pub struct AsyncStdSocket {
    stream: TcpStream,
    read_timeout: Option<Duration>,
    write_timeout: Option<Duration>,
}

pub struct AsyncStdListener {
    listener: TcpListener,
}

pub fn block_on<F: core::future::Future>(future: F) -> F::Output {
    task::block_on(future)
}

impl AsyncStdRuntime {
    pub fn block_on<F: core::future::Future>(&self, future: F) -> F::Output {
        task::block_on(future)
    }
}

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

impl<T> ChannelSender<T> for AsyncStdSender<T>
where
    T: Send + 'static,
{
    async fn send(&self, item: T) -> Result<(), ChannelError> {
        self.inner.send(item).await.map_err(|_| ChannelError::Closed)
    }

    fn try_send(&self, item: T) -> Result<(), ChannelError> {
        match self.inner.try_send(item) {
            Ok(()) => Ok(()),
            Err(async_std::channel::TrySendError::Full(_)) => Err(ChannelError::Full),
            Err(async_std::channel::TrySendError::Closed(_)) => Err(ChannelError::Closed),
        }
    }
}

impl<T> ChannelReceiver<T> for AsyncStdReceiver<T>
where
    T: Send + 'static,
{
    async fn recv(&mut self) -> Result<T, ChannelError> {
        self.inner.recv().await.map_err(|_| ChannelError::Closed)
    }
}

impl AsyncRuntime for AsyncStdRuntime {
    type Task<T> = AsyncStdTask<T> where T: Send + 'static;
    type Sender<T> = AsyncStdSender<T> where T: Send + 'static;
    type Receiver<T> = AsyncStdReceiver<T> where T: Send + 'static;

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
        (
            AsyncStdSender { inner: tx },
            AsyncStdReceiver { inner: rx },
        )
    }

    async fn sleep(&self, duration: Duration) {
        task::sleep(duration).await;
    }
}

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
