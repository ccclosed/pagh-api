//! UDP echo probe over libpagh's [`UdpSocket`] — the native UDP acceptance
//! fixture.
//!
//! It binds `0.0.0.0:45678` (override with the third argument), sends one
//! datagram to `10.0.2.2:9000` (override with the first two arguments), and
//! waits for the echo. QEMU's user network maps `10.0.2.2` to the host's
//! loopback, so a host-side echo server is enough:
//!
//! ```sh
//! python3 tools/udp_echo_server.py 9000          # from the kernel checkout
//! # in the guest shell:
//! run /mnt/udp_echo 10.0.2.2 9000 45678
//! ```
//!
//! Build with:
//!
//! ```sh
//! cargo build --release --example udp_echo --target riscv64gc-unknown-none-elf
//! ```
//!
//! Exit codes: 0 success, 1 bind, 2 send, 3 receive, 4 payload mismatch.

#![no_std]
#![no_main]

use libpagh::env;
use libpagh::net::{Ipv4, UdpSocket};
use libpagh::println;

const DEFAULT_HOST: Ipv4 = Ipv4::new([10, 0, 2, 2]);
const DEFAULT_PORT: u16 = 9000;
const DEFAULT_LOCAL_PORT: u16 = 45678;
const PAYLOAD: &[u8] = b"pagh udp echo";

#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8, _envp: *const *const u8) -> i32 {
    let mut args = env::args().skip(1);
    let host = match args.next() {
        Some(text) => match Ipv4::parse(&text) {
            Some(host) => host,
            None => {
                println!("udp: bad IPv4 literal {text:?}");
                return 1;
            }
        },
        None => DEFAULT_HOST,
    };
    let port = match args.next() {
        Some(text) => match text.parse::<u16>() {
            Ok(port) if port != 0 => port,
            _ => {
                println!("udp: bad port {text:?}");
                return 1;
            }
        },
        None => DEFAULT_PORT,
    };
    let local_port = match args.next() {
        Some(text) => match text.parse::<u16>() {
            Ok(port) if port != 0 => port,
            _ => {
                println!("udp: bad local port {text:?}");
                return 1;
            }
        },
        None => DEFAULT_LOCAL_PORT,
    };

    let mut socket = match UdpSocket::bind(local_port) {
        Ok(socket) => socket,
        Err(error) => {
            println!("udp: bind 0.0.0.0:{local_port} failed: {error}");
            return 1;
        }
    };
    println!("udp: bound 0.0.0.0:{local_port}");

    match socket.send_to(PAYLOAD, host, port) {
        Ok(sent) if sent == PAYLOAD.len() => {
            println!("udp: sent {sent} bytes to {host}:{port}");
        }
        Ok(sent) => {
            println!("udp: short send {sent}/{} to {host}:{port}", PAYLOAD.len());
            return 2;
        }
        Err(error) => {
            println!("udp: send to {host}:{port} failed: {error}");
            return 2;
        }
    }

    let mut buffer = [0u8; 256];
    let (read, from, from_port) = match socket.recv_from(&mut buffer) {
        Ok(received) => received,
        Err(error) => {
            println!("udp: recv failed: {error}");
            return 3;
        }
    };
    println!("udp: recv {read} bytes from {from}:{from_port}");
    if read != PAYLOAD.len() || &buffer[..read] != PAYLOAD {
        println!("udp: echo payload mismatch");
        return 4;
    }
    println!("udp: ok");
    0
}
