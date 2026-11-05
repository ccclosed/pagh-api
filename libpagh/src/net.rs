//! A minimal TCP client over the ABI's BSD-socket subset (`docs/abi.md` §2).
//!
//! The kernel's network stack is one polled IPv4 TCP client: no listening
//! sockets, no UDP, no DNS, no TLS. This module exposes exactly that — an
//! [`Ipv4`] literal (or [`Ipv4::parse`]) plus [`TcpStream`], which is a
//! connected descriptor with `read`/`write`/`shutdown`.
//!
//! # The receive contract
//!
//! `recv` is **non-blocking**: it returns [`Errno::EAGAIN`] when no payload is
//! queued yet, and `Ok(0)` when the peer closed. [`TcpStream::read_to_end`]
//! owns the polling loop (yielding with `sched_yield` between attempts), so a
//! simple HTTP client can call it and get the whole response. The lower-level
//! [`io::Read`] implementation is for callers that have their own loop.
//!
//! # What is not here
//!
//! No `TcpListener`/`accept`, no address resolution, no socket options, no
//! timeouts configured by the program (the kernel's own bounds apply). The
//! kernel keeps one connection at a time, so a second [`TcpStream::connect`]
//! while the first is open fails; drop the first stream (its destructor closes
//! the descriptor) before connecting again.

use alloc::vec::Vec;

use crate::abi::SockAddrIn;
use crate::errno::{Errno, Result};
use crate::io::{Read, Write};
use crate::sys;

/// An IPv4 address: four octets in wire order.
///
/// This ABI has no name resolution; programs that need a host name resolve it
/// themselves (or accept a literal), and pass the address here. A literal can
/// be written with [`Ipv4::new`] or parsed with [`Ipv4::parse`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Ipv4(pub [u8; 4]);

impl Ipv4 {
    /// Build an address from its four octets, in wire order.
    pub const fn new(octets: [u8; 4]) -> Self {
        Self(octets)
    }

    /// The four octets, in wire order.
    pub const fn octets(self) -> [u8; 4] {
        self.0
    }

    /// Parse a dotted-quad literal such as `10.0.2.2`.
    ///
    /// Returns `None` for anything that is not exactly four decimal octets in
    /// `0..=255` (no host names, no IPv6, no shorthand forms). This mirrors the
    /// kernel's own `parse_ipv4_str`, which the shell's `http` command uses.
    pub fn parse(text: &str) -> Option<Self> {
        let mut octets = [0u8; 4];
        let mut count = 0usize;
        for part in text.split('.') {
            if count == octets.len() || part.is_empty() || part.len() > 3 {
                return None;
            }
            if !part.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            let value = part.parse::<u16>().ok()?;
            if value > u16::from(u8::MAX) {
                return None;
            }
            octets[count] = value as u8;
            count += 1;
        }
        if count == octets.len() {
            Some(Self(octets))
        } else {
            None
        }
    }
}

impl From<[u8; 4]> for Ipv4 {
    fn from(octets: [u8; 4]) -> Self {
        Self(octets)
    }
}

impl core::fmt::Display for Ipv4 {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}.{}.{}.{}", self.0[0], self.0[1], self.0[2], self.0[3])
    }
}

/// A connected TCP stream (one socket descriptor).
///
/// Dropping the value calls `close`; a close error is ignored, because there is
/// nothing useful to do with it and the kernel releases the connection anyway.
/// [`TcpStream::shutdown`] closes the connection but keeps the descriptor open
/// for a final `read` (which returns `Ok(0)`, end of stream).
#[derive(Debug)]
pub struct TcpStream {
    fd: i32,
    open: bool,
}

impl TcpStream {
    /// Open a connection to `host:port`, completing the TCP handshake.
    ///
    /// IPv4 literals only: there is no DNS in this ABI. The kernel's own
    /// deadline bounds the wait; a refusal is [`Errno::ECONNREFUSED`], no lease
    /// or no route [`Errno::ENETUNREACH`], a silent peer
    /// [`Errno::ETIMEDOUT`].
    pub fn connect(host: Ipv4, port: u16) -> Result<Self> {
        let fd = sys::socket(sys::AF_INET, sys::SOCK_STREAM, 0)?;
        let stream = Self { fd, open: true };
        // On error `stream` drops here, which closes the fresh descriptor
        // before the error reaches the caller.
        sys::connect(fd, &SockAddrIn::new(host.octets(), port))?;
        Ok(stream)
    }

    /// The underlying socket descriptor.
    pub const fn fd(&self) -> i32 {
        self.fd
    }

    /// Close the connection (both directions; there is no half-close) while
    /// keeping the descriptor. A later `read` returns `Ok(0)`, a later `write`
    /// is [`Errno::EPIPE`]; dropping the stream releases the descriptor.
    pub fn shutdown(&mut self) -> Result<()> {
        sys::shutdown(self.fd, sys::SHUT_RDWR)
    }

    /// Read until the peer closes, appending every payload chunk to `out`.
    ///
    /// `EAGAIN` is the normal "nothing queued yet" answer; the loop yields with
    /// `sched_yield` and retries, so this never spins without letting the
    /// kernel poll its NIC. Any other error, or running out of memory, ends the
    /// loop with that error.
    pub fn read_to_end(&mut self, out: &mut Vec<u8>) -> Result<()> {
        let mut buffer = [0u8; 1024];
        loop {
            match self.read(&mut buffer) {
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
}

impl Drop for TcpStream {
    fn drop(&mut self) {
        if self.open {
            let _ = sys::close(self.fd);
            self.open = false;
        }
    }
}

impl Read for TcpStream {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        sys::recv(self.fd, buf)
    }
}

impl Write for TcpStream {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        sys::send(self.fd, buf)
    }
}
