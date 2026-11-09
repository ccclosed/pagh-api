//! `libpagh` — the native userland API for the `pagh-riscv` kernel.
//!
//! This crate is the userland half of the frozen contract in
//! [`docs/abi.md`](https://example.invalid): it owns `_start`, the initial
//! stack parsing, a `brk` bump allocator, syscall wrappers, `File`/`Dir`,
//! path handling and `print!`/`println!`. It is `#![no_std]`, has no external
//! dependencies and no libc.
//!
//! # Program shape
//!
//! ```ignore
//! #![no_std]
//! #![no_main]
//! #[no_mangle]
//! pub extern "C" fn main(argc: i32, argv: *const *const u8, envp: *const *const u8) -> i32 {
//!     println!("hello from {argc} args");
//!     0
//! }
//! ```
//!
//! The crate must be *used* (any item referenced) so rustc links it; that pulls
//! in `_start` from the linker script's `ENTRY(_start)` and the panic handler.
//! See `examples/hello.rs` and the README.
//!
//! # Not provided
//!
//! Threads and futexes exist ([`thread`]); fork, signals and dynamic linking
//! do not. Sockets are a connected IPv4 TCP client ([`net::TcpStream`]), a TCP
//! listener ([`net::TcpListener`]) and bound UDP datagram sockets
//! ([`net::UdpSocket`]); [`dns`] resolves `A` records over one of those UDP
//! sockets, and [`tls::TlsStream`] puts a kernel-verified TLS 1.3 session on a
//! connected TCP descriptor (there is no UDP `connect`). [`random`] is the
//! kernel entropy pool. See `docs/abi.md` §2/§4 and the README's "What is not
//! supported" list.
//!
//! Every `unsafe` block carries a `SAFETY:` comment; the small number of them
//! is intentional.

#![no_std]
#![feature(alloc_error_handler)]
#![deny(unsafe_op_in_unsafe_fn)]

extern crate alloc;

pub mod abi;
mod alloc_impl;
pub mod dns;
pub mod env;
pub mod errno;
pub mod fs;
pub mod io;
pub mod net;
pub mod path;
pub mod print;
pub mod random;
mod rt;
pub mod sync;
pub mod sys;
pub mod thread;
pub mod tls;

pub use abi::{EntryKind, SockAddrIn, Stat, Timespec, Uname};
pub use errno::{Errno, Result};

/// Print to standard output (fd 1).
///
/// Formatting uses `core::fmt` directly; a failed write is ignored because
/// there is nowhere sensible to report it. For data paths that must notice a
/// write error (like `cat`), use [`io::Write`] instead.
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {
        $crate::print::_print(core::format_args!($($arg)*))
    };
}

/// Print to standard output followed by a newline.
#[macro_export]
macro_rules! println {
    () => {
        $crate::print::_print(core::format_args!("\n"))
    };
    ($($arg:tt)*) => {{
        $crate::print::_print(core::format_args!($($arg)*));
        $crate::print::_print(core::format_args!("\n"));
    }};
}

/// Print to standard diagnostics (fd 2).
#[macro_export]
macro_rules! eprint {
    ($($arg:tt)*) => {
        $crate::print::_eprint(core::format_args!($($arg)*))
    };
}

/// Print to standard diagnostics followed by a newline.
#[macro_export]
macro_rules! eprintln {
    () => {
        $crate::print::_eprint(core::format_args!("\n"))
    };
    ($($arg:tt)*) => {{
        $crate::print::_eprint(core::format_args!($($arg)*));
        $crate::print::_eprint(core::format_args!("\n"));
    }};
}
