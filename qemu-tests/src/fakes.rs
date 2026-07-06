//! In-memory test doubles for the self-contained suites: a loopback serial device and
//! a two-ended ethernet "wire" joining a pair of embassy-net stacks — the same shapes
//! as the host test suite, with no host services involved.

use alloc::{boxed::Box, collections::VecDeque, rc::Rc, vec, vec::Vec};
use core::{
    cell::RefCell,
    task::{Context, Waker},
};

use embassy_net::{Config, Ipv4Cidr, StackResources, StaticConfigV4};
use embassy_net_driver::{Capabilities, Driver, HardwareAddress, LinkState, RxToken, TxToken};
use ferredge_core::prelude::AsyncRuntime;
use ferredge_runtime_embassy::{EmbassyNet, EmbassyNetConfig, EmbassyRuntime};

/// Echoes writes back into its own read buffer.
#[derive(Default)]
pub struct LoopbackSerial {
    buffer: VecDeque<u8>,
}

impl embedded_io_async::ErrorType for LoopbackSerial {
    type Error = core::convert::Infallible;
}

impl embedded_io_async::Read for LoopbackSerial {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        let len = buf.len().min(self.buffer.len());
        for slot in &mut buf[..len] {
            *slot = self.buffer.pop_front().expect("length checked");
        }
        Ok(len)
    }
}

impl embedded_io_async::Write for LoopbackSerial {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        self.buffer.extend(buf);
        Ok(buf.len())
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

/// One direction of an in-memory ethernet link: a frame queue plus the waker of the
/// stack waiting on it.
#[derive(Default)]
pub struct Wire {
    frames: VecDeque<Vec<u8>>,
    waker: Option<Waker>,
}

pub type SharedWire = Rc<RefCell<Wire>>;

/// `embassy_net_driver::Driver` over a pair of [`SharedWire`]s.
pub struct TestDriver {
    pub rx: SharedWire,
    pub tx: SharedWire,
    pub mac: [u8; 6],
}

pub struct TestRxToken {
    frame: Vec<u8>,
}

pub struct TestTxToken {
    wire: SharedWire,
}

impl RxToken for TestRxToken {
    fn consume<R, F>(mut self, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        f(&mut self.frame)
    }
}

impl TxToken for TestTxToken {
    fn consume<R, F>(self, len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let mut frame = vec![0u8; len];
        let result = f(&mut frame);
        let mut wire = self.wire.borrow_mut();
        wire.frames.push_back(frame);
        if let Some(waker) = wire.waker.take() {
            waker.wake();
        }
        result
    }
}

impl Driver for TestDriver {
    type RxToken<'a> = TestRxToken;
    type TxToken<'a> = TestTxToken;

    fn receive(&mut self, cx: &mut Context) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        let mut rx = self.rx.borrow_mut();
        match rx.frames.pop_front() {
            Some(frame) => Some((
                TestRxToken { frame },
                TestTxToken {
                    wire: self.tx.clone(),
                },
            )),
            None => {
                rx.waker = Some(cx.waker().clone());
                None
            }
        }
    }

    fn transmit(&mut self, _cx: &mut Context) -> Option<Self::TxToken<'_>> {
        Some(TestTxToken {
            wire: self.tx.clone(),
        })
    }

    fn link_state(&mut self, _cx: &mut Context) -> LinkState {
        LinkState::Up
    }

    fn capabilities(&self) -> Capabilities {
        let mut capabilities = Capabilities::default();
        capabilities.max_transmission_unit = 1514;
        capabilities
    }

    fn hardware_address(&self) -> HardwareAddress {
        HardwareAddress::Ethernet(self.mac)
    }
}

/// Spawns an embassy-net stack over `driver` with a static address and wraps it in the
/// ferredge adapter.
pub fn make_net(
    runtime: &EmbassyRuntime,
    driver: TestDriver,
    address: Ipv4Cidr,
    seed: u64,
) -> EmbassyNet {
    let resources: &'static mut StackResources<8> = Box::leak(Box::new(StackResources::new()));
    let config = Config::ipv4_static(StaticConfigV4 {
        address,
        gateway: None,
        dns_servers: heapless::Vec::new(),
    });
    let (stack, mut runner) = embassy_net::new(driver, config, resources, seed);
    runtime.spawn(async move { runner.run().await });
    EmbassyNet::new(stack, EmbassyNetConfig::default())
}
