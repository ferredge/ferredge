//! Exercises the `AsyncRuntime` surface: spawn/join, abort, channels, mutexes, and
//! SysTick-backed sleep.

use core::time::Duration;

use ferredge_core::prelude::*;
use ferredge_runtime_embassy::EmbassyRuntime;

pub async fn run(runtime: &EmbassyRuntime) {
    // spawn/join, repeated to exercise task-storage reuse
    for round in 0..3u8 {
        log::trace!("spawn/join round {round}");
        let mut task = runtime.spawn(async move { round });
        assert_eq!(task.join().await, Ok(round));
        assert!(task.is_finished());
    }

    // abort
    log::debug!("testing task abort");
    let sleeper = runtime.clone();
    let mut task = runtime.spawn(async move {
        sleeper.sleep(Duration::from_secs(3600)).await;
    });
    task.abort();
    assert_eq!(task.join().await, Err(TaskJoinError::Cancelled));

    // channel (compile-time capacity is 16 for the default runtime)
    log::debug!("testing channel send/recv");
    let (tx, mut rx) = runtime.channel::<u8>(16);
    tx.send(1).await.expect("send should succeed");
    tx.try_send(2).expect("try_send should succeed");
    assert_eq!(rx.recv().await, Ok(1));
    assert_eq!(rx.try_recv(), Ok(2));
    assert_eq!(rx.try_recv(), Err(ChannelError::Empty));

    // mutex
    log::debug!("testing mutex lock/try_lock");
    let mutex = runtime.mutex(11u8);
    {
        let mut guard = mutex.lock().await.expect("lock should succeed");
        *guard = 13;
        assert!(matches!(mutex.try_lock(), Err(MutexError::Busy)));
    }
    assert_eq!(*mutex.try_lock().expect("try_lock should succeed"), 13);

    // sleep + instant against the SysTick clock
    log::debug!("testing sleep against the SysTick clock");
    let started = runtime.now();
    runtime.sleep(Duration::from_millis(50)).await;
    let elapsed = started.elapsed();
    log::trace!("slept 50ms, clock advanced {elapsed:?}");
    assert!(elapsed >= Duration::from_millis(50));
}
