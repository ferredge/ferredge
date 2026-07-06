//! Self-contained test suites: everything runs in-guest against the in-memory fakes,
//! with no host services required (`just run`).

pub mod modbus;
pub mod mqtt;
pub mod net;
pub mod runtime;
pub mod serial;

use ferredge_runtime_embassy::EmbassyRuntime;

pub async fn run_all(rt: &EmbassyRuntime) {
    runtime::run(rt).await;
    log::info!("runtime tests passed");
    serial::run().await;
    log::info!("serial tests passed");
    net::run(rt).await;
    log::info!("net tests passed");
    modbus::run(rt).await;
    log::info!("modbus driver tests passed");
    mqtt::run(rt).await;
    log::info!("mqtt driver tests passed");
}
