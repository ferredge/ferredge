use std::{net::TcpStream, process::Command, time::Duration};

use crate::{
    net::reserve_free_port,
    process::{ProcessGuard, null_stdio, require_command},
    wait::wait_until,
};

const MOSQUITTO_START_TIMEOUT: Duration = Duration::from_secs(5);
const MOSQUITTO_POLL_INTERVAL: Duration = Duration::from_millis(25);

pub struct MosquittoGuard {
    process: Option<ProcessGuard>,
    port: u16,
}

impl MosquittoGuard {
    pub fn start() -> Self {
        require_command("mosquitto");
        let port = reserve_free_port();
        let mut guard = Self {
            process: None,
            port,
        };
        guard.start_broker();
        guard
    }

    pub fn start_broker(&mut self) {
        assert!(self.process.is_none(), "mosquitto broker already running");
        let mut command = Command::new("mosquitto");
        command.args(["-p", &self.port.to_string(), "-v"]);
        self.process = Some(ProcessGuard::spawn(
            "mosquitto",
            null_stdio(&mut command),
            false,
        ));
        wait_until(
            "mosquitto should start before timeout",
            MOSQUITTO_START_TIMEOUT,
            MOSQUITTO_POLL_INTERVAL,
            || TcpStream::connect(("127.0.0.1", self.port)).is_ok(),
        );
    }

    pub fn stop_broker(&mut self) {
        if let Some(mut process) = self.process.take() {
            process.stop();
        }
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn host(&self) -> &str {
        "127.0.0.1"
    }

    pub fn port_string(&self) -> String {
        self.port.to_string()
    }

    pub fn broker_url(&self) -> String {
        format!("mqtt://127.0.0.1:{}", self.port)
    }
}

impl Drop for MosquittoGuard {
    fn drop(&mut self) {
        self.stop_broker();
    }
}
