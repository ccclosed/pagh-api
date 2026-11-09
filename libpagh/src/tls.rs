//! TLS 1.3 sessions over the native `tls_*` syscalls (`docs/abi.md` §2).
//!
//! [`TlsStream`] is the userland half of the kernel's TLS client. The
//! **handshake is the kernel's** — the same fail-closed one the shell's `https`
//! command runs: the server certificate must chain to the committed CA bundle,
//! the name passed to [`TlsStream::connect`] must be authorized by its SAN
//! entries, the clock gate must pass, and `CertificateVerify` must prove
//! possession of the leaf key. There is no flag here that weakens any of it,
//! because there is no flag in the ABI: `libpagh` cannot turn verification off,
//! and neither can a program that uses it.
//!
//! # Shape
//!
//! ```ignore
//! let socket = TcpStream::connect(ip, 443)?;      // the TCP client is yours
//! let mut tls = TlsStream::connect(socket, "example.com")?;
//! tls.write_all(b"GET / HTTP/1.1\r\nHost: example.com\r\n\r\n")?;
//! let mut chunk = [0u8; 4096];
//! loop {
//!     match tls.read(&mut chunk)? { 0 => break, n => { /* ... */ } }
//! }
//! tls.close()?;                                    // close_notify
//! ```
//!
//! [`TlsStream::connect`] consumes the [`TcpStream`]: after the handshake the
//! descriptor belongs to the TLS stream, and there is exactly one owner of it.
//!
//! # Reading
//!
//! [`TlsStream::read`] **blocks** (bounded by the kernel's idle budget) until at
//! least one byte is available; `Ok(0)` is a `close_notify` from the peer. A TCP
//! close *without* one is [`Errno::ECONNRESET`](crate::errno::Errno::ECONNRESET),
//! not `Ok(0)` — the layering is what makes truncation visible, and a caller that
//! treats every short stream as a complete response throws that away.
//!
//! # Closing
//!
//! [`TlsStream::close`] sends `close_notify` and releases the descriptor.
//! Dropping the value calls `close(2)` instead, which releases the session
//! without the alert: `Drop` runs at process teardown too and may not wait for a
//! peer. A program that cares about a graceful goodbye calls `close` explicitly.

use alloc::vec::Vec;

use crate::errno::{Errno, Result};
use crate::io::{Read, Write};
use crate::net::TcpStream;
use crate::sys;

/// A connected TCP socket with a completed, verified TLS 1.3 session on it.
///
/// One owner, one descriptor: created by [`TlsStream::connect`], closed by
/// [`TlsStream::close`] (graceful) or `Drop` (immediate).
#[derive(Debug)]
pub struct TlsStream {
    fd: i32,
    open: bool,
}

impl TlsStream {
    /// Perform the TLS 1.3 handshake on an already-connected `socket` and take
    /// ownership of the descriptor.
    ///
    /// `host` is what the server must prove it is: it goes out as SNI and the
    /// certificate's SAN entries are matched against it. It is **not** resolved
    /// or connected to — the caller did that, so a name can be tested against a
    /// deliberately pinned address (that is how the kernel's negative test
    /// works), and a program is free to use its own resolver.
    ///
    /// On error the connection is gone (the kernel abandons it, RFC 8446 §6.1)
    /// and `socket` has been consumed; the caller opens a new one to retry.
    pub fn connect(socket: TcpStream, host: &str) -> Result<Self> {
        // The socket's descriptor now belongs to this value; `into_fd` clears
        // the socket's own `Drop` so it cannot close what we are about to own.
        let fd = socket.into_fd();
        // From here on every error path must release `fd`, and `stream`'s Drop
        // does exactly that (the kernel already tore the connection down).
        let stream = Self { fd, open: true };
        sys::tls_connect(fd, host)?;
        Ok(stream)
    }

    /// The underlying descriptor, now carrying the TLS session.
    pub const fn fd(&self) -> i32 {
        self.fd
    }

    /// Send `close_notify` and stop writing, keeping the descriptor: a later
    /// `read` returns `Ok(0)` and a later `write` is [`Errno::EPIPE`].
    pub fn shutdown(&mut self) -> Result<()> {
        sys::shutdown(self.fd, sys::SHUT_RDWR)
    }

    /// Read until the peer sends `close_notify`, appending the plaintext to
    /// `out`.
    ///
    /// This is the right loop for a peer that closes the stream when it is done;
    /// a server that keeps the connection open (any HTTP/1.1 response with a
    /// body length) must be framed by the protocol instead — see
    /// `examples/https_get.rs`.
    ///
    /// [`Errno::EAGAIN`] means a concurrent call on the same session (another
    /// thread of this process); the loop yields and retries, like
    /// [`TcpStream::read_to_end`].
    pub fn read_to_end(&mut self, out: &mut Vec<u8>) -> Result<()> {
        let mut buffer = [0u8; 4096];
        loop {
            match sys::tls_read(self.fd, &mut buffer) {
                Ok(0) => return Ok(()),
                Ok(read) => {
                    out.try_reserve(read).map_err(|_| Errno::ENOMEM)?;
                    out.extend_from_slice(&buffer[..read]);
                }
                Err(Errno::EAGAIN) => {
                    let _ = sys::sched_yield();
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// Send `close_notify` (best effort), release the session buffers and close
    /// the descriptor. Consumes the value.
    pub fn close(mut self) -> Result<()> {
        self.open = false;
        sys::tls_close(self.fd)
    }
}

impl Drop for TlsStream {
    fn drop(&mut self) {
        if self.open {
            // `close(2)`, not `tls_close(1103)`: Drop must not block waiting for
            // a peer's alert, and the kernel releases the session either way.
            let _ = sys::close(self.fd);
            self.open = false;
        }
    }
}

impl Read for TlsStream {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        sys::tls_read(self.fd, buf)
    }
}

impl Write for TlsStream {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        sys::tls_write(self.fd, buf)
    }
}
