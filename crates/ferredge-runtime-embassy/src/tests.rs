use core::future::Future;
use core::time::Duration;

use embassy_executor::Executor;
use ferredge_core::prelude::*;

use crate::EmbassyRuntime;

/// Runs one test future on a leaked embassy executor on a dedicated thread.
///
/// The executor never returns, so completion is reported over an mpsc channel; a panic
/// inside the test future surfaces as a receive timeout (with the panic message already
/// printed by the default hook).
pub(crate) fn run_test<const CHANNEL_CAPACITY: usize, F>(
    make: impl FnOnce(EmbassyRuntime<CHANNEL_CAPACITY>) -> F + Send + 'static,
) where
    F: Future<Output = ()> + 'static,
{
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let executor: &'static mut Executor = Box::leak(Box::new(Executor::new()));
        executor.run(move |spawner| {
            let runtime = EmbassyRuntime::<CHANNEL_CAPACITY>::new(spawner);
            let test_future = make(runtime.clone());
            runtime.spawn(async move {
                test_future.await;
                done_tx
                    .send(())
                    .expect("test main thread should be waiting");
            });
        });
    });
    done_rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("embassy test future should complete");
}

#[test]
fn runtime_spawn_join_and_finish_flag() {
    run_test::<16, _>(|runtime| async move {
        let mut task = runtime.spawn(async { 29u8 });
        assert_eq!(task.join().await, Ok(29));
        assert!(task.is_finished());
        assert_eq!(task.join().await, Err(TaskJoinError::Cancelled));
    });
}

#[test]
fn runtime_spawn_reuses_task_storage_across_rounds() {
    run_test::<16, _>(|runtime| async move {
        for round in 0..3u8 {
            let mut task = runtime.spawn(async move { round });
            assert_eq!(task.join().await, Ok(round));
        }
    });
}

#[test]
fn runtime_abort_cancels_task() {
    run_test::<16, _>(|runtime| async move {
        let sleeper = runtime.clone();
        let mut task = runtime.spawn(async move {
            sleeper.sleep(Duration::from_secs(60)).await;
        });
        assert!(!task.is_finished());
        task.abort();
        assert_eq!(task.join().await, Err(TaskJoinError::Cancelled));
        assert!(task.is_finished());
    });
}

#[test]
fn runtime_channel_capacity_and_try_ops() {
    run_test::<2, _>(|runtime| async move {
        let (tx, mut rx) = runtime.channel::<u8>(2);
        tx.send(1).await.expect("send should succeed");
        tx.try_send(2).expect("try_send should succeed");
        assert_eq!(tx.try_send(3), Err(ChannelError::Full));
        assert_eq!(rx.recv().await, Ok(1));
        assert_eq!(rx.try_recv(), Ok(2));
        assert_eq!(rx.try_recv(), Err(ChannelError::Empty));
    });
}

#[test]
fn runtime_mutex_lock_and_try_lock() {
    run_test::<16, _>(|runtime| async move {
        let mutex = runtime.mutex(11u8);
        {
            let mut guard = mutex.lock().await.expect("lock should succeed");
            *guard = 13;
            assert!(matches!(mutex.try_lock(), Err(MutexError::Busy)));
        }
        assert_eq!(*mutex.try_lock().expect("try_lock should succeed"), 13);
    });
}

#[test]
fn runtime_sleep_advances_instant() {
    run_test::<16, _>(|runtime| async move {
        let started = runtime.now();
        runtime.sleep(Duration::from_millis(50)).await;
        assert!(started.elapsed() >= Duration::from_millis(40));
    });
}

#[cfg(feature = "serial")]
mod serial {
    use std::collections::VecDeque;

    use ferredge_core::prelude::*;

    use super::run_test;
    use crate::EmbassySerial;

    #[derive(Default)]
    struct LoopbackSerial {
        buffer: VecDeque<u8>,
    }

    impl embedded_io_async::ErrorType for LoopbackSerial {
        type Error = core::convert::Infallible;
    }

    impl embedded_io_async::Read for LoopbackSerial {
        async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
            let len = buf.len().min(self.buffer.len());
            for slot in &mut buf[..len] {
                *slot = self.buffer.pop_front().expect("length checked");
            }
            Ok(len)
        }
    }

    impl embedded_io_async::Write for LoopbackSerial {
        async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
            self.buffer.extend(buf);
            Ok(buf.len())
        }

        async fn flush(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    #[test]
    fn serial_registry_hands_out_port_once() {
        run_test::<16, _>(|_runtime| async move {
            let serial = EmbassySerial::new();
            serial.register("/dev/ttyUSB0", LoopbackSerial::default());
            let config = SerialPortConfig {
                path: "/dev/ttyUSB0".to_string(),
                ..SerialPortConfig::default()
            };

            let mut port = serial.open(&config).await.expect("open should succeed");
            assert_eq!(port.write(b"rtu").await, Ok(3));
            assert_eq!(port.flush().await, Ok(()));
            let mut buf = [0u8; 8];
            assert_eq!(port.read(&mut buf).await, Ok(3));
            assert_eq!(&buf[..3], b"rtu");

            assert!(serial.open(&config).await.is_err());
        });
    }

    #[test]
    fn serial_open_unregistered_path_fails() {
        run_test::<16, _>(|_runtime| async move {
            let serial = EmbassySerial::<LoopbackSerial>::new();
            let config = SerialPortConfig {
                path: "/dev/ferredge-missing-serial".to_string(),
                ..SerialPortConfig::default()
            };
            assert!(serial.open(&config).await.is_err());
        });
    }
}
