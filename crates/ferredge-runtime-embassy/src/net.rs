use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::vec;
use core::mem::ManuallyDrop;
use core::net::SocketAddr;
use core::str::FromStr;
use core::time::Duration;

use embassy_net::tcp::{self, TcpSocket};
use embassy_net::udp::{self as udp, PacketMetadata, UdpMetadata, UdpSocket};
use embassy_net::{IpEndpoint, IpListenEndpoint, Stack};
use ferredge_core::prelude::*;

use crate::time_util::to_embassy_duration;

/// Buffer sizing for sockets created by [`EmbassyNet`].
#[derive(Clone, Copy)]
pub struct EmbassyNetConfig {
    pub tcp_rx_buffer_size: usize,
    pub tcp_tx_buffer_size: usize,
    pub udp_rx_buffer_size: usize,
    pub udp_tx_buffer_size: usize,
    pub udp_rx_metadata_slots: usize,
    pub udp_tx_metadata_slots: usize,
}

impl Default for EmbassyNetConfig {
    fn default() -> Self {
        Self {
            tcp_rx_buffer_size: 4096,
            tcp_tx_buffer_size: 4096,
            udp_rx_buffer_size: 2048,
            udp_tx_buffer_size: 2048,
            udp_rx_metadata_slots: 8,
            udp_tx_metadata_slots: 8,
        }
    }
}

/// [`AsyncNet`]/[`AsyncDatagramNet`] adapter over an embassy-net [`Stack`].
///
/// The stack (and its runner task) must be set up by the application, e.g. with
/// `StaticCell`, exactly as in any embassy-net firmware. Addresses must be numeric
/// `ip:port` strings unless the `dns` feature is enabled, which resolves hostnames
/// through the stack's DNS socket.
#[derive(Clone)]
pub struct EmbassyNet {
    stack: Stack<'static>,
    config: EmbassyNetConfig,
}

/// Heap buffer leaked to `&'static mut` for an embassy-net socket, reclaimed manually
/// once the socket that borrows it is dropped.
struct LeakedSlice<T> {
    pointer: *mut T,
    len: usize,
}

impl<T> LeakedSlice<T> {
    fn from_vec(values: alloc::vec::Vec<T>) -> (Self, &'static mut [T]) {
        let slice = Box::leak(values.into_boxed_slice());
        let raw = Self {
            pointer: slice.as_mut_ptr(),
            len: slice.len(),
        };
        (raw, slice)
    }

    /// # Safety
    ///
    /// Callers must guarantee the socket borrowing the slice has been dropped and that
    /// this is called at most once.
    unsafe fn reclaim(&mut self) {
        unsafe {
            drop(Box::from_raw(core::ptr::slice_from_raw_parts_mut(
                self.pointer,
                self.len,
            )));
        }
    }
}

pub struct EmbassySocket {
    socket: ManuallyDrop<TcpSocket<'static>>,
    rx_buffer: LeakedSlice<u8>,
    tx_buffer: LeakedSlice<u8>,
    read_timeout: Option<Duration>,
    write_timeout: Option<Duration>,
}

impl Drop for EmbassySocket {
    fn drop(&mut self) {
        // SAFETY: the socket borrows the leaked buffers, so it is dropped first (removing
        // it from the stack's socket set), then the buffers are reclaimed exactly once.
        unsafe {
            ManuallyDrop::drop(&mut self.socket);
            self.rx_buffer.reclaim();
            self.tx_buffer.reclaim();
        }
    }
}

/// Accept-side handle returned by [`EmbassyNet::bind`].
///
/// embassy-net has no listener object with a backlog; each `accept` call opens one
/// listening socket on the endpoint and waits for one inbound connection.
pub struct EmbassyListener {
    net: EmbassyNet,
    endpoint: IpListenEndpoint,
}

pub struct EmbassyDatagramSocket {
    socket: ManuallyDrop<UdpSocket<'static>>,
    rx_metadata: LeakedSlice<PacketMetadata>,
    rx_buffer: LeakedSlice<u8>,
    tx_metadata: LeakedSlice<PacketMetadata>,
    tx_buffer: LeakedSlice<u8>,
    read_timeout: Option<Duration>,
}

impl Drop for EmbassyDatagramSocket {
    fn drop(&mut self) {
        // SAFETY: same ordering contract as `EmbassySocket::drop`.
        unsafe {
            ManuallyDrop::drop(&mut self.socket);
            self.rx_metadata.reclaim();
            self.rx_buffer.reclaim();
            self.tx_metadata.reclaim();
            self.tx_buffer.reclaim();
        }
    }
}

impl EmbassyNet {
    pub fn new(stack: Stack<'static>, config: EmbassyNetConfig) -> Self {
        Self { stack, config }
    }

