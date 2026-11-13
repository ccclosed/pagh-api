//! `Read`/`Write`/`Seek` traits and the three standard descriptors.
//!
//! The traits are deliberately tiny: one syscall per method, errors are
//! [`Errno`], no buffering layer and no `std::io` compatibility. `File`,
//! `Stdin`, `Stdout` and `Stderr` implement them, and [`stdin`], [`stdout`]
//! and [`stderr`] return the console handles (std-shaped names; the handles
//! are copyable markers, not locked globals).
//!
//! `EAGAIN` is **not** retried here. It is returned to the caller because
//! `read(0)` uses it to mean "no byte is queued yet" (`docs/abi.md` §2); the
//! console-reading applets loop with `sched_yield` around it, which is the only
//! way to be a good citizen on a single-CPU kernel without a `poll` syscall.

use alloc::vec::Vec;

use crate::errno::{Errno, Result};
use crate::sys;

/// A byte source.
pub trait Read {
    /// Read up to `buf.len()` bytes. `Ok(0)` means end of file.
    fn read(&mut self, buf: &mut [u8]) -> Result<usize>;

    /// Read until end of file into a freshly allocated `Vec`.
    ///
    /// Uses `try_reserve` so running out of memory is `ENOMEM` rather than an
    /// allocator abort. `EAGAIN` is propagated, not retried.
    fn read_to_end(&mut self) -> Result<Vec<u8>> {
        const CHUNK: usize = 4096;
        let mut out = Vec::new();
        let mut buf = [0u8; CHUNK];
        loop {
            let n = self.read(&mut buf)?;
            if n == 0 {
                return Ok(out);
            }
            if n > buf.len() {
                // A kernel returning more bytes than the buffer is broken.
                return Err(Errno::EIO);
            }
            out.try_reserve(n).map_err(|_| Errno::ENOMEM)?;
            out.extend_from_slice(&buf[..n]);
        }
    }
}

/// A byte sink.
pub trait Write {
    /// Write up to `buf.len()` bytes; a short write is possible.
    fn write(&mut self, buf: &[u8]) -> Result<usize>;

    /// Flush buffered data. These writers are unbuffered, so this is a no-op
    /// unless a future writer overrides it.
    fn flush(&mut self) -> Result<()> {
        Ok(())
    }

    /// Write all of `buf`, looping over short writes.
    fn write_all(&mut self, mut buf: &[u8]) -> Result<()> {
        while !buf.is_empty() {
            let n = self.write(buf)?;
            if n == 0 {
                // No progress: report an I/O error instead of looping forever.
                return Err(Errno::EIO);
            }
            if n > buf.len() {
                return Err(Errno::EIO);
            }
            buf = &buf[n..];
        }
        Ok(())
    }
}

/// Where [`Seek::seek`] starts counting from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeekFrom {
    /// Absolute byte offset from the start.
    Start(u64),
    /// Offset relative to the current position (may be negative).
    Current(i64),
    /// Offset relative to the end (may be negative).
    End(i64),
}

/// A seekable byte sink/source (regular files).
pub trait Seek {
    /// Move the offset and return the new absolute offset.
    fn seek(&mut self, pos: SeekFrom) -> Result<u64>;
}

/// Console input, fd 0.
///
/// `read` returns `EAGAIN` while no byte is queued; see the module docs.
#[derive(Clone, Copy, Debug, Default)]
pub struct Stdin;

/// Console output, fd 1.
#[derive(Clone, Copy, Debug, Default)]
pub struct Stdout;

/// Console diagnostics, fd 2.
#[derive(Clone, Copy, Debug, Default)]
pub struct Stderr;

impl Stdin {
    /// A reader for fd 0.
    pub const fn new() -> Self {
        Self
    }
}

/// A reader for fd 0 (std-shaped `io::stdin()`; the handle is a copyable
/// marker, so unlike std there is no global handle to lock).
pub const fn stdin() -> Stdin {
    Stdin
}

/// A writer for fd 1.
pub const fn stdout() -> Stdout {
    Stdout
}

/// A writer for fd 2.
pub const fn stderr() -> Stderr {
    Stderr
}

impl Stdout {
    /// A writer for fd 1.
    pub const fn new() -> Self {
        Self
    }
}

impl Stderr {
    /// A writer for fd 2.
    pub const fn new() -> Self {
        Self
    }
}

impl Read for Stdin {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        sys::read(0, buf)
    }
}

impl Write for Stdout {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        sys::write(1, buf)
    }
}

impl Write for Stderr {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        sys::write(2, buf)
    }
}

/// Copy `reader` to `writer` until EOF, returning the number of bytes copied.
pub fn copy<R: Read + ?Sized, W: Write + ?Sized>(
    reader: &mut R,
    writer: &mut W,
) -> Result<u64> {
    let mut buf = [0u8; 4096];
    let mut total: u64 = 0;
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            return Ok(total);
        }
        if n > buf.len() {
            return Err(Errno::EIO);
        }
        writer.write_all(&buf[..n])?;
        total = total
            .checked_add(n as u64)
            .ok_or(Errno::EOVERFLOW)?;
    }
}