use std::{
    net::TcpStream,
    process::Command,
    thread,
    time::{Duration, Instant},
};

use crate::{
    net::reserve_free_port,
    process::{ProcessGuard, null_stdio, require_command},
};

const MOSQUITTO_START_TIMEOUT: Duration = Duration::from_secs(5);
const MOSQUITTO_POLL_INTERVAL: Duration = Duration::from_millis(25);
const MOSQUITTO_START_ATTEMPTS: usize = 5;

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
        // The reserved port is released before mosquitto binds it, so a parallel
        // test can steal it in between; retry on a fresh port when that happens.
        for attempt in 0..MOSQUITTO_START_ATTEMPTS {
            if attempt > 0 {
                guard.port = reserve_free_port();
            }
            if guard.try_start_broker() {
                return guard;
            }
            guard.stop_broker();
        }
        panic!("mosquitto failed to start after {MOSQUITTO_START_ATTEMPTS} attempts");
    }

    /// Restarts the broker on the port already handed out to clients, so the
    /// port is retried as-is instead of reserving a fresh one.
    pub fn start_broker(&mut self) {
        assert!(self.process.is_none(), "mosquitto broker already running");
        for _ in 0..MOSQUITTO_START_ATTEMPTS {
            if self.try_start_broker() {
                return;
            }
            self.stop_broker();
            thread::sleep(MOSQUITTO_POLL_INTERVAL);
        }
        panic!(
            "mosquitto failed to restart on port {} after {MOSQUITTO_START_ATTEMPTS} attempts",
            self.port
        );
    }

    /// Spawns mosquitto on `self.port` and waits for readiness. Returns `false`
    /// when the process exits during startup — typically because another process
    /// won the race for the port — so the caller can retry.
    fn try_start_broker(&mut self) -> bool {
        let mut command = Command::new("mosquitto");
        command.args(["-p", &self.port.to_string(), "-v"]);
        let mut process = ProcessGuard::spawn("mosquitto", null_stdio(&mut command), false);

        let deadline = Instant::now() + MOSQUITTO_START_TIMEOUT;
        loop {
            if process.has_exited() {
                return false;
            }
            if TcpStream::connect(("127.0.0.1", self.port)).is_ok() {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "mosquitto should start before timeout"
            );
            thread::sleep(MOSQUITTO_POLL_INTERVAL);
        }
        // A successful probe may have reached another process that owns the
        // port; only a live child proves the listener is ours.
        if process.has_exited() {
            return false;
        }
        self.process = Some(process);
        true
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
