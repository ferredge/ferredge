//! End-to-end exercise of the embassy net adapter: two embassy-net stacks joined by an
//! in-memory ethernet "wire", talking TCP and UDP through the ferredge `AsyncNet` traits.

#![cfg(all(feature = "net", feature = "runtime", feature = "std"))]

use core::future::Future;
use core::task::{Context, Waker};
use core::time::Duration;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use embassy_executor::Executor;
use embassy_net::{Config, Ipv4Address, Ipv4Cidr, StackResources, StaticConfigV4};
use embassy_net_driver::{Capabilities, Driver, HardwareAddress, LinkState, RxToken, TxToken};
use ferredge_core::prelude::*;
use ferredge_runtime_embassy::{EmbassyNet, EmbassyNetConfig, EmbassyRuntime};

#[derive(Default)]
struct Wire {
    frames: VecDeque<Vec<u8>>,
    waker: Option<Waker>,
}

type SharedWire = Rc<RefCell<Wire>>;

struct TestDriver {
    rx: SharedWire,
    tx: SharedWire,
    mac: [u8; 6],
}

struct TestRxToken {
    frame: Vec<u8>,
}

struct TestTxToken {
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

fn make_net(
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

fn run_net_test<F>(make: impl FnOnce(EmbassyRuntime, EmbassyNet, EmbassyNet) -> F + Send + 'static)
where
    F: Future<Output = ()> + 'static,
{
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let executor: &'static mut Executor = Box::leak(Box::new(Executor::new()));
        executor.run(move |spawner| {
            let runtime: EmbassyRuntime = EmbassyRuntime::new(spawner);
            let a_to_b: SharedWire = Rc::new(RefCell::new(Wire::default()));
            let b_to_a: SharedWire = Rc::new(RefCell::new(Wire::default()));
            let driver_a = TestDriver {
                rx: b_to_a.clone(),
                tx: a_to_b.clone(),
                mac: [0x02, 0, 0, 0, 0, 1],
            };
            let driver_b = TestDriver {
                rx: a_to_b,
                tx: b_to_a,
                mac: [0x02, 0, 0, 0, 0, 2],
            };
            let net_a = make_net(
                &runtime,
                driver_a,
                Ipv4Cidr::new(Ipv4Address::new(192, 168, 7, 1), 24),
                0x0123_4567_89ab_cdef,
            );
            let net_b = make_net(
                &runtime,
                driver_b,
                Ipv4Cidr::new(Ipv4Address::new(192, 168, 7, 2), 24),
                0xfedc_ba98_7654_3210,
            );
            let test_future = make(runtime.clone(), net_a, net_b);
            runtime.spawn(async move {
                test_future.await;
                done_tx
                    .send(())
                    .expect("test main thread should be waiting");
            });
        });
    });
    done_rx
        .recv_timeout(std::time::Duration::from_secs(30))
        .expect("embassy net test should complete");
}

#[test]
fn tcp_echo_between_two_stacks() {
    run_net_test(|runtime, net_a, net_b| async move {
        let mut listener = net_b
            .bind("0.0.0.0:4000")
            .await
            .expect("bind should succeed");
        let mut accept_task = runtime.spawn(async move {
            let mut server = listener.accept().await.expect("accept should succeed");
            let mut buf = [0u8; 16];
            let n = server
                .read(&mut buf)
                .await
                .expect("server read should succeed");
            write_all_socket(&mut server, &buf[..n])
                .await
                .expect("server write should succeed");
            server.flush().await.expect("server flush should succeed");
            // Wait for the client to close before dropping the socket.
            let _ = server.read(&mut buf).await;
        });

        let mut client = net_a
            .connect("192.168.7.2:4000")
            .await
            .expect("connect should succeed");
        write_all_socket(&mut client, b"ping")
            .await
            .expect("client write should succeed");
        client.flush().await.expect("client flush should succeed");
        let mut buf = [0u8; 16];
        let n = client
            .read(&mut buf)
            .await
            .expect("client read should succeed");
        assert_eq!(&buf[..n], b"ping");
        client.close().await.expect("client close should succeed");
        drop(client);

        accept_task.join().await.expect("accept task should finish");
    });
}

#[test]
fn tcp_read_timeout_fires() {
    run_net_test(|runtime, net_a, net_b| async move {
        let mut listener = net_b
            .bind("0.0.0.0:4001")
            .await
            .expect("bind should succeed");
        let mut accept_task = runtime.spawn(async move {
            let mut server = listener.accept().await.expect("accept should succeed");
            // Hold the connection open without sending anything.
            let mut buf = [0u8; 4];
            let _ = server.read(&mut buf).await;
        });

        let mut client = net_a
            .connect("192.168.7.2:4001")
            .await
            .expect("connect should succeed");
        client
            .set_read_timeout(Some(Duration::from_millis(50)))
            .expect("timeout should be configurable");
        let mut buf = [0u8; 4];
        assert_eq!(client.read(&mut buf).await, Err(NetError::TimedOut));

        // Close gracefully so the server's pending read observes the FIN and the task ends.
        client.close().await.expect("client close should succeed");
        drop(client);
        accept_task.join().await.expect("accept task should finish");
    });
}

#[test]
fn udp_datagrams_between_two_stacks() {
    run_net_test(|_runtime, net_a, net_b| async move {
        let mut receiver = net_b
            .bind_datagram("0.0.0.0:5000")
            .await
            .expect("receiver bind should succeed");
        let mut sender = net_a
            .bind_datagram("0.0.0.0:5001")
            .await
            .expect("sender bind should succeed");

        sender
            .send_to(b"udp", "192.168.7.2:5000")
            .await
            .expect("send_to should succeed");
        let mut buf = [0u8; 16];
        let (n, peer) = receiver
            .recv_from(&mut buf)
            .await
            .expect("recv_from should succeed");
        assert_eq!(&buf[..n], b"udp");
        assert_eq!(peer, "192.168.7.1:5001");

        receiver.close().await.expect("close should succeed");
    });
}
