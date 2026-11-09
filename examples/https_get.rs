//! `https_get`: a real HTTPS/1.1 `GET` from a libpagh program.
//!
//! This is the example the TLS syscalls exist for: resolve a name, connect a
//! plain TCP socket, hand that socket to the kernel for a **fail-closed TLS 1.3
//! handshake**, send a request, read the response and print the body. Nothing
//! here parses certificates — the kernel does that, against the committed CA
//! bundle, before a single application byte moves — and nothing here can turn
//! verification off.
//!
//! ```sh
//! # in the guest shell (QEMU user network: 10.0.2.2 is the host, 10.0.2.3 DNS)
//! run /mnt/https_get
//! run /mnt/https_get example.com /
//! run /mnt/https_get example.com / 10.0.2.3
//! ```
//!
//! Build with:
//!
//! ```sh
//! cargo build --release --example https_get --target riscv64gc-unknown-none-elf
//! ```
//!
//! # What it demonstrates, and what it does not
//!
//! * [`libpagh::dns`] resolves the name (one `A` query, UDP, no DNSSEC);
//! * [`libpagh::net::TcpStream`] owns the connection until
//!   [`libpagh::tls::TlsStream::connect`] takes it over;
//! * [`libpagh::tls::TlsStream`] reads and writes plaintext; the record layer
//!   is the kernel's;
//! * `close_notify` is sent by `TlsStream::close`, and a stream that ends
//!   **without** it is `ECONNRESET`, not end-of-file — so a truncated body
//!   cannot masquerade as a complete one.
//!
//! The HTTP framing is this program's business, not the TLS layer's. It asks for
//! `Connection: close` and understands the three endings HTTP/1.x defines:
//! `Content-Length`, `Transfer-Encoding: chunked` (the example's own decoder,
//! including the terminal chunk), and close-delimited (no length: the body ends
//! with the connection, and an abrupt close is *not* distinguishable from a
//! complete body — the limitation of that framing, stated rather than hidden).
//!
//! Exit codes: 0 success, 1 usage/resolve, 2 TCP connect, 3 TLS handshake
//! (including a rejected certificate), 4 request write, 5 response framing,
//! 6 the peer's status was not 200, 7 the body exceeded the cap.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

use libpagh::io::{Read, Write};
use libpagh::net::{Ipv4, TcpStream};
use libpagh::tls::TlsStream;
use libpagh::{dns, println};

/// HTTPS port. This example speaks TLS 1.3 only, so there is no plaintext mode.
const HTTPS_PORT: u16 = 443;

/// Upper bound on the response this program will buffer, head included.
const MAX_RESPONSE: usize = 8 * 1024 * 1024;

/// Default target: the kernel's own live-proof host, which chains to a root in
/// the committed CA bundle.
const DEFAULT_HOST: &str = "example.com";

