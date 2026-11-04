//! `print!`/`println!`/`eprintln!` and the panic/OOM diagnostics.
//!
//! Output goes straight to fd 1/2 with the `write` syscall; there is no line
//! buffering and no allocation on the formatting path. A failed console write
//! is ignored by the macros (there is nowhere to report it), but the
//! `Write`-based helpers return the error so `cat` can exit non-zero.

use core::fmt;

use crate::errno::Errno;
use crate::sys;

/// Write every byte of `buf` to `fd`, retrying `EAGAIN` with `sched_yield`.
///
/// `EAGAIN` is documented for `read(0)`, not for console writes, but retrying
/// is harmless and keeps a future bounded tty queue from turning `println!`
/// into a truncated line. A zero-byte `write` is treated as `EIO` so the loop
/// cannot spin forever.
pub(crate) fn write_all_fd(fd: i32, mut buf: &[u8]) -> Result<(), Errno> {
    while !buf.is_empty() {
        match sys::write(fd, buf) {
            Ok(0) => return Err(Errno::EIO),
            Ok(n) => {
                if n > buf.len() {
                    return Err(Errno::EIO);
                }
                buf = &buf[n..];
            }
            Err(Errno::EAGAIN) => {
                let _ = sys::sched_yield();
            }
            Err(err) => return Err(err),
        }
    }
    Ok(())
}

/// A `core::fmt::Write` sink on one file descriptor.
pub(crate) struct FdWriter(pub(crate) i32);

impl fmt::Write for FdWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        write_all_fd(self.0, s.as_bytes()).map_err(|_| fmt::Error)
    }
}

/// Format `args` to `fd`, ignoring formatting/write errors.
pub(crate) fn write_fmt_fd(fd: i32, args: fmt::Arguments<'_>) -> fmt::Result {
    let mut writer = FdWriter(fd);
    fmt::write(&mut writer, args)
}

/// Backing function for `print!`; not part of the stable surface.
pub fn _print(args: fmt::Arguments<'_>) {
    let _ = write_fmt_fd(1, args);
}

/// Backing function for `eprint!`; not part of the stable surface.
pub fn _eprint(args: fmt::Arguments<'_>) {
    let _ = write_fmt_fd(2, args);
}