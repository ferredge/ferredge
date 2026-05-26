#[cfg(unix)]
use std::{
    fs,
    path::PathBuf,
    process::Command,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
use crate::{
    process::{ProcessGuard, null_stdio, require_command},
    wait::wait_until,
};

#[cfg(unix)]
const SOCAT_START_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(unix)]
const SOCAT_POLL_INTERVAL: Duration = Duration::from_millis(25);

#[cfg(unix)]
pub struct SerialPtyGuard {
    process: Option<ProcessGuard>,
    master_path: PathBuf,
    slave_path: PathBuf,
}

#[cfg(unix)]
impl SerialPtyGuard {
    pub fn start() -> Self {
        require_command("socat");
        let nonce = format!(
            "{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time should be after epoch")
                .as_nanos()
        );
        let master_path = std::env::temp_dir().join(format!("ferredge-master-{nonce}.pty"));
        let slave_path = std::env::temp_dir().join(format!("ferredge-slave-{nonce}.pty"));
        let mut guard = Self {
            process: None,
            master_path,
            slave_path,
        };
        guard.start_pair();
        guard
    }

    pub fn start_pair(&mut self) {
        assert!(self.process.is_none(), "serial pty pair already running");
        let mut command = Command::new("socat");
        command.args([
            "-d",
            "-d",
            &format!(
                "PTY,raw,echo=0,link={},mode=666",
                self.master_path.display()
            ),
            &format!("PTY,raw,echo=0,link={},mode=666", self.slave_path.display()),
        ]);
        self.process = Some(ProcessGuard::spawn(
            "socat",
            null_stdio(&mut command),
            false,
        ));
        wait_until(
            "socat should create PTY links before timeout",
            SOCAT_START_TIMEOUT,
            SOCAT_POLL_INTERVAL,
            || self.master_path.exists() && self.slave_path.exists(),
        );
    }

    pub fn master_path(&self) -> String {
        self.master_path.to_string_lossy().into_owned()
    }

    pub fn slave_path(&self) -> String {
        self.slave_path.to_string_lossy().into_owned()
    }

    pub fn stop_pair(&mut self) {
        if let Some(mut process) = self.process.take() {
            process.stop();
        }
        let _ = fs::remove_file(&self.master_path);
        let _ = fs::remove_file(&self.slave_path);
    }
}

#[cfg(unix)]
impl Drop for SerialPtyGuard {
    fn drop(&mut self) {
        self.stop_pair();
    }
}

#[cfg(windows)]
pub struct SerialPtyGuard;

#[cfg(windows)]
impl SerialPtyGuard {
    pub fn start() -> Self {
        panic!("SerialPtyGuard is not supported on Windows")
    }
}