#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8, _envp: *const *const u8) -> i32 {
    let args: Vec<String> = libpagh::env::args().collect();
    let host = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| String::from(DEFAULT_HOST));
    let path = args.get(2).cloned().unwrap_or_else(|| String::from("/"));
    let server = match args.get(3) {
        Some(text) => match Ipv4::parse(text) {
            Some(ip) => ip,
            None => {
                println!("https_get: resolver {text:?} is not an IPv4 literal");
                return 1;
            }
        },
        None => dns::DEFAULT_SERVER,
    };

    println!("https_get: {host}{path} via resolver {server}");

    // 1. Resolve. The TLS handshake authenticates the *name*, so a lie here is
    // caught there: a server that is not authorized for `host` is refused.
    let address = match dns::resolve(&host, server) {
        Ok(address) => address,
        Err(error) => {
            println!("https_get: FAILED stage=resolve ({error})");
            return 1;
        }
    };
    println!("https_get: {host} -> {address}");

    // 2. Plain TCP first; TLS is layered on the connected descriptor.
    let socket = match TcpStream::connect(address, HTTPS_PORT) {
        Ok(socket) => socket,
        Err(error) => {
            println!("https_get: FAILED stage=connect ({error})");
            return 2;
        }
    };
    println!("https_get: tcp connected to {address}:{HTTPS_PORT}");

    // 3. The handshake. Every failure here is a refusal: `EKEYREJECTED` is the
    // certificate, `EPROTO` the protocol, `EAGAIN`/`ENOTSUP` a kernel with no
    // entropy seed or no clock.
    let mut tls = match TlsStream::connect(socket, &host) {
        Ok(tls) => tls,
        Err(error) => {
            println!("https_get: FAILED stage=tls ({error})");
            return 3;
        }
    };
    println!("https_get: TLS 1.3 session up (server authenticated by the kernel)");

    let request = build_request(&host, &path);
    if let Err(error) = tls.write_all(request.as_bytes()) {
        println!("https_get: FAILED stage=write ({error})");
        return 4;
    }

    let body = match read_response(&mut tls) {
        Ok(body) => body,
        Err(ResponseError::Errno(error)) => {
            println!("https_get: FAILED stage=response ({error})");
            return 5;
        }
        Err(ResponseError::Status(status)) => {
            println!("https_get: FAILED stage=response (HTTP status {status})");
            return 6;
        }
        Err(ResponseError::TooLarge) => {
            println!("https_get: FAILED stage=response (body over {MAX_RESPONSE} bytes)");
            return 7;
        }
        Err(ResponseError::Malformed) => {
            println!("https_get: FAILED stage=response (malformed head or chunked framing)");
            return 5;
        }
    };

    let _ = tls.close();
    println!("https_get: ----- body ({} bytes) -----", body.len());
    let mut stdout = libpagh::io::Stdout;
    if stdout.write_all(&body).is_err() {
        return 5;
    }
    println!("\nhttps_get: ----- end body -----");
    println!("https_get: OK");
    0
}

/// The request line and headers. `Connection: close` keeps the response's end
/// unambiguous for a server that would otherwise keep the connection open, and
/// the `User-Agent` names what is asking (there is exactly one).
fn build_request(host: &str, path: &str) -> String {
    let mut request = String::new();
    request.push_str("GET ");
    request.push_str(path);
    request.push_str(" HTTP/1.1\r\nHost: ");
    request.push_str(host);
    request
        .push_str("\r\nUser-Agent: libpagh/https_get\r\nAccept: */*\r\nConnection: close\r\n\r\n");
    request
}

/// Why a response could not be turned into a body.
enum ResponseError {
    /// A transport/record failure (including a truncated stream).
    Errno(libpagh::Errno),
    /// The status line was not 200.
    Status(u16),
    /// The head or the chunked framing did not parse.
    Malformed,
    /// The response passed `MAX_RESPONSE`.
    TooLarge,
}

