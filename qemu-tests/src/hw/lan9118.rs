//! Minimal driver for the SMSC LAN9118 ethernet MAC that QEMU's `mps2-an385` board
//! emulates at `0x4020_0000`, exposed to embassy-net through
//! `embassy-net-driver-channel`. With `-nic user,model=lan9118` this gives the guest a
//! real, unprivileged TCP/IP path to the host (slirp: guest 10.0.2.15, host 10.0.2.2).
//!
//! The driver is deliberately QEMU-scoped: no interrupts (a polling task services the
//! FIFOs), no PHY/MII management (the emulated link is always up), and no checksum
//! offload. Behavior was written against
//! [qemu-11.0.2 `hw/net/lan9118.c`](https://raw.githubusercontent.com/qemu/qemu/refs/tags/v11.0.2/hw/net/lan9118.c).
//! The module only depends on embassy crates and raw MMIO, so it could be extracted into a
//! standalone `embassy-net-lan9118` crate later.

use core::ptr::{read_volatile, write_volatile};

use embassy_futures::yield_now;
use embassy_net_driver::{HardwareAddress, LinkState};
use embassy_net_driver_channel as ch;

/// MMIO base of the LAN9118 on the mps2-an385 (qemu `hw/arm/mps2.c`).
pub const LAN9118_BASE: usize = 0x4020_0000;

/// Max ethernet frame size (1500 payload + 14 header), excluding the FCS.
pub const MTU: usize = 1514;

pub type State = ch::State<MTU, 4, 4>;
pub type Device = ch::Device<'static, MTU>;

// Register offsets (LAN9118 datasheet §5 / qemu hw/net/lan9118.c). All accesses must
// be aligned 32-bit ops: the QEMU model only implements the 32-bit interface.
const RX_DATA_FIFO: usize = 0x00;
const TX_DATA_FIFO: usize = 0x20;
const RX_STATUS_FIFO: usize = 0x40;
const TX_STATUS_FIFO: usize = 0x48;
const ID_REV: usize = 0x50;
const BYTE_TEST: usize = 0x64;
const TX_CFG: usize = 0x70;
const HW_CFG: usize = 0x74;
const RX_FIFO_INF: usize = 0x7C;
const TX_FIFO_INF: usize = 0x80;
const PMT_CTRL: usize = 0x84;
const MAC_CSR_CMD: usize = 0xA4;
const MAC_CSR_DATA: usize = 0xA8;

const BYTE_TEST_PATTERN: u32 = 0x8765_4321;
const TX_CFG_TX_ON: u32 = 1 << 1;
const HW_CFG_SRST: u32 = 1;
const PMT_CTRL_READY: u32 = 1;
const MAC_CSR_BUSY: u32 = 1 << 31;
const MAC_CSR_READ: u32 = 1 << 30;

// Indirect MAC registers, accessed through MAC_CSR_CMD/MAC_CSR_DATA.
const MAC_CR: u32 = 1;
const MAC_ADDRH: u32 = 2;
const MAC_ADDRL: u32 = 3;
const MAC_CR_TXEN: u32 = 1 << 3;
const MAC_CR_RXEN: u32 = 1 << 2;

// TX command word A: First Segment, Last Segment, buffer length in [10:0].
const TX_CMD_A_FIRST: u32 = 1 << 13;
const TX_CMD_A_LAST: u32 = 1 << 12;

struct Regs {
    base: usize,
}

impl Regs {
    fn read(&self, offset: usize) -> u32 {
        unsafe { read_volatile((self.base + offset) as *const u32) }
    }

    fn write(&self, offset: usize, value: u32) {
        unsafe { write_volatile((self.base + offset) as *mut u32, value) }
    }

    /// Busy-waits until `reg & mask == want`. Every wait in this driver completes
    /// immediately under QEMU (the model is synchronous), so a small bound is enough
    /// to turn a wrong register offset into a loud failure instead of a hang.
    fn wait(&self, offset: usize, mask: u32, want: u32, what: &str) {
        for _ in 0..100_000 {
            if self.read(offset) & mask == want {
                return;
            }
        }
        panic!("lan9118: timed out waiting for {what}");
    }

    fn mac_write(&self, reg: u32, value: u32) {
        self.wait(MAC_CSR_CMD, MAC_CSR_BUSY, 0, "MAC CSR idle");
        self.write(MAC_CSR_DATA, value);
        self.write(MAC_CSR_CMD, MAC_CSR_BUSY | reg);
        self.wait(MAC_CSR_CMD, MAC_CSR_BUSY, 0, "MAC CSR write");
    }

    fn mac_read(&self, reg: u32) -> u32 {
        self.wait(MAC_CSR_CMD, MAC_CSR_BUSY, 0, "MAC CSR idle");
        self.write(MAC_CSR_CMD, MAC_CSR_BUSY | MAC_CSR_READ | reg);
        self.wait(MAC_CSR_CMD, MAC_CSR_BUSY, 0, "MAC CSR read");
        self.read(MAC_CSR_DATA)
    }
}

