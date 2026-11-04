//! Minimal `libpagh` program: prints its arguments, `PWD`, a resolved relative
//! path and the result of `stat("/")`.
//!
//! Build it with `./build.sh` (or `cargo build --release --example hello
//! --target riscv64gc-unknown-none-elf`) and run the resulting static ELF in
//! the kernel with `run <path>`.
//!
//! Note the shape required by `docs/abi.md` §1: `#![no_std]`, `#![no_main]`, and
//! a `#[no_mangle] extern "C" fn main` with the three-argument signature. The
//! crate must use `libpagh` somewhere (here `println!`), otherwise rustc has no
//! reason to link the crate that provides `_start` and the panic handler.

#![no_std]
#![no_main]

use libpagh::println;

#[no_mangle]
pub extern "C" fn main(
    argc: i32,
    argv: *const *const u8,
    envp: *const *const u8,
) -> i32 {
    // The runtime parses the same stack; the raw arguments are only used here
    // to prove the entry contract is intact.
    println!("hello from libpagh");
    println!("raw argc = {argc}, argv = {argv:p}, envp = {envp:p}");

    for (index, arg) in libpagh::env::args().enumerate() {
        println!("argv[{index}] = {arg}");
    }

    match libpagh::env::var("PWD") {
        Some(pwd) => println!("PWD = {pwd}"),
        None => println!("PWD is unset"),
    }
    println!("current_dir = {}", libpagh::env::current_dir());
    println!("resolve(./hello) = {}", libpagh::path::resolve("./hello"));

    match libpagh::fs::stat("/") {
        Ok(stat) => println!(
            "stat(/) = kind {:?}, mode {:#o}, size {}",
            stat.entry_kind(),
            stat.mode,
            stat.size
        ),
        Err(err) => println!("stat(/) failed: {err}"),
    }

    0
}