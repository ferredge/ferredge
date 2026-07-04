use std::{
    env,
    io::Read,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    thread,
};

pub fn require_command(command: &str) {
    assert!(
        command_in_path(command),
        "required test binary `{command}` was not found in PATH"
    );
}

pub fn command_in_path(command: &str) -> bool {
    let path = env::var_os("PATH").unwrap_or_default();
    env::split_paths(&path).any(|dir| {
        candidate_paths(command, dir)
            .into_iter()
            .any(|path| path.exists())
    })
}

fn candidate_paths(command: &str, dir: PathBuf) -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        use std::{ffi::OsString, path::Path};

        let pathext =
            env::var_os("PATHEXT").unwrap_or_else(|| OsString::from(".COM;.EXE;.BAT;.CMD"));
        let mut paths = Vec::new();
        for ext in pathext
            .to_string_lossy()
            .split(';')
            .filter(|ext| !ext.is_empty())
        {
            paths.push(dir.join(format!("{command}{ext}")));
        }
        if Path::new(command).extension().is_some() {
            paths.push(dir.join(command));
        }
        paths
    }

    #[cfg(not(windows))]
    {
        vec![dir.join(command)]
    }
}

pub struct ChildOutputCapture {
    buffer: Arc<Mutex<Vec<u8>>>,
    stderr_task: Option<thread::JoinHandle<()>>,
    stdout_task: Option<thread::JoinHandle<()>>,
}

impl ChildOutputCapture {
    pub fn start(child: &mut Child) -> Self {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let stdout_task = child
            .stdout
            .take()
            .map(|stdout| spawn_output_capture(stdout, Arc::clone(&buffer)));
        let stderr_task = child
            .stderr
            .take()
            .map(|stderr| spawn_output_capture(stderr, Arc::clone(&buffer)));
        Self {
            buffer,
            stderr_task,
            stdout_task,
        }
    }

    pub fn finish(&mut self, label: &str) {
        if let Some(task) = self.stdout_task.take() {
            let _ = task.join();
        }
        if let Some(task) = self.stderr_task.take() {
            let _ = task.join();
        }
        if thread::panicking() {
            let output = self.buffer.lock().expect("output buffer should lock");
            if !output.is_empty() {
                eprintln!(
                    "captured {label} output:\n{}",
                    String::from_utf8_lossy(&output)
                );
            }
        }
    }
}

fn spawn_output_capture<R>(reader: R, buffer: Arc<Mutex<Vec<u8>>>) -> thread::JoinHandle<()>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut reader = reader;
        let mut output = Vec::new();
        let _ = reader.read_to_end(&mut output);
        if !output.is_empty() {
            buffer
                .lock()
                .expect("output buffer should lock")
                .extend_from_slice(&output);
        }
    })
}

pub struct ProcessGuard {
    child: Option<Child>,
    capture: Option<ChildOutputCapture>,
    label: String,
}

impl ProcessGuard {
    pub fn spawn(label: impl Into<String>, command: &mut Command, capture_output: bool) -> Self {
        let label = label.into();
        let mut child = command.spawn().unwrap_or_else(|error| {
            panic!("failed to spawn {label}: {error}");
        });
        let capture = capture_output.then(|| ChildOutputCapture::start(&mut child));
        Self {
            child: Some(child),
            capture,
            label,
        }
    }

    pub fn child(&self) -> &Child {
        self.child.as_ref().expect("process should be running")
    }

    pub fn child_mut(&mut self) -> &mut Child {
        self.child.as_mut().expect("process should be running")
    }

    pub fn has_exited(&mut self) -> bool {
        match self.child.as_mut() {
            Some(child) => child
                .try_wait()
                .map(|status| status.is_some())
                .unwrap_or(true),
            None => true,
        }
    }

    pub fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(mut capture) = self.capture.take() {
            capture.finish(&self.label);
        }
    }
}

impl Drop for ProcessGuard {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn null_stdio(command: &mut Command) -> &mut Command {
    command.stdout(Stdio::null()).stderr(Stdio::null())
}

pub fn piped_stdio(command: &mut Command) -> &mut Command {
    command.stdout(Stdio::piped()).stderr(Stdio::piped())
}
