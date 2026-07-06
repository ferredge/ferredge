//! Tests against the real host services brought up by `just up`: UART1 is bridged via
//! a pty to diagslave (Modbus RTU), and the emulated LAN9118 NIC rides QEMU's
//! user-mode network stack, giving the guest a real TCP/IP path to mosquitto (:41883),
//! a plain HTTP file server (:48080), and a Modbus TCP diagslave (:41502) on the host.

pub mod http;
pub mod modbus;
pub mod mqtt;

use alloc::boxed::Box;

use embassy_futures::select::{Either, select};
use embassy_net::{Config, Stack, StackResources};
use ferredge_core::prelude::AsyncRuntime;
use ferredge_runtime_embassy::{EmbassyNet, EmbassyNetConfig, EmbassyRuntime};

use crate::hw::lan9118;

/// QEMU's user-mode network stack forwards this virtual address to the host's
/// loopback; every host service is reached through it (default subnet 10.0.2.0,
/// host alias .2).
const HOST_ADDR: &str = "10.0.2.2";

pub async fn run(runtime: &EmbassyRuntime) {
    modbus::rtu(runtime).await;
    log::info!("harness modbus RTU tests passed");

    let stack = net_up(runtime).await;
    refused(EmbassyNet::new(stack, EmbassyNetConfig::default())).await;
    log::info!("harness refused-connection test passed");
    mqtt::run(runtime, EmbassyNet::new(stack, EmbassyNetConfig::default())).await;
    log::info!("harness mqtt tests passed");
    http::run(runtime, EmbassyNet::new(stack, EmbassyNetConfig::default())).await;
    log::info!("harness http tests passed");
    modbus::tcp(runtime, EmbassyNet::new(stack, EmbassyNetConfig::default())).await;
    log::info!("harness modbus TCP tests passed");
}

/// Brings up embassy-net on the emulated LAN9118 and waits for DHCP from QEMU's
/// user-mode network stack (which assigns the guest 10.0.2.15).
async fn net_up(runtime: &EmbassyRuntime) -> Stack<'static> {
    let state: &'static mut lan9118::State = Box::leak(Box::new(lan9118::State::new()));
    let (device, driver_runner) = lan9118::new(lan9118::LAN9118_BASE, state);
    runtime.spawn(async move { driver_runner.run().await });

    let resources: &'static mut StackResources<8> = Box::leak(Box::new(StackResources::new()));
    let (stack, mut net_runner) = embassy_net::new(
        device,
        Config::dhcpv4(Default::default()),
        resources,
        0x0f0e_0d0c_0b0a_0908,
    );
    runtime.spawn(async move { net_runner.run().await });

    log::debug!("waiting for DHCP on the LAN9118 (15s timeout)");
    match select(stack.wait_config_up(), embassy_time::Timer::after_secs(15)).await {
        Either::First(()) => {
            let config = stack.config_v4().expect("config is up");
            log::debug!("DHCP assigned {}", config.address);
            stack
        }
        Either::Second(()) => panic!("DHCP did not complete within 15s"),
    }
}

/// QEMU's user-mode stack surfaces a host-side connection refusal as a TCP RST, so
/// connecting to a port nothing listens on must fail cleanly instead of hanging.
async fn refused(net: EmbassyNet) {
    use ferredge_core::prelude::*;

    log::debug!("connecting to {HOST_ADDR}:1 (expected to be refused)");
    let error = net
        .connect(&alloc::format!("{HOST_ADDR}:1"))
        .await
        .err()
        .expect("connect to a closed port should fail");
    log::debug!("connect refused as expected: {error:?}");
}
