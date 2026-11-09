//! Random bytes from the kernel entropy pool (`getrandom`, `docs/abi.md` §2).
//!
//! One source, one contract: the kernel's SHA-256 pool is seeded from the device
//! tree at boot and hands bytes to both userland and the kernel's own TLS client,
//! so a program that needs a key, a nonce or a salt asks here instead of
//! inventing something from the clock.
//!
//! # Fail closed
//!
//! [`fill`] returns [`Errno::EAGAIN`] when the pool has no seed. That is not a
//! transient condition to paper over: the kernel refuses to invent randomness,
//! and a caller that substitutes a counter would be building the exact weakness
//! the pool exists to avoid. Treat it as fatal for anything cryptographic.
//!
//! # Short reads
//!
//! One `getrandom` call returns at most [`sys::GETRANDOM_MAX`] bytes; [`fill`]
//! loops over the cap, so a large buffer is filled in one call to this module.
//!
//! ```ignore
//! let mut key = [0u8; 32];
//! random::fill(&mut key)?;
//! ```

use crate::errno::{Errno, Result};
use crate::sys;

/// Fill `buf` completely from the kernel entropy pool.
///
/// Returns [`Errno::EAGAIN`] when the pool was never seeded; the partial
/// contents of `buf` are unspecified in that case (the kernel writes nothing,
/// so the caller's buffer keeps whatever it held).
pub fn fill(buf: &mut [u8]) -> Result<()> {
    let mut written = 0usize;
    while written < buf.len() {
        match sys::getrandom(&mut buf[written..], 0) {
            Ok(0) => return Err(Errno::EIO),
            Ok(n) => written += n,
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// One 64-bit value from the pool, for identifiers and jitter.
///
/// Not for keys: a key needs a documented width and [`fill`] into an array of
/// exactly that width.
pub fn u64() -> Result<u64> {
    let mut bytes = [0u8; 8];
    fill(&mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}
