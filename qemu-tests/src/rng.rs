//! Custom RNG implementation for bare metal targets.
//!
//! Provides a simple pseudo-random number generator for `getrandom` using the
//! SysTick counter as entropy. This is sufficient for non-cryptographic use cases
//! like UUIDs and session IDs in Zenoh.

use core::sync::atomic::{AtomicU32, Ordering};

static PRNG_STATE: AtomicU32 = AtomicU32::new(0);

/// Initialize the PRNG state with a seed from the SysTick counter.
pub fn init() {
    // Use the current SysTick value as initial seed
    let seed = cortex_m::peripheral::SYST::get_current();
    PRNG_STATE.store(seed.wrapping_mul(1664525).wrapping_add(1013904223), Ordering::Relaxed);
}

/// Simple xorshift32 PRNG for generating random bytes.
fn next_u32() -> u32 {
    let mut state = PRNG_STATE.load(Ordering::Relaxed);
    state ^= state << 13;
    state ^= state >> 17;
    state ^= state << 5;
    PRNG_STATE.store(state, Ordering::Relaxed);
    state
}

/// Custom getrandom implementation for no_std targets.
fn custom_getrandom(buf: &mut [u8]) -> Result<(), getrandom::Error> {
    for chunk in buf.chunks_mut(4) {
        let random = next_u32().to_le_bytes();
        chunk.copy_from_slice(&random[..chunk.len()]);
    }
    Ok(())
}

getrandom::register_custom_getrandom!(custom_getrandom);
