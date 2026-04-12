use core::future::Future;

/// Error returned by abstract async network operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetError {
    /// Remote endpoint closed the connection.
    Closed,
    /// Operation timed out before completion.
    TimedOut,
    /// Address could not be resolved or connected.
    Unreachable,
    /// Requested operation is unsupported by this transport.
    Unsupported,
    /// Underlying runtime or driver is unavailable.
    RuntimeUnavailable,
    /// Transport-specific failure string preserved by adapter.
    Other(&'static str),
}

/// Async bidirectional byte stream used by protocol adapters.
pub trait AsyncSocket: Send + Sync + 'static {
    /// Reads bytes into the provided buffer and returns the number of bytes read.
    fn read(&mut self, buf: &mut [u8]) -> impl Future<Output = Result<usize, NetError>> + Send;

    /// Writes bytes from the provided buffer and returns the number of bytes written.
    fn write(&mut self, buf: &[u8]) -> impl Future<Output = Result<usize, NetError>> + Send;

    /// Flushes buffered outbound bytes when the transport supports it.
    fn flush(&mut self) -> impl Future<Output = Result<(), NetError>> + Send;

    /// Closes the socket gracefully when the transport supports it.
    fn close(&mut self) -> impl Future<Output = Result<(), NetError>> + Send;
}

/// Async listener that accepts inbound sockets.
pub trait AsyncListener: Send + Sync + 'static {
    /// Socket type produced by this listener.
    type Socket: AsyncSocket;

    /// Accepts one inbound connection from the listener.
    fn accept(&mut self) -> impl Future<Output = Result<Self::Socket, NetError>> + Send;

    /// Closes the listener and releases bound resources.
    fn close(&mut self) -> impl Future<Output = Result<(), NetError>> + Send;
}

/// Async network transport factory shared by protocol adapters.
///
/// Implementations are intended to bridge concrete ecosystems such as Tokio, async-std,
/// or embassy-net into a transport-neutral socket model consumed by protocol crates.
pub trait AsyncNet: Clone + Send + Sync + 'static {
    /// Connected socket type returned by `connect`.
    type Socket: AsyncSocket;
    /// Listener type returned by `bind`.
    type Listener: AsyncListener<Socket = Self::Socket>;

    /// Establishes one outbound byte-stream connection to the given address.
    fn connect(&self, address: &str)
        -> impl Future<Output = Result<Self::Socket, NetError>> + Send;

    /// Binds one listener to the given local address when supported.
    fn bind(&self, address: &str)
        -> impl Future<Output = Result<Self::Listener, NetError>> + Send;
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::{
        pin::Pin,
        task::{Context, Poll, Waker},
    };

    #[derive(Default)]
    struct MockSocket {
        read_data: Option<&'static [u8]>,
        write_count: usize,
        closed: bool,
    }

    impl AsyncSocket for MockSocket {
        async fn read(&mut self, buf: &mut [u8]) -> Result<usize, NetError> {
            let Some(read_data) = self.read_data.take() else {
                return Err(NetError::Closed);
            };
            let len = read_data.len().min(buf.len());
            buf[..len].copy_from_slice(&read_data[..len]);
            Ok(len)
        }

        async fn write(&mut self, buf: &[u8]) -> Result<usize, NetError> {
            self.write_count += buf.len();
            Ok(buf.len())
        }

        async fn flush(&mut self) -> Result<(), NetError> {
            Ok(())
        }

        async fn close(&mut self) -> Result<(), NetError> {
            self.closed = true;
            Ok(())
        }
    }

    struct MockListener {
        accepted: bool,
    }

    impl AsyncListener for MockListener {
        type Socket = MockSocket;

        async fn accept(&mut self) -> Result<Self::Socket, NetError> {
            if self.accepted {
                Err(NetError::Closed)
            } else {
                self.accepted = true;
                Ok(MockSocket {
                    read_data: Some(b"ping"),
                    write_count: 0,
                    closed: false,
                })
            }
        }

        async fn close(&mut self) -> Result<(), NetError> {
            self.accepted = true;
            Ok(())
        }
    }

    #[derive(Clone, Default)]
    struct MockNet;

    impl AsyncNet for MockNet {
        type Socket = MockSocket;
        type Listener = MockListener;

        async fn connect(&self, _address: &str) -> Result<Self::Socket, NetError> {
            Ok(MockSocket {
                read_data: Some(b"pong"),
                write_count: 0,
                closed: false,
            })
        }

        async fn bind(&self, _address: &str) -> Result<Self::Listener, NetError> {
            Ok(MockListener { accepted: false })
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
    fn mock_async_net_contract_is_usable() {
        let net = MockNet;
        let mut socket = block_on(net.connect("example:1883")).expect("connect should succeed");
        let mut buf = [0u8; 8];
        let n = block_on(socket.read(&mut buf)).expect("read should succeed");
        assert_eq!(&buf[..n], b"pong");
        assert_eq!(block_on(socket.write(b"abc")), Ok(3));
        assert_eq!(block_on(socket.flush()), Ok(()));
        assert_eq!(block_on(socket.close()), Ok(()));

        let mut listener = block_on(net.bind("127.0.0.1:0")).expect("bind should succeed");
        let mut accepted = block_on(listener.accept()).expect("accept should succeed");
        let n = block_on(accepted.read(&mut buf)).expect("accepted read should succeed");
        assert_eq!(&buf[..n], b"ping");
        assert_eq!(block_on(listener.close()), Ok(()));
    }
}
