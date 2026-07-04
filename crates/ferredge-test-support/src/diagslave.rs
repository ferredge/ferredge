use std::{
    net::TcpStream,
    process::Command,
    thread,
    time::{Duration, Instant},
};

use crate::{
    net::reserve_free_port,
    process::{ProcessGuard, piped_stdio, require_command},
};

const DIAGSLAVE_START_TIMEOUT: Duration = Duration::from_secs(5);
const DIAGSLAVE_POLL_INTERVAL: Duration = Duration::from_millis(25);
const DIAGSLAVE_UDP_START_SETTLE: Duration = Duration::from_millis(300);
const DIAGSLAVE_SERIAL_START_SETTLE: Duration = Duration::from_millis(300);
const DIAGSLAVE_TCP_START_SETTLE: Duration = Duration::from_millis(50);
const DIAGSLAVE_START_ATTEMPTS: usize = 5;

pub struct DiagslaveGuard {
    process: Option<ProcessGuard>,
    port: u16,
    mode: &'static str,
    serial_port: Option<String>,
}

impl DiagslaveGuard {
    pub fn start(mode: &'static str) -> Self {
        require_command("diagslave");
        let port = reserve_free_port();
        let mut guard = Self {
            process: None,
            port,
            mode,
            serial_port: None,
        };
        guard.start_slave();
        guard
    }

    #[cfg(unix)]
    pub fn start_serial(mode: &'static str, serial_port: &str) -> Self {
        require_command("diagslave");
        let mut guard = Self {
            process: None,
            port: 0,
            mode,
            serial_port: Some(serial_port.to_string()),
        };
        guard.start_serial_slave(serial_port);
        guard
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn start_slave(&mut self) {
        assert!(self.process.is_none(), "diagslave already running");
        // The reserved port is released before diagslave binds it, so a parallel
        // test can steal it in between; retry on a fresh port when that happens.
        for attempt in 0..DIAGSLAVE_START_ATTEMPTS {
            if attempt > 0 {
                self.port = reserve_free_port();
            }
            if self.try_start_slave() {
                return;
            }
            self.stop_slave();
        }
        panic!(
            "diagslave {} failed to start after {DIAGSLAVE_START_ATTEMPTS} attempts",
            self.mode
        );
    }

    /// Spawns diagslave on `self.port` and waits for readiness. Returns `false`
    /// when the process exits during startup — typically because another process
    /// won the race for the port — so the caller can retry.
    fn try_start_slave(&mut self) -> bool {
        let mut process = {
            let mut command = Command::new("diagslave");
            command.args(["-m", self.mode, "-a", "1", "-p", &self.port.to_string()]);
            ProcessGuard::spawn(
                format!("diagslave {}", self.mode),
                piped_stdio(&mut command),
                true,
            )
        };

        if self.mode == "udp" {
            thread::sleep(DIAGSLAVE_UDP_START_SETTLE);
            if process.has_exited() {
                return false;
            }
            self.process = Some(process);
            return true;
        }

        let deadline = Instant::now() + DIAGSLAVE_START_TIMEOUT;
        loop {
            if process.has_exited() {
                return false;
            }
            if TcpStream::connect(("127.0.0.1", self.port)).is_ok() {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "diagslave {} should start before timeout",
                self.mode
            );
            thread::sleep(DIAGSLAVE_POLL_INTERVAL);
        }
        thread::sleep(DIAGSLAVE_TCP_START_SETTLE);
        // A successful probe may have reached another process that owns the
        // port; only a live child proves the listener is ours.
        if process.has_exited() {
            return false;
        }
        self.process = Some(process);
        true
    }

    #[cfg(unix)]
    fn start_serial_slave(&mut self, serial_port: &str) {
        assert!(self.process.is_none(), "diagslave already running");
        let mut command = Command::new("diagslave");
        command.args([
            "-m",
            self.mode,
            "-a",
            "1",
            "-b",
            "9600",
            "-d",
            "8",
            "-s",
            "1",
            "-p",
            "none",
            serial_port,
        ]);
        self.process = Some(ProcessGuard::spawn(
            format!("diagslave {}", self.mode),
            piped_stdio(&mut command),
            true,
        ));
        thread::sleep(DIAGSLAVE_SERIAL_START_SETTLE);
    }

    pub fn stop_slave(&mut self) {
        if let Some(mut process) = self.process.take() {
            process.stop();
        }
    }

    pub fn restart(&mut self) {
        self.stop_slave();
        #[cfg(unix)]
        if let Some(serial_port) = self.serial_port.clone() {
            self.start_serial_slave(&serial_port);
            return;
        }
        // Drivers under test already hold this port, so retry on it as-is
        // instead of reserving a fresh one.
        for _ in 0..DIAGSLAVE_START_ATTEMPTS {
            if self.try_start_slave() {
                return;
            }
            self.stop_slave();
            thread::sleep(DIAGSLAVE_POLL_INTERVAL);
        }
        panic!(
            "diagslave {} failed to restart on port {} after {DIAGSLAVE_START_ATTEMPTS} attempts",
            self.mode, self.port
        );
    }
}

impl Drop for DiagslaveGuard {
    fn drop(&mut self) {
        self.stop_slave();
    }
}
