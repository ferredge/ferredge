//! Runs the Zenoh driver on bare metal: construction with an explicit stack and
//! basic driver initialization tests. The zenoh-nostd integration is not yet complete,
//! so this verifies the driver compiles and initializes without panicking.

use alloc::{rc::Rc, string::ToString, vec::Vec};
use core::cell::RefCell;

use embassy_executor::Spawner;
use embassy_net::{Ipv4Address, Ipv4Cidr};
use ferredge_core::prelude::*;
use ferredge_proto_zenoh::{ZenohDriver, ZenohEndpointConfig};
use ferredge_runtime_embassy::EmbassyRuntime;

use crate::fakes::{SharedWire, TestDriver, Wire, make_net};

pub async fn run(spawner: Spawner, runtime: &EmbassyRuntime) {
    log::debug!("starting zenoh driver tests");

    // Create a fake network stack for testing
    let a_to_b: SharedWire = Rc::new(RefCell::new(Wire::default()));
    let b_to_a: SharedWire = Rc::new(RefCell::new(Wire::default()));
    let net = make_net(
        spawner,
        TestDriver {
            rx: b_to_a.clone(),
            tx: a_to_b.clone(),
            mac: [0x02, 0, 0, 0, 2, 1],
        },
        Ipv4Cidr::new(Ipv4Address::new(10, 0, 10, 1), 24),
        0x00c0_ffee_c0ff_ee01,
    );

    // Create a Zenoh driver with default config
    let device = Device {
        id: DeviceId::new("zenoh-device-1"),
        name: "Zenoh Device".to_string(),
        status: DeviceStatus::Online,
        endpoint: ZenohEndpointConfig {
            locators: Vec::new(), // Empty = use default discovery
        },
        metadata: None,
        max_connections: Some(4),
        resources: Map::default(),
        message_endpoints: Vec::new(),
    };

    // Create driver - this verifies the embassy runtime integration compiles
    let driver = ZenohDriver::with_stack(
        device,
        runtime.clone(),
        net,
    );

    log::debug!("zenoh driver constructed successfully");

    // Test plan conversion
    test_plan_conversion(&driver);

    log::debug!("zenoh driver tests completed");
}

fn test_plan_conversion(driver: &ZenohDriver) {
    let command = Command {
        id: CommandId::new("cmd-1"),
        source_device_id: None,
        target_device_id: DeviceId::new("zenoh-device-1"),
        intent: Intent::Send {
            channel: BrokerAddress {
                name: ChannelName::new("sensor/temperature"),
                kind: Some(BrokerChannelKind::Topic),
            },
            payload: PayloadValue::String(alloc::borrow::Cow::Borrowed("25.5")),
            options: Default::default(),
        },
        correlation: None,
    };

    // Test plan generation
    let plan = driver.plan_native(&command);
    assert!(plan.is_ok(), "Plan generation should succeed");

    log::debug!("zenoh plan conversion test passed");
}
