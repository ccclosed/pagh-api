//! A minimal DNS `A`-record resolver over a UDP socket.
//!
//! The ABI's socket layer has no `getaddrinfo` and the kernel's own resolver
//! (`net::resolve_name`) is not reachable from userland, so a program that wants
//! to talk to a *name* — which is what a TLS certificate is issued for — sends
//! the query itself. This module is that query: one `A` lookup, one datagram
//! reply, no caching, no `CNAME` chasing, no `AAAA`, no truncation retry.
//!
//! # What it is, honestly
//!
//! * **UDP only, one question per call.** A reply with `TC` set (truncated) is an
//!   error rather than a silent retry over TCP.
//! * **No DNSSEC and no validation.** The resolver tells you what the configured
//!   server said. That is exactly why the TLS layer authenticates the *name*
//!   afterwards: an attacker who controls DNS can point the socket at another
//!   server, and the handshake then fails closed because that server cannot
//!   present a certificate for the requested name.
//! * **`CNAME` answers are followed only when the record itself is an `A`.** A
//!   reply whose answer section is a `CNAME` chain without an `A` record is an
//!   error; `example.com`'s zone answers directly, and pretending to be a full
//!   resolver would be worse than saying no.
//!
//! # The server address
//!
//! QEMU's user-mode network forwards DNS at `10.0.2.3` (the same network where
//! `10.0.2.2` is the host's loopback and `10.0.2.15` is the guest). The DHCP
//! lease carries the real server, but the ABI has no `getsockopt(IP_DNS)` or
//! `/etc/resolv.conf`, so the address is a parameter: [`DEFAULT_SERVER`] is what
//! a QEMU guest needs and `examples/https_get.rs` takes an override.

use alloc::vec::Vec;

use crate::errno::{Errno, Result};
use crate::net::{Ipv4, UdpSocket};

/// The resolver address QEMU's user network forwards to the host's DNS.
pub const DEFAULT_SERVER: Ipv4 = Ipv4::new([10, 0, 2, 3]);

/// DNS port.
pub const PORT: u16 = 53;

/// Longest name accepted, in wire form: the protocol's own limit is 255 bytes.
const NAME_MAX: usize = 255;

/// One query, one reply, one address.
///
/// Returns the first `A` record in the answer section. The errors are:
/// `EAGAIN` — the reply never arrived (the kernel's bounded UDP wait);
/// `EINVAL` — the name is not one this resolver will send (empty, too long, a
/// label over 63 bytes, non-ASCII); [`Errno::EPROTO`] — the reply does not
/// answer the question that was asked (a different transaction id, an `RCODE`,
/// the `TC` truncation bit, a `CNAME`-only answer) or is malformed.
pub fn resolve(host: &str, server: Ipv4) -> Result<Ipv4> {
    let query = build_query(host, query_id()?)?;
    let mut socket = UdpSocket::bind(0)?;
    socket.send_to(&query, server, PORT)?;
    let mut reply = [0u8; 512];
    let (len, _, _) = socket.recv_from(&mut reply)?;
    parse_answer(&reply[..len], &query)
}

/// A transaction id for one query.
///
/// From the kernel entropy pool when it is available; a missing pool is not a
/// reason to refuse a *lookup* (the answer is authenticated by the TLS name
/// check, not by this id), so an unavailable pool falls back to the clock. The
/// id only has to be unlikely to collide with another outstanding query on a
/// socket this program owns.
fn query_id() -> Result<u16> {
    match crate::random::u64() {
        Ok(value) => Ok((value as u16) ^ ((value >> 16) as u16)),
        Err(_) => {
            let mut now = crate::abi::Timespec::default();
            crate::sys::clock_gettime(crate::sys::CLOCK_MONOTONIC, &mut now)?;
            Ok(now.nanos as u16)
        }
    }
}

