use std::{
    thread,
    time::{Duration, Instant},
};

pub fn wait_until<F>(
    description: &str,
    timeout: Duration,
    poll_interval: Duration,
    mut condition: F,
) where
    F: FnMut() -> bool,
{
    let deadline = Instant::now() + timeout;
    loop {
        if condition() {
            return;
        }
        assert!(Instant::now() < deadline, "{description}");
        thread::sleep(poll_interval);
    }
}
