//! Tests against the real host services brought up by `just up`: UART1 is bridged via
//! a pty to diagslave (Modbus RTU), and the emulated LAN9118 NIC rides QEMU's
//! user-mode network stack, giving the guest a real TCP/IP path to mosquitto (:41883),
//! a plain HTTP file server (:48080), and a Modbus TCP diagslave (:41502) on the host.

pub mod http;
pub mod modbus;
pub mod mqtt;

use embassy_executor::Spawner;
use embassy_futures::select::{Either, select};
use embassy_net::{Config, Stack, StackResources};
use ferredge_runtime_embassy::{EmbassyNet, EmbassyNetConfig, EmbassyRuntime};
use static_cell::StaticCell;

use crate::hw::lan9118;

/// QEMU's user-mode network stack forwards this virtual address to the host's
/// loopback; every host service is reached through it (default subnet 10.0.2.0,
/// host alias .2).
const HOST_ADDR: &str = "10.0.2.2";

pub async fn run(spawner: Spawner, runtime: &EmbassyRuntime) {
    modbus::rtu(runtime).await;
    log::info!("harness modbus RTU tests passed");

    let stack = net_up(spawner).await;
    refused(EmbassyNet::new(stack, EmbassyNetConfig::default())).await;
    log::info!("harness refused-connection test passed");
    mqtt::run(runtime, EmbassyNet::new(stack, EmbassyNetConfig::default())).await;
    log::info!("harness mqtt tests passed");
    http::run(runtime, EmbassyNet::new(stack, EmbassyNetConfig::default())).await;
    log::info!("harness http tests passed");
    modbus::tcp(runtime, EmbassyNet::new(stack, EmbassyNetConfig::default())).await;
    log::info!("harness modbus TCP tests passed");
}

/// Services the LAN9118 FIFOs, shuttling frames to and from the driver channel.
#[embassy_executor::task]
async fn lan9118_task(runner: lan9118::Runner) -> ! {
    runner.run().await
}

/// Drives the embassy-net stack over the LAN9118 device.
#[embassy_executor::task]
async fn net_task(mut runner: embassy_net::Runner<'static, lan9118::Device>) -> ! {
    runner.run().await
}

/// Brings up embassy-net on the emulated LAN9118 and waits for DHCP from QEMU's
/// user-mode network stack (which assigns the guest 10.0.2.15).
async fn net_up(spawner: Spawner) -> Stack<'static> {
    static STATE: StaticCell<lan9118::State> = StaticCell::new();
    let (device, driver_runner) = lan9118::new(lan9118::LAN9118_BASE, STATE.init(lan9118::State::new()));
    spawner.spawn(lan9118_task(driver_runner).expect("lan9118 task slot"));

    static RESOURCES: StaticCell<StackResources<8>> = StaticCell::new();
    let (stack, net_runner) = embassy_net::new(
        device,
        Config::dhcpv4(Default::default()),
        RESOURCES.init(StackResources::new()),
        0x0f0e_0d0c_0b0a_0908,
    );
    spawner.spawn(net_task(net_runner).expect("net task slot"));

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
