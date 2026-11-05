//! HTTP/1.0 GET over a libpagh [`TcpStream`] — the native-socket smoke example.
//!
//! It connects to `10.0.2.2:8000` (QEMU's user network maps that to the host's
//! loopback), sends one `GET /` and prints the whole response body. Run a host
//! server first:
//!
//! ```sh
//! python3 -m http.server 8000 --bind 127.0.0.1
//! # in the guest shell:
//! run /mnt/http_get
//! ```
//!
//! Build with:
//!
//! ```sh
//! cargo build --release --example http_get --target riscv64gc-unknown-none-elf
//! ```
//!
//! The address is a literal on purpose: this ABI has no DNS (`docs/abi.md` §2).
//! Exit codes: 0 success, 1 connect, 2 write, 3 read, 4 empty response.

#![no_std]
#![no_main]

extern crate alloc;

use libpagh::io::Write;
use libpagh::net::{Ipv4, TcpStream};
use libpagh::println;

const HOST: Ipv4 = Ipv4::new([10, 0, 2, 2]);
const PORT: u16 = 8000;

#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8, _envp: *const *const u8) -> i32 {
    println!("http_get: connecting to {HOST}:{PORT}");
    let mut stream = match TcpStream::connect(HOST, PORT) {
        Ok(stream) => stream,
        Err(error) => {
            println!("http_get: connect failed: {error}");
            return 1;
        }
    };
    println!("http_get: connected, sending GET /");

    let request = b"GET / HTTP/1.0\r\nHost: 10.0.2.2\r\n\r\n";
    if let Err(error) = stream.write_all(request) {
        println!("http_get: write failed: {error}");
        return 2;
    }

    // `read_to_end` polls on EAGAIN and stops at EOF, which HTTP/1.0's
    // connection-close framing makes the end of the response.
    let mut response = alloc::vec::Vec::new();
    if let Err(error) = stream.read_to_end(&mut response) {
        println!("http_get: read failed: {error}");
        return 3;
    }
    if response.is_empty() {
        println!("http_get: empty response");
        return 4;
    }

    println!("http_get: {} response bytes", response.len());
    let mut stdout = libpagh::io::Stdout;
    if let Err(error) = stdout.write_all(&response) {
        println!("http_get: printing the response failed: {error}");
        return 2;
    }
    println!("\nhttp_get: done");
    0
}
