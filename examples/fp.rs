//! Prove that U-mode can execute RISC-V floating-point instructions.
//!
//! The kernel must enable `sstatus.FS` before `sret`; otherwise the first
//! `fadd`/`fmul` traps as an illegal instruction. This example performs a few
//! f64 operations and prints the result, so it is a live check, not a symbol
//! grep.
#![no_std]
#![no_main]

extern crate alloc;

use libpagh::println;

#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8, _envp: *const *const u8) -> i32 {
    let a = 1.5f64;
    let b = 2.25f64;
    let product = a * b;
    let mut sum = 0.0f64;
    for i in 1..=100 {
        sum += i as f64;
    }
    println!("fp product={product}");
    println!("fp sum={sum}");
    if product == 3.375 && sum == 5050.0 {
        0
    } else {
        7
    }
}
