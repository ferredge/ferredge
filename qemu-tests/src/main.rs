//! Executes the ferredge embassy adapters on a bare-metal Cortex-M3 under QEMU.
//!
//! Time is driven by a real SysTick-based embassy-time driver (`systick-timer`), so
//! timers track wall-clock time — a prerequisite for talking to real host services.
//! The self-contained suites ([`suites`]) run against in-memory fakes ([`fakes`]); the
//! optional [`harness`] suites (feature `harness`) talk to real host services over the
//! emulated LAN9118 NIC and a pty-bridged UART ([`hw`]).
//!
//! The binary exits QEMU via semihosting: exit code 0 when every check passed, and a
//! panic (assert failure, allocation error) exits non-zero via `panic-semihosting`.

#![no_std]
#![no_main]

extern crate alloc;
extern crate panic_semihosting;

use core::mem::MaybeUninit;

use cortex_m_semihosting::debug;
use embassy_executor::Spawner;
use embedded_alloc::LlffHeap as Heap;
use ferredge_runtime_embassy::EmbassyRuntime;

mod fakes;
#[cfg(feature = "harness")]
mod harness;
mod hw;
mod logger;
mod suites;

#[global_allocator]
static HEAP: Heap = Heap::empty();

const HEAP_SIZE: usize = 1024 * 1024; // 1 MB

static LOGGER: logger::SemihostingLogger = logger::SemihostingLogger;

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    {
        static mut HEAP_MEM: [MaybeUninit<u8>; HEAP_SIZE] = [MaybeUninit::uninit(); HEAP_SIZE];
        unsafe { HEAP.init(core::ptr::addr_of_mut!(HEAP_MEM) as usize, HEAP_SIZE) }
    }

    log::set_logger(&LOGGER).expect("logger installs once");
    // Debug keeps the run readable; raise to Trace when chasing a failure.
    log::set_max_level(log::LevelFilter::Debug);
    log::debug!("heap initialized ({HEAP_SIZE} bytes)");

    let mut peripherals = cortex_m::Peripherals::take().expect("core peripherals");
    hw::time_driver::init(&mut peripherals.SYST);
    log::debug!("SysTick time driver initialized");

    let runtime: EmbassyRuntime = EmbassyRuntime::new(spawner);

    suites::run_all(spawner, &runtime).await;
    #[cfg(feature = "harness")]
    harness::run(spawner, &runtime).await;
    log::info!("all embedded embassy tests passed");
    debug::exit(debug::EXIT_SUCCESS);
}