/// Owns the chip and the lower end of the driver channel; [`Runner::run`] must be
/// spawned for the returned [`Device`] to move any frames.
pub struct Runner {
    regs: Regs,
    ch: ch::Runner<'static, MTU>,
}

/// Resets and enables the LAN9118 at `base` and returns the embassy-net device plus
/// the FIFO-servicing runner. The MAC address is the one QEMU programmed into the
/// chip's EEPROM (`-nic ...,mac=` or the default 52:54:00:12:34:56).
pub fn new(base: usize, state: &'static mut State) -> (Device, Runner) {
    let regs = Regs { base };

    assert_eq!(
        regs.read(BYTE_TEST),
        BYTE_TEST_PATTERN,
        "lan9118: BYTE_TEST mismatch — no LAN9118 at {base:#x}"
    );
    log::debug!("lan9118: ID_REV {:#010x}", regs.read(ID_REV));

    regs.write(HW_CFG, HW_CFG_SRST);
    regs.wait(HW_CFG, HW_CFG_SRST, 0, "soft reset");
    regs.wait(PMT_CTRL, PMT_CTRL_READY, PMT_CTRL_READY, "power-up ready");

    let addrl = regs.mac_read(MAC_ADDRL);
    let addrh = regs.mac_read(MAC_ADDRH);
    let mut mac = [0u8; 6];
    mac[..4].copy_from_slice(&addrl.to_le_bytes());
    mac[4..].copy_from_slice(&addrh.to_le_bytes()[..2]);
    log::debug!(
        "lan9118: MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        mac[0],
        mac[1],
        mac[2],
        mac[3],
        mac[4],
        mac[5]
    );

    regs.write(TX_CFG, TX_CFG_TX_ON);
    regs.mac_write(MAC_CR, MAC_CR_TXEN | MAC_CR_RXEN);

    let (mut runner, device) = ch::new(state, HardwareAddress::Ethernet(mac));
    // The emulated link is always up; there is no PHY state worth tracking.
    runner.set_link_state(LinkState::Up);

    (device, Runner { regs, ch: runner })
}

impl Runner {
    /// Shuttles frames between the chip FIFOs and the driver channel forever.
    pub async fn run(mut self) -> ! {
        loop {
            let progressed = self.service_rx() | self.service_tx();
            if !progressed {
                yield_now().await;
            }
        }
    }

    /// Delivers at most one received frame up the channel. The RX status length
    /// includes the 4-byte FCS that the model appends to the data FIFO; the frame is
    /// stored as little-endian dwords, zero-padded to a word boundary.
    fn service_rx(&mut self) -> bool {
        if (self.regs.read(RX_FIFO_INF) >> 16) & 0xff == 0 {
            return false;
        }
        let Some(buf) = self.ch.try_rx_buf() else {
            return false;
        };

        let status = self.regs.read(RX_STATUS_FIFO);
        let len = ((status >> 16) & 0x3fff) as usize;
        // Errored or oversized frames are drained from the data FIFO all the same
        // (RX_DP_CTRL fast-forward desyncs the model's FIFO accounting) but not
        // delivered. QEMU never flags errors, so this path is belt-and-braces.
        let deliver = status & (1 << 15) == 0 && len >= 4 && len - 4 <= MTU;
        for i in (0..len).step_by(4) {
            let word = self.regs.read(RX_DATA_FIFO).to_le_bytes();
            if deliver && i < len - 4 {
                let n = (len - 4 - i).min(4);
                buf[i..i + n].copy_from_slice(&word[..n]);
            }
        }
        if deliver {
            self.ch.rx_done(len - 4);
        } else {
            log::warn!("lan9118: dropping RX frame (status {status:#010x})");
        }
        true
    }

    /// Pushes at most one outbound frame into the TX FIFO. The model transmits
    /// synchronously on the last data word, so space is always available by the time
    /// the wait runs; the status FIFO is drained per packet so it can never fill.
    fn service_tx(&mut self) -> bool {
        let Some(frame) = self.ch.try_tx_buf() else {
            return false;
        };
        let len = frame.len();

        let needed = (len.div_ceil(4) + 2) * 4;
        for _ in 0..100_000 {
            if (self.regs.read(TX_FIFO_INF) & 0xffff) as usize >= needed {
                break;
            }
        }

        self.regs
            .write(TX_DATA_FIFO, TX_CMD_A_FIRST | TX_CMD_A_LAST | len as u32);
        self.regs.write(TX_DATA_FIFO, len as u32);
        for chunk in frame.chunks(4) {
            let mut word = [0u8; 4];
            word[..chunk.len()].copy_from_slice(chunk);
            self.regs.write(TX_DATA_FIFO, u32::from_le_bytes(word));
        }
        self.ch.tx_done();

        while (self.regs.read(TX_FIFO_INF) >> 16) & 0xff != 0 {
            let _ = self.regs.read(TX_STATUS_FIFO);
        }
        true
    }
}