    fn make_tcp_socket(&self) -> EmbassySocket {
        let (rx_raw, rx) = LeakedSlice::from_vec(vec![0u8; self.config.tcp_rx_buffer_size]);
        let (tx_raw, tx) = LeakedSlice::from_vec(vec![0u8; self.config.tcp_tx_buffer_size]);
        EmbassySocket {
            socket: ManuallyDrop::new(TcpSocket::new(self.stack, rx, tx)),
            rx_buffer: rx_raw,
            tx_buffer: tx_raw,
            read_timeout: None,
            write_timeout: None,
        }
    }

    async fn resolve(&self, address: &str) -> Result<IpEndpoint, NetError> {
        if let Ok(address) = SocketAddr::from_str(address) {
            return Ok(address.into());
        }
        self.resolve_host(address).await
    }

    #[cfg(feature = "dns")]
    async fn resolve_host(&self, address: &str) -> Result<IpEndpoint, NetError> {
        use embassy_net::dns::DnsQueryType;

        let (host, port) = split_host_port(address)?;
        for query_type in [DnsQueryType::A, DnsQueryType::Aaaa] {
            if let Ok(addresses) = self.stack.dns_query(host, query_type).await
                && let Some(ip) = addresses.first()
            {
                return Ok(IpEndpoint::new(*ip, port));
            }
        }
        Err(NetError::Unreachable)
    }

    #[cfg(not(feature = "dns"))]
    async fn resolve_host(&self, _address: &str) -> Result<IpEndpoint, NetError> {
        Err(NetError::Unreachable)
    }
}

#[cfg(feature = "dns")]
fn split_host_port(address: &str) -> Result<(&str, u16), NetError> {
    let (host, port) = address.rsplit_once(':').ok_or(NetError::Unreachable)?;
    let port = port.parse().map_err(|_| NetError::Unreachable)?;
    Ok((host.trim_start_matches('[').trim_end_matches(']'), port))
}

impl EmbassySocket {
    pub fn set_read_timeout(&mut self, timeout: Option<Duration>) -> Result<(), NetError> {
        self.read_timeout = timeout;
        Ok(())
    }

    pub fn set_write_timeout(&mut self, timeout: Option<Duration>) -> Result<(), NetError> {
        self.write_timeout = timeout;
        Ok(())
    }
}

async fn with_optional_timeout<F: core::future::Future>(
    timeout: Option<Duration>,
    future: F,
) -> Result<F::Output, NetError> {
    match timeout {
        Some(timeout) => embassy_time::with_timeout(to_embassy_duration(timeout), future)
            .await
            .map_err(|_| NetError::TimedOut),
        None => Ok(future.await),
    }
}

impl AsyncSocket for EmbassySocket {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, NetError> {
        with_optional_timeout(self.read_timeout, self.socket.read(buf))
            .await?
            .map_err(map_tcp_error)
    }

    async fn write(&mut self, buf: &[u8]) -> Result<usize, NetError> {
        with_optional_timeout(self.write_timeout, self.socket.write(buf))
            .await?
            .map_err(map_tcp_error)
    }

    async fn flush(&mut self) -> Result<(), NetError> {
        with_optional_timeout(self.write_timeout, self.socket.flush())
            .await?
            .map_err(map_tcp_error)
    }

    async fn close(&mut self) -> Result<(), NetError> {
        self.socket.close();
        // Drive the FIN handshake out before reporting the socket closed.
        self.socket.flush().await.map_err(map_tcp_error)
    }
}

impl AsyncListener for EmbassyListener {
    type Socket = EmbassySocket;

    async fn accept(&mut self) -> Result<Self::Socket, NetError> {
        let mut socket = self.net.make_tcp_socket();
        socket
            .socket
            .accept(self.endpoint)
            .await
            .map_err(map_accept_error)?;
        Ok(socket)
    }

    async fn close(&mut self) -> Result<(), NetError> {
        Ok(())
    }
}

impl AsyncNet for EmbassyNet {
    type Socket = EmbassySocket;
    type Listener = EmbassyListener;

    async fn connect(&self, address: &str) -> Result<Self::Socket, NetError> {
        let endpoint = self.resolve(address).await?;
        let mut socket = self.make_tcp_socket();
        socket
            .socket
            .connect(endpoint)
            .await
            .map_err(map_connect_error)?;
        Ok(socket)
    }

    async fn bind(&self, address: &str) -> Result<Self::Listener, NetError> {
        let endpoint = parse_listen_endpoint(address)?;
        Ok(EmbassyListener {
            net: self.clone(),
            endpoint,
        })
    }
}

