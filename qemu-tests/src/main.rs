//! Executes the ferredge embassy adapters on a bare-metal Cortex-M3 under QEMU.
//!
//! Time is driven by the embassy `MockDriver`, advanced by a dedicated always-ready task,
//! so timers fire deterministically without a hardware timer. Networking runs over an
//! in-memory ethernet wire between two embassy-net stacks, and serial over an in-memory
//! loopback `embedded-io-async` device — the same shapes as the host test suite.
//!
//! The binary exits QEMU via semihosting: exit code 0 when every check passed, and a
//! panic (assert failure, allocation error) exits non-zero via `panic-semihosting`.

#![no_std]
#![no_main]

extern crate alloc;
extern crate panic_semihosting;

use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::rc::Rc;
use alloc::string::ToString;
use alloc::vec::Vec;
use core::cell::RefCell;
use core::mem::MaybeUninit;
use core::task::{Context, Waker};
use core::time::Duration;

use cortex_m_rt::entry;
use cortex_m_semihosting::{debug, hprintln};
use embassy_executor::Executor;
use embassy_net::{Config, Ipv4Address, Ipv4Cidr, StackResources, StaticConfigV4};
use embassy_net_driver::{Capabilities, Driver, HardwareAddress, LinkState, RxToken, TxToken};
use embassy_time::MockDriver;
use embedded_alloc::LlffHeap as Heap;
use ferredge_core::prelude::*;
use ferredge_runtime_embassy::{EmbassyNet, EmbassyNetConfig, EmbassyRuntime, EmbassySerial};

#[global_allocator]
static HEAP: Heap = Heap::empty();

const HEAP_SIZE: usize = 1024 * 1024;

#[entry]
fn main() -> ! {
    {
        static mut HEAP_MEM: [MaybeUninit<u8>; HEAP_SIZE] = [MaybeUninit::uninit(); HEAP_SIZE];
        unsafe { HEAP.init(core::ptr::addr_of_mut!(HEAP_MEM) as usize, HEAP_SIZE) }
    }

    let executor: &'static mut Executor = Box::leak(Box::new(Executor::new()));
    executor.run(|spawner| {
        let runtime: EmbassyRuntime = EmbassyRuntime::new(spawner);

        // Drive the mock clock forward whenever the executor has nothing else to do, so
        // sleeps and timeouts fire without a hardware timer.
        runtime.spawn(async {
            loop {
                MockDriver::get().advance(embassy_time::Duration::from_millis(1));
                embassy_futures::yield_now().await;
            }
        });

        let tests_runtime = runtime.clone();
        runtime.spawn(async move {
            runtime_tests(&tests_runtime).await;
            hprintln!("runtime tests passed");
            serial_tests().await;
            hprintln!("serial tests passed");
            net_tests(&tests_runtime).await;
            hprintln!("net tests passed");
            hprintln!("all embedded embassy tests passed");
            debug::exit(debug::EXIT_SUCCESS);
        });
    })
}

async fn runtime_tests(runtime: &EmbassyRuntime) {
    // spawn/join, repeated to exercise task-storage reuse
    for round in 0..3u8 {
        let mut task = runtime.spawn(async move { round });
        assert_eq!(task.join().await, Ok(round));
        assert!(task.is_finished());
    }

    // abort
    let sleeper = runtime.clone();
    let mut task = runtime.spawn(async move {
        sleeper.sleep(Duration::from_secs(3600)).await;
    });
    task.abort();
    assert_eq!(task.join().await, Err(TaskJoinError::Cancelled));

    // channel (compile-time capacity is 16 for the default runtime)
    let (tx, mut rx) = runtime.channel::<u8>(16);
    tx.send(1).await.expect("send should succeed");
    tx.try_send(2).expect("try_send should succeed");
    assert_eq!(rx.recv().await, Ok(1));
    assert_eq!(rx.try_recv(), Ok(2));
    assert_eq!(rx.try_recv(), Err(ChannelError::Empty));

    // mutex
    let mutex = runtime.mutex(11u8);
    {
        let mut guard = mutex.lock().await.expect("lock should succeed");
        *guard = 13;
        assert!(matches!(mutex.try_lock(), Err(MutexError::Busy)));
    }
    assert_eq!(*mutex.try_lock().expect("try_lock should succeed"), 13);

    // sleep + instant against the mock clock
    let started = runtime.now();
    runtime.sleep(Duration::from_millis(50)).await;
    assert!(started.elapsed() >= Duration::from_millis(50));
}

#[derive(Default)]
struct LoopbackSerial {
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

async fn serial_tests() {
    let serial = EmbassySerial::new();
    serial.register("/dev/uart0", LoopbackSerial::default());
    let config = SerialPortConfig {
        path: "/dev/uart0".to_string(),
        ..SerialPortConfig::default()
    };

    let mut port = serial.open(&config).await.expect("open should succeed");
    assert_eq!(port.write(b"rtu").await, Ok(3));
    let mut buf = [0u8; 8];
    assert_eq!(port.read(&mut buf).await, Ok(3));
    assert_eq!(&buf[..3], b"rtu");
    assert!(serial.open(&config).await.is_err());
}

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
        let mut frame = alloc::vec![0u8; len];
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

async fn net_tests(runtime: &EmbassyRuntime) {
    let a_to_b: SharedWire = Rc::new(RefCell::new(Wire::default()));
    let b_to_a: SharedWire = Rc::new(RefCell::new(Wire::default()));
    let net_a = make_net(
        runtime,
        TestDriver {
            rx: b_to_a.clone(),
            tx: a_to_b.clone(),
            mac: [0x02, 0, 0, 0, 0, 1],
        },
        Ipv4Cidr::new(Ipv4Address::new(192, 168, 7, 1), 24),
        0x0123_4567_89ab_cdef,
    );
    let net_b = make_net(
        runtime,
        TestDriver {
            rx: a_to_b,
            tx: b_to_a,
            mac: [0x02, 0, 0, 0, 0, 2],
        },
        Ipv4Cidr::new(Ipv4Address::new(192, 168, 7, 2), 24),
        0xfedc_ba98_7654_3210,
    );

    // TCP echo
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

    // UDP datagrams
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
    let mut dgram = [0u8; 16];
    let (n, peer) = receiver
        .recv_from(&mut dgram)
        .await
        .expect("recv_from should succeed");
    assert_eq!(&dgram[..n], b"udp");
    assert_eq!(peer, "192.168.7.1:5001");
}
