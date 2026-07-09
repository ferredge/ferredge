//! Minimal polling driver for the CMSDK APB UART on the mps2-an385, exposing
//! `embedded-io-async` `Read`/`Write`. QEMU backs each UART with a chardev,
//! so polling with `yield_now` between checks is plenty.

use embassy_futures::yield_now;

pub const UART1_BASE: usize = 0x4000_5000;

const DATA: usize = 0x00;
const STATE: usize = 0x04;
const CTRL: usize = 0x08;
const BAUDDIV: usize = 0x10;

const STATE_TX_FULL: u32 = 1 << 0;
const STATE_RX_FULL: u32 = 1 << 1;
const CTRL_TX_EN: u32 = 1 << 0;
const CTRL_RX_EN: u32 = 1 << 1;

pub struct CmsdkUart {
    base: usize,
}

impl CmsdkUart {
    /// # Safety-adjacent note
    /// Each base address must be owned by exactly one instance.
    pub fn new(base: usize) -> Self {
        let uart = Self { base };
        // QEMU treats BAUDDIV < 16 as an unusable divider; the pty ignores baud anyway.
        uart.reg_write(BAUDDIV, 16);
        uart.reg_write(CTRL, CTRL_TX_EN | CTRL_RX_EN);
        uart
    }

    fn reg_read(&self, offset: usize) -> u32 {
        unsafe { core::ptr::read_volatile((self.base + offset) as *const u32) }
    }

    fn reg_write(&self, offset: usize, value: u32) {
        unsafe { core::ptr::write_volatile((self.base + offset) as *mut u32, value) }
    }

    fn rx_ready(&self) -> bool {
        self.reg_read(STATE) & STATE_RX_FULL != 0
    }

    fn tx_full(&self) -> bool {
        self.reg_read(STATE) & STATE_TX_FULL != 0
    }

    async fn recv_byte(&mut self) -> u8 {
        while !self.rx_ready() {
            yield_now().await;
        }
        self.reg_read(DATA) as u8
    }
}

impl embedded_io_async::ErrorType for CmsdkUart {
    type Error = core::convert::Infallible;
}

impl embedded_io_async::Read for CmsdkUart {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        if buf.is_empty() {
            return Ok(0);
        }
        let mut n = 0;
        buf[n] = self.recv_byte().await;
        n += 1;
        // Drain whatever else already arrived, without waiting again.
        while n < buf.len() && self.rx_ready() {
            buf[n] = self.reg_read(DATA) as u8;
            n += 1;
        }
        Ok(n)
    }
}

impl embedded_io_async::Write for CmsdkUart {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        for &byte in buf {
            while self.tx_full() {
                yield_now().await;
            }
            self.reg_write(DATA, byte as u32);
        }
        Ok(buf.len())
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        while self.tx_full() {
            yield_now().await;
        }
        Ok(())
    }
}
