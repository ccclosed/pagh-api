//! BSD sockets over the ABI's IPv4 stack (`docs/abi.md` §2).
//!
//! The kernel is a polled IPv4 stack with one active-open TCP client, one TCP
//! listener with a bounded accept queue, and userland UDP sockets: no DNS, no
//! TLS. This module exposes an [`Ipv4`] literal (or [`Ipv4::parse`]),
//! [`TcpStream`] (a connected descriptor with `read`/`write`/`shutdown`),
//! [`TcpListener`] (bind + listen + accept) and [`UdpSocket`] (bind, send one
//! datagram, receive one datagram).
//!
//! # The receive contract
//!
//! `recv` is **non-blocking**: it returns [`Errno::EAGAIN`] when no payload is
//! queued yet, and `Ok(0)` when the peer closed. [`TcpStream::read_to_end`]
//! owns the polling loop (yielding with `sched_yield` between attempts), so a
//! simple HTTP client can call it and get the whole response. The lower-level
//! [`io::Read`] implementation is for callers that have their own loop.
//!
//! # UDP
//!
//! [`UdpSocket::bind`] creates an `AF_INET`/`SOCK_DGRAM` descriptor and binds a
//! local port; [`UdpSocket::send_to`] sends one datagram and
//! [`UdpSocket::recv_from`] waits for one (bounded by the kernel, with
//! `EAGAIN` if the wait expires). `UdpSocket::recv_from_nonblocking` is the
//! single-poll shape. The kernel caps a datagram at 1024 bytes
//! ([`crate::sys::UDP_DATAGRAM_MAX`]); a larger payload is `EMSGSIZE`.
//!
//! # What is not here
//!
//! No address resolution, no socket options, no timeouts configured by the
//! program (the kernel's own bounds apply). The active-open client is one
//! connection at a time, so a second [`TcpStream::connect`] while the first is
//! open fails; drop the first stream (its destructor closes the descriptor)
//! before connecting again. The listener is one per kernel with a four-entry
//! accept queue; a server can hold several accepted [`TcpStream`]s, up to the
//! kernel's eight-slot TCP table. UDP has no `connect`, no `send`/`recv` on a
//! connected datagram socket and no IPv6.

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
    /// The wildcard address `0.0.0.0` (bind to every local address).
    pub const ANY: Self = Self([0, 0, 0, 0]);

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

