//! Exercises the `AsyncNet`/`AsyncDatagramNet` surface: TCP echo and UDP datagrams
//! between two embassy-net stacks joined by the in-memory wire.

use alloc::rc::Rc;
use core::cell::RefCell;

use embassy_executor::Spawner;
use embassy_net::{Ipv4Address, Ipv4Cidr};
use ferredge_core::prelude::*;
use ferredge_runtime_embassy::EmbassyRuntime;

use crate::fakes::{SharedWire, TestDriver, Wire, make_net};

pub async fn run(spawner: Spawner, runtime: &EmbassyRuntime) {
    let a_to_b: SharedWire = Rc::new(RefCell::new(Wire::default()));
    let b_to_a: SharedWire = Rc::new(RefCell::new(Wire::default()));
    let net_a = make_net(
        spawner,
        TestDriver {
            rx: b_to_a.clone(),
            tx: a_to_b.clone(),
            mac: [0x02, 0, 0, 0, 0, 1],
        },
        Ipv4Cidr::new(Ipv4Address::new(10, 0, 7, 1), 24),
        0x0123_4567_89ab_cdef,
    );
    let net_b = make_net(
        spawner,
        TestDriver {
            rx: a_to_b,
            tx: b_to_a,
            mac: [0x02, 0, 0, 0, 0, 2],
        },
        Ipv4Cidr::new(Ipv4Address::new(10, 0, 7, 2), 24),
        0xfedc_ba98_7654_3210,
    );

    // TCP echo
    log::debug!("testing TCP echo 10.0.7.1 -> 10.0.7.2:4000");
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
        .connect("10.0.7.2:4000")
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
    log::debug!("testing UDP datagram :5001 -> :5000");
    let mut receiver = net_b
        .bind_datagram("0.0.0.0:5000")
        .await
        .expect("receiver bind should succeed");
    let mut sender = net_a
        .bind_datagram("0.0.0.0:5001")
        .await
        .expect("sender bind should succeed");
    sender
        .send_to(b"udp", "10.0.7.2:5000")
        .await
        .expect("send_to should succeed");
    let mut dgram = [0u8; 16];
    let (n, peer) = receiver
        .recv_from(&mut dgram)
        .await
        .expect("recv_from should succeed");
    assert_eq!(&dgram[..n], b"udp");
    assert_eq!(peer, "10.0.7.1:5001");
}
