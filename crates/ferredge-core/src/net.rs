extern crate alloc;

use alloc::format;
use alloc::string::{String, ToString};
use core::future::Future;
use core::time::Duration;

use crate::maybe::{MaybeSend, MaybeSync};

/// Error returned by abstract async network operations.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NetError {
    /// Remote endpoint closed the connection.
    #[error("network transport is closed")]
    Closed,
    /// Operation timed out before completion.
    #[error("network operation timed out")]
    TimedOut,
    /// Address could not be resolved or connected.
    #[error("network target is unreachable")]
    Unreachable,
    /// Requested operation is unsupported by this transport.
    #[error("network operation is unsupported")]
    Unsupported,
    /// Underlying runtime or driver is unavailable.
    #[error("network runtime is unavailable")]
    RuntimeUnavailable,
    /// Transport-specific failure string preserved by adapter.
    #[error("{0}")]
    Other(String),
}

/// Async datagram socket used by UDP-style protocol adapters.
pub trait AsyncDatagramSocket: MaybeSend + MaybeSync + 'static {
    /// Receives one datagram into the provided buffer and returns byte count plus peer address.
    fn recv_from(
        &mut self,
        buf: &mut [u8],
    ) -> impl Future<Output = Result<(usize, String), NetError>> + MaybeSend;

    /// Sends one datagram to the provided peer address and returns byte count sent.
    fn send_to(
        &mut self,
        buf: &[u8],
        address: &str,
    ) -> impl Future<Output = Result<usize, NetError>> + MaybeSend;

    /// Sets or clears the receive timeout applied to subsequent `recv_from` calls.
    fn set_read_timeout(&mut self, timeout: Option<Duration>) -> Result<(), NetError>;

    /// Closes the datagram socket gracefully when the transport supports it.
    fn close(&mut self) -> impl Future<Output = Result<(), NetError>> + MaybeSend;
}

/// Async bidirectional byte stream used by protocol adapters.
pub trait AsyncSocket: MaybeSend + MaybeSync + 'static {
    /// Sets or clears the timeout applied to subsequent `read` calls.
    fn set_read_timeout(&mut self, timeout: Option<Duration>) -> Result<(), NetError>;

    /// Sets or clears the timeout applied to subsequent `write` and `flush` calls.
    fn set_write_timeout(&mut self, timeout: Option<Duration>) -> Result<(), NetError>;

    /// Reads bytes into the provided buffer and returns the number of bytes read.
    fn read(&mut self, buf: &mut [u8])
    -> impl Future<Output = Result<usize, NetError>> + MaybeSend;

    /// Writes bytes from the provided buffer and returns the number of bytes written.
    fn write(&mut self, buf: &[u8]) -> impl Future<Output = Result<usize, NetError>> + MaybeSend;

    /// Flushes buffered outbound bytes when the transport supports it.
    fn flush(&mut self) -> impl Future<Output = Result<(), NetError>> + MaybeSend;

    /// Closes the socket gracefully when the transport supports it.
    fn close(&mut self) -> impl Future<Output = Result<(), NetError>> + MaybeSend;
}

/// Brackets one bare IPv6 host literal for URI/socket-address use.
pub fn bracket_ipv6_host(host: &str) -> String {
    if host.starts_with('[') || host.matches(':').count() <= 1 {
        host.to_string()
    } else {
        format!("[{host}]")
    }
}

/// Formats one host plus one explicit port, bracketing bare IPv6 literals as needed.
pub fn format_host_port(host: &str, port: u16) -> String {
    format!("{}:{port}", bracket_ipv6_host(host))
}

/// Appends the default port when the authority omits it, bracketing bare IPv6 literals.
pub fn normalize_host_port(authority: &str, default_port: u16) -> String {
    if authority.starts_with('[') {
        if authority.contains("]:") {
            authority.to_string()
        } else {
            format_host_port(authority, default_port)
        }
    } else {
        match authority.matches(':').count() {
            0 => format_host_port(authority, default_port),
            1 => authority.to_string(),
            _ => format_host_port(authority, default_port),
        }
    }
}

/// Writes the full buffer to one async socket, retrying short writes until completion.
pub async fn write_all_socket<S>(socket: &mut S, buf: &[u8]) -> Result<(), NetError>
where
    S: AsyncSocket,
{
    let mut written = 0;
    while written < buf.len() {
        let count = socket.write(&buf[written..]).await?;
        if count == 0 {
            return Err(NetError::Closed);
        }
        written += count;
    }
    Ok(())
}

/// Async listener that accepts inbound sockets.
pub trait AsyncListener: MaybeSend + MaybeSync + 'static {
    /// Socket type produced by this listener.
    type Socket: AsyncSocket;

    /// Accepts one inbound connection from the listener.
    fn accept(&mut self) -> impl Future<Output = Result<Self::Socket, NetError>> + MaybeSend;

    /// Closes the listener and releases bound resources.
    fn close(&mut self) -> impl Future<Output = Result<(), NetError>> + MaybeSend;
}

/// Async network transport factory shared by protocol adapters.
///
/// Implementations are intended to bridge concrete ecosystems such as Tokio, async-std,
/// or embassy-net into a transport-neutral socket model consumed by protocol crates.
pub trait AsyncNet: Clone + MaybeSend + MaybeSync + 'static {
    /// Connected socket type returned by `connect`.
    type Socket: AsyncSocket;
    /// Listener type returned by `bind`.
    type Listener: AsyncListener<Socket = Self::Socket>;

    /// Establishes one outbound byte-stream connection to the given address.
    fn connect(
        &self,
        address: &str,
    ) -> impl Future<Output = Result<Self::Socket, NetError>> + MaybeSend;

    /// Binds one listener to the given local address when supported.
    fn bind(
        &self,
        address: &str,
    ) -> impl Future<Output = Result<Self::Listener, NetError>> + MaybeSend;
}

