//! Board support for the QEMU mps2-an385: MMIO drivers for the emulated peripherals
//! (SysTick time base, CMSDK UART, LAN9118 ethernet).

#[cfg(feature = "harness")]
pub mod lan9118;
pub mod time_driver;
#[cfg(feature = "harness")]
pub mod uart;