/// Read the response head, then the body it declares.
///
/// Three framings are understood, in the order HTTP/1.x defines them:
/// `Transfer-Encoding: chunked` (decoded here, terminal chunk required),
/// `Content-Length` (the body must contain that many bytes — a short stream is
/// an error, not a shorter body), and neither (close-delimited: the body ends
/// when the peer closes, and an abrupt close is indistinguishable from a
/// complete body — the honest limitation of that framing).
fn read_response(tls: &mut TlsStream) -> Result<Vec<u8>, ResponseError> {
    let mut raw: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 4096];
    let mut head: Option<Head> = None;

    loop {
        match tls.read(&mut chunk) {
            Ok(0) => break, // close_notify: the peer is done
            Ok(read) => {
                raw.try_reserve(read).map_err(|_| ResponseError::TooLarge)?;
                raw.extend_from_slice(&chunk[..read]);
                if raw.len() > MAX_RESPONSE {
                    return Err(ResponseError::TooLarge);
                }
            }
            // A TCP close without `close_notify`: TLS calls that truncation.
            // It is acceptable here only when the framing already says the body
            // is complete, which the checks below decide.
            Err(libpagh::Errno::ECONNRESET) => break,
            Err(error) => return Err(ResponseError::Errno(error)),
        }

        if head.is_none() {
            match parse_head(&raw)? {
                Some(parsed) => head = Some(parsed),
                None => continue, // head still incomplete
            }
        }
        // Stop as soon as a self-delimiting framing is satisfied, instead of
        // waiting for a close the server may never send.
        let parsed = head.as_ref().expect("head was just set");
        match parsed.framing {
            Framing::Chunked { body_off } => {
                if chunked_body(&raw[body_off..])?.is_some() {
                    break;
                }
            }
            Framing::Length { body_off, length } => {
                if raw.len() >= body_off + length {
                    break;
                }
            }
            Framing::UntilClose { .. } => {}
        }
    }

    let parsed = match head {
        Some(parsed) => parsed,
        None => return Err(ResponseError::Malformed),
    };
    if parsed.status != 200 {
        return Err(ResponseError::Status(parsed.status));
    }
    match parsed.framing {
        Framing::Chunked { body_off } => match chunked_body(&raw[body_off..])? {
            Some(body) => Ok(body),
            // The peer closed mid-chunk: the terminal chunk never arrived.
            None => Err(ResponseError::Malformed),
        },
        Framing::Length { body_off, length } => {
            let end = body_off + length;
            if raw.len() < end {
                // Short body: the framing says how much was expected, so this
                // is a truncation the close-delimited case cannot detect.
                return Err(ResponseError::Malformed);
            }
            Ok(raw[body_off..end].to_vec())
        }
        Framing::UntilClose { body_off } => Ok(raw[body_off..].to_vec()),
    }
}

/// The status line and the body framing of a parsed head.
struct Head {
    status: u16,
    framing: Framing,
}

/// How the body's end is determined.
enum Framing {
    /// `Transfer-Encoding: chunked`: the body starts at `body_off`.
    Chunked { body_off: usize },
    /// `Content-Length: length`: the body starts at `body_off`.
    Length { body_off: usize, length: usize },
    /// Neither header: the body ends when the connection does.
    UntilClose { body_off: usize },
}

/// Parse the response head, returning `None` while it is still incomplete.
///
/// A head ends at the first `\r\n\r\n`; a line that is not `Name: value` is
/// malformed. Only a few headers matter here — the framing ones — and the rest
/// are skipped, which is what makes this small rather than a general parser.
fn parse_head(raw: &[u8]) -> Result<Option<Head>, ResponseError> {
    let Some(end) = find(raw, b"\r\n\r\n") else {
        // A head larger than this is not a head this program wants to read.
        return if raw.len() > 64 * 1024 {
            Err(ResponseError::Malformed)
        } else {
            Ok(None)
        };
    };
    let head = &raw[..end];
    let body_off = end + 4;

    let mut lines = head.split(|byte| *byte == b'\n');
    let Some(status_line) = lines.next() else {
        return Err(ResponseError::Malformed);
    };
    let status_line = trim_cr(status_line);
    // `HTTP/1.x SP status SP reason`
    let mut parts = status_line.split(|byte| *byte == b' ');
    let version = parts.next().ok_or(ResponseError::Malformed)?;
    if !version.starts_with(b"HTTP/1.") {
        return Err(ResponseError::Malformed);
    }
    let status =
        parse_u16(parts.next().ok_or(ResponseError::Malformed)?).ok_or(ResponseError::Malformed)?;

    let mut content_length: Option<usize> = None;
    let mut chunked = false;
    for line in lines {
        let line = trim_cr(line);
        if line.is_empty() {
            continue;
        }
        let Some(colon) = line.iter().position(|byte| *byte == b':') else {
            return Err(ResponseError::Malformed);
        };
        let name = &line[..colon];
        let value = trim_spaces(&line[colon + 1..]);
        if eq_ignore_case(name, b"content-length") {
            content_length = Some(parse_usize(value).ok_or(ResponseError::Malformed)?);
        } else if eq_ignore_case(name, b"transfer-encoding") {
            chunked = contains_ignore_case(value, b"chunked");
        }
    }

    // `Transfer-Encoding` wins over `Content-Length` (RFC 9112 §6.3): when both
    // are present the message is chunked, and believing the length instead would
    // be a request-smuggling-shaped mistake.
    let framing = if chunked {
        Framing::Chunked { body_off }
    } else if let Some(length) = content_length {
        Framing::Length { body_off, length }
    } else {
        Framing::UntilClose { body_off }
    };
    Ok(Some(Head { status, framing }))
}

