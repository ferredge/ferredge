//! Real-time embassy-time driver: `systick-timer`'s `SystickDriver` ticking at
//! `embassy_time_driver::TICK_HZ` (1 kHz) off the mps2-an385 25 MHz SYSCLK.

use cortex_m::peripheral::SYST;
use cortex_m_rt::exception;
use systick_timer::SystickDriver;

const SYSCLK_HZ: u64 = 25_000_000;

// 64 wakeup slots: the harness runs embassy-net (DHCP, TCP timers)
// plus the ferredge test drivers concurrently.
embassy_time_driver::time_driver_impl!(
    static DRIVER: SystickDriver<64> = SystickDriver::new_default(SYSCLK_HZ)
);

/// Starts the SysTick counter; must run before any timer is awaited.
pub fn init(syst: &mut SYST) {
    DRIVER.start(syst);
}

#[exception]
fn SysTick() {
    DRIVER.systick_interrupt();
}