/// Build a standard recursive query for `host`'s `A` record.
///
/// The question is `host` encoded as DNS labels, then `QType = A (1)` and
/// `QClass = IN (1)`. `host` must be a dotted name of ASCII labels, each at most
/// 63 bytes and the whole name at most [`NAME_MAX`]; anything else is `EINVAL`
/// rather than a malformed packet.
pub fn build_query(host: &str, id: u16) -> Result<Vec<u8>> {
    let mut wire = Vec::with_capacity(host.len() + 18);
    if host.is_empty() || host.len() > NAME_MAX {
        return Err(Errno::EINVAL);
    }
    // Header: id, flags = RD, one question, no answers/authority/additional.
    wire.extend_from_slice(&id.to_be_bytes());
    wire.extend_from_slice(&0x0100u16.to_be_bytes());
    wire.extend_from_slice(&1u16.to_be_bytes());
    wire.extend_from_slice(&0u16.to_be_bytes());
    wire.extend_from_slice(&0u16.to_be_bytes());
    wire.extend_from_slice(&0u16.to_be_bytes());
    // Question: labels, root, QType A, QClass IN.
    for label in host.split('.') {
        if label.is_empty() || label.len() > 63 || !label.is_ascii() {
            return Err(Errno::EINVAL);
        }
        wire.push(label.len() as u8);
        wire.extend_from_slice(label.as_bytes());
    }
    wire.push(0);
    wire.extend_from_slice(&1u16.to_be_bytes());
    wire.extend_from_slice(&1u16.to_be_bytes());
    Ok(wire)
}

/// Read the first `A` record out of `reply`, which must be the answer to
/// `query`.
pub fn parse_answer(reply: &[u8], query: &[u8]) -> Result<Ipv4> {
    if reply.len() < 12 {
        return Err(Errno::EPROTO);
    }
    // The transaction id must match the query: a reply for someone else's
    // question must never be believed.
    if reply[0..2] != query[0..2] {
        return Err(Errno::EPROTO);
    }
    // QR must be set (a response) and RCODE must be 0 (no error).
    if reply[2] & 0x80 == 0 || reply[3] & 0x0f != 0 {
        return Err(Errno::EPROTO);
    }
    // Truncated replies are refused, not retried over TCP.
    if reply[2] & 0x02 != 0 {
        return Err(Errno::EPROTO);
    }
    let questions = u16::from_be_bytes([reply[4], reply[5]]) as usize;
    let answers = u16::from_be_bytes([reply[6], reply[7]]) as usize;
    if questions != 1 || answers == 0 {
        return Err(Errno::EPROTO);
    }

    // Skip the question section rather than assuming it mirrors the query: the
    // reply's own labels are what the offsets below are relative to.
    let mut offset = 12usize;
    offset = skip_name(reply, offset)?;
    offset = offset.checked_add(4).ok_or(Errno::EPROTO)?; // QType + QClass
    if offset > reply.len() {
        return Err(Errno::EPROTO);
    }

    for _ in 0..answers {
        offset = skip_name(reply, offset)?;
        // TYPE(2) CLASS(2) TTL(4) RDLENGTH(2)
        if offset + 10 > reply.len() {
            return Err(Errno::EPROTO);
        }
        let rtype = u16::from_be_bytes([reply[offset], reply[offset + 1]]);
        let rdlength = u16::from_be_bytes([reply[offset + 8], reply[offset + 9]]) as usize;
        offset += 10;
        if offset + rdlength > reply.len() {
            return Err(Errno::EPROTO);
        }
        if rtype == 1 {
            if rdlength != 4 {
                return Err(Errno::EPROTO);
            }
            return Ok(Ipv4::new([
                reply[offset],
                reply[offset + 1],
                reply[offset + 2],
                reply[offset + 3],
            ]));
        }
        offset += rdlength;
    }
    // Only CNAMEs (or other record types): this resolver does not chase them.
    Err(Errno::EPROTO)
}

/// Advance past a (possibly compressed) name and return the offset after it.
///
/// A compression pointer terminates the name: it is two bytes and the rest of
/// the name lives elsewhere in the packet, which this walk does not need to
/// follow to stay in sync.
fn skip_name(reply: &[u8], mut offset: usize) -> Result<usize> {
    loop {
        let Some(&length) = reply.get(offset) else {
            return Err(Errno::EPROTO);
        };
        if length == 0 {
            return Ok(offset + 1);
        }
        if length & 0xc0 == 0xc0 {
            // Two-byte pointer; the target is not followed here.
            return if offset + 2 <= reply.len() {
                Ok(offset + 2)
            } else {
                Err(Errno::EPROTO)
            };
        }
        if length > 63 {
            return Err(Errno::EPROTO);
        }
        offset = offset
            .checked_add(1 + length as usize)
            .ok_or(Errno::EPROTO)?;
        if offset > reply.len() {
            return Err(Errno::EPROTO);
        }
    }
}