impl EmbassyDatagramSocket {
    pub fn set_read_timeout(&mut self, timeout: Option<Duration>) -> Result<(), NetError> {
        self.read_timeout = timeout;
        Ok(())
    }
}

impl AsyncDatagramSocket for EmbassyDatagramSocket {
    async fn recv_from(&mut self, buf: &mut [u8]) -> Result<(usize, String), NetError> {
        let (size, metadata) = with_optional_timeout(self.read_timeout, self.socket.recv_from(buf))
            .await?
            .map_err(map_udp_recv_error)?;
        Ok((size, format_endpoint(metadata)))
    }

    async fn send_to(&mut self, buf: &[u8], address: &str) -> Result<usize, NetError> {
        let endpoint = SocketAddr::from_str(address)
            .map(IpEndpoint::from)
            .map_err(|_| NetError::Unreachable)?;
        self.socket
            .send_to(buf, endpoint)
            .await
            .map_err(map_udp_send_error)?;
        Ok(buf.len())
    }

    async fn close(&mut self) -> Result<(), NetError> {
        self.socket.close();
        Ok(())
    }
}

impl AsyncDatagramNet for EmbassyNet {
    type DatagramSocket = EmbassyDatagramSocket;

    async fn bind_datagram(&self, address: &str) -> Result<Self::DatagramSocket, NetError> {
        let endpoint = parse_listen_endpoint(address)?;
        let (rx_meta_raw, rx_meta) = LeakedSlice::from_vec(vec![
            PacketMetadata::EMPTY;
            self.config.udp_rx_metadata_slots
        ]);
        let (rx_raw, rx) = LeakedSlice::from_vec(vec![0u8; self.config.udp_rx_buffer_size]);
        let (tx_meta_raw, tx_meta) = LeakedSlice::from_vec(vec![
            PacketMetadata::EMPTY;
            self.config.udp_tx_metadata_slots
        ]);
        let (tx_raw, tx) = LeakedSlice::from_vec(vec![0u8; self.config.udp_tx_buffer_size]);
        let mut socket = EmbassyDatagramSocket {
            socket: ManuallyDrop::new(UdpSocket::new(self.stack, rx_meta, rx, tx_meta, tx)),
            rx_metadata: rx_meta_raw,
            rx_buffer: rx_raw,
            tx_metadata: tx_meta_raw,
            tx_buffer: tx_raw,
            read_timeout: None,
        };
        socket.socket.bind(endpoint).map_err(map_udp_bind_error)?;
        Ok(socket)
    }
}

fn parse_listen_endpoint(address: &str) -> Result<IpListenEndpoint, NetError> {
    let address = SocketAddr::from_str(address).map_err(|_| NetError::Unreachable)?;
    let endpoint = IpEndpoint::from(address);
    Ok(IpListenEndpoint {
        addr: (!endpoint.addr.is_unspecified()).then_some(endpoint.addr),
        port: endpoint.port,
    })
}

fn format_endpoint(metadata: UdpMetadata) -> String {
    format_host_port(&metadata.endpoint.addr.to_string(), metadata.endpoint.port)
}

fn map_tcp_error(error: tcp::Error) -> NetError {
    match error {
        tcp::Error::ConnectionReset => NetError::Closed,
    }
}

fn map_connect_error(error: tcp::ConnectError) -> NetError {
    match error {
        tcp::ConnectError::ConnectionReset | tcp::ConnectError::NoRoute => NetError::Unreachable,
        tcp::ConnectError::TimedOut => NetError::TimedOut,
        tcp::ConnectError::InvalidState => NetError::Other("tcp connect invalid state".into()),
    }
}

fn map_accept_error(error: tcp::AcceptError) -> NetError {
    match error {
        tcp::AcceptError::ConnectionReset => NetError::Closed,
        tcp::AcceptError::InvalidPort => NetError::Unsupported,
        tcp::AcceptError::InvalidState => NetError::Other("tcp accept invalid state".into()),
    }
}

fn map_udp_bind_error(error: udp::BindError) -> NetError {
    match error {
        udp::BindError::NoRoute => NetError::Unreachable,
        udp::BindError::InvalidState => NetError::Other("udp bind invalid state".into()),
    }
}

fn map_udp_send_error(error: udp::SendError) -> NetError {
    match error {
        udp::SendError::NoRoute => NetError::Unreachable,
        udp::SendError::SocketNotBound => NetError::Closed,
        udp::SendError::PacketTooLarge => NetError::Unsupported,
    }
}

fn map_udp_recv_error(error: udp::RecvError) -> NetError {
    match error {
        udp::RecvError::Truncated => NetError::Other("udp datagram truncated".into()),
    }
}