/// Decode a `Transfer-Encoding: chunked` body.
///
/// `Ok(Some(body))` once the zero-length terminal chunk arrived,
/// `Ok(None)` while more bytes are needed, `Err` on malformed framing. Chunk
/// extensions are skipped; the trailer section is ignored (there is no use for
/// it here, and RFC 9112 allows a recipient to do that).
fn chunked_body(data: &[u8]) -> Result<Option<Vec<u8>>, ResponseError> {
    let mut body = Vec::new();
    let mut offset = 0usize;
    loop {
        // Chunk size line: hex digits, optionally `;extensions`, then CRLF.
        let Some(line_end) = find(&data[offset..], b"\r\n") else {
            return Ok(None);
        };
        let line = &data[offset..offset + line_end];
        let digits = match line.iter().position(|byte| *byte == b';') {
            Some(semicolon) => &line[..semicolon],
            None => line,
        };
        let size = parse_hex(digits).ok_or(ResponseError::Malformed)?;
        offset += line_end + 2;
        if size == 0 {
            return Ok(Some(body));
        }
        if offset + size + 2 > data.len() {
            return Ok(None);
        }
        body.try_reserve(size)
            .map_err(|_| ResponseError::TooLarge)?;
        body.extend_from_slice(&data[offset..offset + size]);
        offset += size;
        if &data[offset..offset + 2] != b"\r\n" {
            return Err(ResponseError::Malformed);
        }
        offset += 2;
    }
}

// ─── small byte helpers (no `std`, no regex, no panic) ─────────────────────

/// Index of the first occurrence of `needle` in `haystack`.
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Drop one trailing `\r`.
fn trim_cr(line: &[u8]) -> &[u8] {
    match line.last() {
        Some(b'\r') => &line[..line.len() - 1],
        _ => line,
    }
}

/// Drop leading and trailing spaces and tabs.
fn trim_spaces(value: &[u8]) -> &[u8] {
    let mut start = 0usize;
    let mut end = value.len();
    while start < end && (value[start] == b' ' || value[start] == b'\t') {
        start += 1;
    }
    while end > start && (value[end - 1] == b' ' || value[end - 1] == b'\t') {
        end -= 1;
    }
    &value[start..end]
}

/// ASCII case-insensitive equality, overflow-free over non-ASCII bytes.
fn eq_ignore_case(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(a, b)| a.to_ascii_lowercase() == b.to_ascii_lowercase())
}

/// Does `haystack` contain `needle`, ASCII case-insensitively?
fn contains_ignore_case(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.len() > haystack.len() {
        return false;
    }
    (0..=haystack.len() - needle.len())
        .any(|start| eq_ignore_case(&haystack[start..start + needle.len()], needle))
}

/// Decimal, refusing anything that is not all digits (including empty input).
fn parse_usize(text: &[u8]) -> Option<usize> {
    if text.is_empty() {
        return None;
    }
    let mut value = 0usize;
    for byte in text {
        if !byte.is_ascii_digit() {
            return None;
        }
        value = value.checked_mul(10)?.checked_add((byte - b'0') as usize)?;
    }
    Some(value)
}

/// Same, capped at `u16` for the status code.
fn parse_u16(text: &[u8]) -> Option<u16> {
    let value = parse_usize(text)?;
    u16::try_from(value).ok()
}

/// Hex, refusing an empty or non-hex chunk size.
fn parse_hex(text: &[u8]) -> Option<usize> {
    if text.is_empty() {
        return None;
    }
    let mut value = 0usize;
    for byte in text {
        let digit = match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            b'A'..=b'F' => byte - b'A' + 10,
            _ => return None,
        };
        value = value.checked_mul(16)?.checked_add(digit as usize)?;
    }
    Some(value)
}
