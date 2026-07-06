//! Runs the full MQTT driver on bare metal: construction with an explicit stack, a
//! real connect attempt over embassy-net (refused — nothing listens on the peer
//! stack), and the async listener-state APIs.

use alloc::{rc::Rc, string::ToString, vec, vec::Vec};
use core::cell::RefCell;

use embassy_net::{Ipv4Address, Ipv4Cidr};
use ferredge_core::prelude::*;
use ferredge_proto_mqtt::{MqttDriver, MqttListenerStatus};
use ferredge_runtime_embassy::EmbassyRuntime;

use crate::fakes::{SharedWire, TestDriver, Wire, make_net};

pub async fn run(runtime: &EmbassyRuntime) {
    let a_to_b: SharedWire = Rc::new(RefCell::new(Wire::default()));
    let b_to_a: SharedWire = Rc::new(RefCell::new(Wire::default()));
    let net = make_net(
        runtime,
        TestDriver {
            rx: b_to_a.clone(),
            tx: a_to_b.clone(),
            mac: [0x02, 0, 0, 0, 1, 1],
        },
        Ipv4Cidr::new(Ipv4Address::new(10, 0, 9, 1), 24),
        0x00c0_ffee_c0ff_ee00,
    );
    // Peer stack participates in ARP and refuses the TCP connection.
    let _peer = make_net(
        runtime,
        TestDriver {
            rx: a_to_b,
            tx: b_to_a,
            mac: [0x02, 0, 0, 0, 1, 2],
        },
        Ipv4Cidr::new(Ipv4Address::new(10, 0, 9, 2), 24),
        0x00de_dede_dede_de00,
    );

    let driver = MqttDriver::with_stack(
        Device {
            id: "mqtt-device-1".to_string(),
            name: "MQTT Device".to_string(),
            status: DeviceStatus::Online,
            endpoint: DeviceEndpoint::mqtt(MqttEndpointConfig {
                broker: "10.0.9.2:1883".to_string(),
                client_id: "qemu-client".to_string(),
                auth: None,
                tls: None,
                keepalive_secs: Some(30),
                clean_start: true,
                session_expiry_secs: None,
                topic_prefix: None,
                connect_properties: MqttConnectProperties::default(),
                will: None,
                reconnect: BrokerReconnectConfig::default(),
                supported_versions: vec![MqttProtocolVersion::V5_0],
            }),
            metadata: None,
            max_connections: Some(4),
            resources: Map::default(),
            message_endpoints: Vec::new(),
        },
        runtime.clone(),
        net,
    );

    assert_eq!(driver.negotiated_connack_async().await, Ok(None));
    assert_eq!(
        driver.listener_status_async().await,
        Ok(MqttListenerStatus::Stopped)
    );

    // No broker listens on the peer stack, so the live connect must fail cleanly.
    log::debug!("attempting MQTT connect to 10.0.9.2:1883 (expected to be refused)");
    let connect = driver.start().await;
    log::trace!("connect result: {connect:?}");
    assert!(connect.is_err(), "connect should be refused");
    assert!(
        connect.as_ref().unwrap_err().contains("failed to connect"),
        "unexpected error: {connect:?}"
    );

    // The failed connect must leave the driver in a clean, stopped state.
    assert_eq!(
        driver.listener_status_async().await,
        Ok(MqttListenerStatus::Stopped)
    );
    driver.stop().await.expect("stop should succeed");
}