/// Async datagram network transport factory shared by UDP-style protocol adapters.
pub trait AsyncDatagramNet: Clone + MaybeSend + MaybeSync + 'static {
    /// Datagram socket type returned by `bind_datagram`.
    type DatagramSocket: AsyncDatagramSocket;

    /// Binds one datagram socket to the given local address.
    fn bind_datagram(
        &self,
        address: &str,
    ) -> impl Future<Output = Result<Self::DatagramSocket, NetError>> + MaybeSend;
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::{boxed::Box, string::ToString, vec, vec::Vec};
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
        fn set_read_timeout(&mut self, _timeout: Option<Duration>) -> Result<(), NetError> {
            Ok(())
        }

        fn set_write_timeout(&mut self, _timeout: Option<Duration>) -> Result<(), NetError> {
            Ok(())
        }

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

    #[derive(Default)]
    struct MockPartialWriteSocket {
        write_plan: Vec<usize>,
        written: Vec<u8>,
    }

    impl AsyncSocket for MockPartialWriteSocket {
        fn set_read_timeout(&mut self, _timeout: Option<Duration>) -> Result<(), NetError> {
            Ok(())
        }

        fn set_write_timeout(&mut self, _timeout: Option<Duration>) -> Result<(), NetError> {
            Ok(())
        }

        async fn read(&mut self, _buf: &mut [u8]) -> Result<usize, NetError> {
            Err(NetError::Closed)
        }

        async fn write(&mut self, buf: &[u8]) -> Result<usize, NetError> {
            let count = self
                .write_plan
                .first()
                .copied()
                .unwrap_or(buf.len())
                .min(buf.len());
            if !self.write_plan.is_empty() {
                self.write_plan.remove(0);
            }
            self.written.extend_from_slice(&buf[..count]);
            Ok(count)
        }

        async fn flush(&mut self) -> Result<(), NetError> {
            Ok(())
        }

        async fn close(&mut self) -> Result<(), NetError> {
            Ok(())
        }
    }

    struct MockListener {
        accepted: bool,
    }

    struct MockDatagramSocket {
        recv_data: Option<&'static [u8]>,
        sent_packets: usize,
        closed: bool,
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

    impl AsyncDatagramSocket for MockDatagramSocket {
        fn set_read_timeout(&mut self, _timeout: Option<Duration>) -> Result<(), NetError> {
            Ok(())
        }

        async fn recv_from(&mut self, buf: &mut [u8]) -> Result<(usize, String), NetError> {
            let Some(recv_data) = self.recv_data.take() else {
                return Err(NetError::Closed);
            };
            let len = recv_data.len().min(buf.len());
            buf[..len].copy_from_slice(&recv_data[..len]);
            Ok((len, "127.0.0.1:502".to_string()))
        }

        async fn send_to(&mut self, buf: &[u8], _address: &str) -> Result<usize, NetError> {
            self.sent_packets += 1;
            Ok(buf.len())
        }

        async fn close(&mut self) -> Result<(), NetError> {
            self.closed = true;
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

    impl AsyncDatagramNet for MockNet {
        type DatagramSocket = MockDatagramSocket;

        async fn bind_datagram(&self, _address: &str) -> Result<Self::DatagramSocket, NetError> {
            Ok(MockDatagramSocket {
                recv_data: Some(b"udp"),
                sent_packets: 0,
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

    #[test]
    fn mock_async_datagram_net_contract_is_usable() {
        let net = MockNet;
        let mut socket =
            block_on(net.bind_datagram("127.0.0.1:0")).expect("bind_datagram should succeed");
        let mut buf = [0u8; 8];
        let (n, peer) = block_on(socket.recv_from(&mut buf)).expect("recv_from should succeed");
        assert_eq!(&buf[..n], b"udp");
        assert_eq!(peer, "127.0.0.1:502");
        assert_eq!(block_on(socket.send_to(b"abc", "127.0.0.1:502")), Ok(3));
        assert_eq!(block_on(socket.close()), Ok(()));
    }

    #[test]
    fn write_all_socket_retries_short_writes() {
        let mut socket = MockPartialWriteSocket {
            write_plan: vec![2, 3, 8],
            ..Default::default()
        };

        block_on(write_all_socket(&mut socket, b"abcdefgh")).expect("write_all should succeed");

        assert_eq!(socket.written, b"abcdefgh");
    }

    #[test]
    fn host_port_helpers_bracket_bare_ipv6_and_preserve_existing_ports() {
        assert_eq!(bracket_ipv6_host("2001:db8::10"), "[2001:db8::10]");
        assert_eq!(bracket_ipv6_host("[2001:db8::10]"), "[2001:db8::10]");
        assert_eq!(bracket_ipv6_host("example.com"), "example.com");

        assert_eq!(format_host_port("2001:db8::10", 502), "[2001:db8::10]:502");
        assert_eq!(
            format_host_port("[2001:db8::10]", 502),
            "[2001:db8::10]:502"
        );
        assert_eq!(format_host_port("127.0.0.1", 502), "127.0.0.1:502");

        assert_eq!(
            normalize_host_port("mqtt://ignored", 1883),
            "mqtt://ignored"
        );
        assert_eq!(normalize_host_port("example.com", 1883), "example.com:1883");
        assert_eq!(
            normalize_host_port("example.com:1884", 1883),
            "example.com:1884"
        );
        assert_eq!(normalize_host_port("[::1]", 1883), "[::1]:1883");
        assert_eq!(normalize_host_port("[::1]:1884", 1883), "[::1]:1884");
        assert_eq!(
            normalize_host_port("2001:db8::10", 1883),
            "[2001:db8::10]:1883"
        );
    }
}
