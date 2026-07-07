//! Self-contained test suites: everything runs in-guest against the in-memory fakes,
//! with no host services required (`just run`).

pub mod modbus;
pub mod mqtt;
pub mod net;
pub mod runtime;
pub mod serial;

use embassy_executor::Spawner;
use ferredge_runtime_embassy::EmbassyRuntime;

pub async fn run_all(spawner: Spawner, rt: &EmbassyRuntime) {
    runtime::run(rt).await;
    log::info!("runtime tests passed");
    serial::run().await;
    log::info!("serial tests passed");
    net::run(spawner, rt).await;
    log::info!("net tests passed");
    modbus::run(rt).await;
    log::info!("modbus driver tests passed");
    mqtt::run(spawner, rt).await;
    log::info!("mqtt driver tests passed");
}
