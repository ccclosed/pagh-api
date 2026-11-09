//! TCP echo server over libpagh's [`TcpListener`] — the native server
//! acceptance fixture.
//!
//! It binds `0.0.0.0:45679` (override with the first argument), listens with a
//! backlog of 4, accepts exactly one connection, reads until the peer closes
//! its sending side, writes the same bytes back and exits. QEMU's user network
//! needs an explicit `hostfwd` for a host client to reach the guest:
//!
//! ```sh
//! # tools/run.py --net --hostfwd 45679:45679 ...
//! # in the guest shell:
//! run /mnt/tcp_echo 45679
//! # on the host:
//! printf 'hello' | nc 127.0.0.1 45679
//! ```
//!
//! Build with:
//!
//! ```sh
//! cargo build --release --example tcp_echo --target riscv64gc-unknown-none-elf
//! ```
//!
//! Exit codes: 0 success, 1 bind/listen, 2 accept, 3 receive, 4 echo.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;

use libpagh::env;
use libpagh::io::Write;
use libpagh::net::{Ipv4, TcpListener};
use libpagh::println;

const DEFAULT_PORT: u16 = 45679;
const BACKLOG: u16 = 4;

#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8, _envp: *const *const u8) -> i32 {
    let port = match env::args().skip(1).next() {
        Some(text) => match text.parse::<u16>() {
            Ok(port) if port != 0 => port,
            _ => {
                println!("tcp: bad port {text:?}");
                return 1;
            }
        },
        None => DEFAULT_PORT,
    };

    let mut listener = match TcpListener::bind_with_backlog(Ipv4::ANY, port, BACKLOG) {
        Ok(listener) => listener,
        Err(error) => {
            println!("tcp: bind+listen 0.0.0.0:{port} failed: {error}");
            return 1;
        }
    };
    println!("tcp: bound 0.0.0.0:{port} backlog {BACKLOG}");
    println!("tcp: listening");

    let (mut stream, peer) = match listener.accept() {
        Ok(accepted) => accepted,
        Err(error) => {
            println!("tcp: accept failed: {error}");
            return 2;
        }
    };
    println!(
        "tcp: accept from {}:{}",
        Ipv4::new(peer.address()),
        peer.port()
    );

    // The host shuts down its write side after sending, so EOF is the end of
    // the request. `read_to_end` polls on EAGAIN and yields between attempts.
    let mut payload = Vec::new();
    if let Err(error) = stream.read_to_end(&mut payload) {
        println!("tcp: recv failed: {error}");
        return 3;
    }
    println!("tcp: recv {} bytes", payload.len());

    let mut written = 0usize;
    while written < payload.len() {
        match stream.write(&payload[written..]) {
            Ok(0) => break,
            Ok(count) => written += count,
            Err(error) => {
                println!("tcp: echo failed after {written} bytes: {error}");
                return 4;
            }
        }
    }
    if written != payload.len() {
        println!("tcp: short echo {written}/{}", payload.len());
        return 4;
    }
    println!("tcp: echoed {written} bytes");
    println!("tcp: ok");
    0
}