impl TcpStream {
    /// Wrap a descriptor the kernel just returned from `accept`.
    pub(crate) fn from_accepted(fd: i32) -> Self {
        Self { fd, open: true }
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

/// A listening TCP socket (one `AF_INET`/`SOCK_STREAM` descriptor).
///
/// [`TcpListener::bind`] performs the kernel's `bind(200)` and `listen(201)` in
/// one call. [`accept`](TcpListener::accept) returns the next handshaken
/// connection and its peer address; the kernel queues up to four of them
/// (clamped `listen` backlog), and a connection that arrives while the queue is
/// full is reset. Dropping the listener closes it and resets connections that
/// were never accepted; accepted [`TcpStream`]s keep their own descriptors.
#[derive(Debug)]
pub struct TcpListener {
    fd: i32,
    open: bool,
}

impl TcpListener {
    /// Create a stream socket, bind `0.0.0.0:port` and listen with backlog 4.
    ///
    /// Port 0 asks the kernel for an ephemeral port (there is no `getsockname`
    /// to learn it, so a server should pass an explicit port).
    pub fn bind(port: u16) -> Result<Self> {
        Self::bind_with_backlog(Ipv4::ANY, port, 4)
    }

    /// Bind `address:port` and listen with backlog 4.
    ///
    /// `address` must be `0.0.0.0` or the interface's DHCP lease; anything else
    /// is `EADDRNOTAVAIL`.
    pub fn bind_to(address: Ipv4, port: u16) -> Result<Self> {
        Self::bind_with_backlog(address, port, 4)
    }

    /// Bind `address:port` and listen with an explicit backlog.
    ///
    /// The kernel clamps `backlog` to `1..=4`, because the accept queue is a
    /// fixed four-entry array.
    pub fn bind_with_backlog(address: Ipv4, port: u16, backlog: u16) -> Result<Self> {
        Self::create(address, port, backlog, false)
    }

    /// Bind `0.0.0.0:port` and listen, with `accept` in non-blocking mode.
    ///
    /// An `accept` with an empty queue returns [`Errno::EAGAIN`] immediately
    /// instead of polling for the kernel's 30 s accept budget.
    pub fn bind_nonblocking(port: u16) -> Result<Self> {
        Self::create(Ipv4::ANY, port, 4, true)
    }

    fn create(address: Ipv4, port: u16, backlog: u16, nonblocking: bool) -> Result<Self> {
        let kind = if nonblocking {
            sys::SOCK_STREAM | sys::SOCK_NONBLOCK
        } else {
            sys::SOCK_STREAM
        };
        let fd = sys::socket(sys::AF_INET, kind, 0)?;
        let listener = Self { fd, open: true };
        // On error `listener` drops here and closes the fresh descriptor.
        sys::bind(fd, &SockAddrIn::new(address.octets(), port))?;
        sys::listen(fd, i32::from(backlog))?;
        Ok(listener)
    }

    /// The underlying socket descriptor.
    pub const fn fd(&self) -> i32 {
        self.fd
    }

    /// Wait for one connection, returning its stream and peer address.
    ///
    /// The wait polls the kernel's NIC for up to 30 s and then returns
    /// [`Errno::EAGAIN`]; a listener created with
    /// [`TcpListener::bind_nonblocking`] returns `EAGAIN` immediately when the
    /// queue is empty. The returned [`TcpStream`] owns its descriptor.
    pub fn accept(&mut self) -> Result<(TcpStream, SockAddrIn)> {
        let (fd, peer) = sys::accept(self.fd)?;
        Ok((TcpStream::from_accepted(fd), peer))
    }
}

impl Drop for TcpListener {
    fn drop(&mut self) {
        if self.open {
            let _ = sys::close(self.fd);
            self.open = false;
        }
    }
}

/// A bound UDP socket (one `AF_INET`/`SOCK_DGRAM` descriptor).
///
/// The socket is bound in [`UdpSocket::bind`]; `send_to` names the destination
/// on every datagram, and `recv_from` returns the source address. Dropping the
/// value closes the descriptor and releases the port.
#[derive(Debug)]
pub struct UdpSocket {
    fd: i32,
    open: bool,
}

impl UdpSocket {
    /// Create a UDP socket and bind `0.0.0.0:port`.
    ///
    /// Port 0 asks the kernel for an ephemeral port (`recv_from` still works,
    /// but the caller has no `getsockname` to learn the number; use an explicit
    /// port when the peer must reply).
    pub fn bind(port: u16) -> Result<Self> {
        Self::bind_to(Ipv4::new([0, 0, 0, 0]), port)
    }

    /// Create a UDP socket and bind `address:port`.
    ///
    /// `address` must be `0.0.0.0` or the interface's DHCP lease; anything else
    /// is `EADDRNOTAVAIL`. A port owned by another socket is `EADDRINUSE`.
    pub fn bind_to(address: Ipv4, port: u16) -> Result<Self> {
        let fd = sys::socket(sys::AF_INET, sys::SOCK_DGRAM, 0)?;
        let socket = Self { fd, open: true };
        // On error `socket` drops here, closing the fresh descriptor before the
        // error reaches the caller.
        sys::bind(fd, &SockAddrIn::new(address.octets(), port))?;
        Ok(socket)
    }

    /// The underlying descriptor.
    pub const fn fd(&self) -> i32 {
        self.fd
    }

    /// Send `buf` as one datagram to `host:port`.
    ///
    /// Returns the payload length on success. `EMSGSIZE` means the kernel's
    /// 1024-byte UDP payload limit was exceeded; no bytes were sent.
    pub fn send_to(&self, buf: &[u8], host: Ipv4, port: u16) -> Result<usize> {
        sys::sendto(self.fd, buf, &SockAddrIn::new(host.octets(), port))
    }

    /// Wait for one datagram, returning `(bytes, source address, source port)`.
    ///
    /// The wait is the kernel's bounded poll (about 10 s on the current
    /// kernel); `EAGAIN` means no datagram arrived, not an error in the
    /// socket. A datagram larger than `buf` is truncated by the kernel.
    pub fn recv_from(&mut self, buf: &mut [u8]) -> Result<(usize, Ipv4, u16)> {
        let (read, source) = sys::recvfrom(self.fd, buf, 0)?;
        Ok((read, Ipv4::new(source.address()), source.port()))
    }

    /// The single-poll shape of [`UdpSocket::recv_from`]:
    /// `EAGAIN` means nothing is queued right now.
    pub fn recv_from_nonblocking(&mut self, buf: &mut [u8]) -> Result<(usize, Ipv4, u16)> {
        let (read, source) = sys::recvfrom(self.fd, buf, sys::MSG_DONTWAIT)?;
        Ok((read, Ipv4::new(source.address()), source.port()))
    }
}

impl Drop for UdpSocket {
    fn drop(&mut self) {
        if self.open {
            let _ = sys::close(self.fd);
            self.open = false;
        }
    }
}
