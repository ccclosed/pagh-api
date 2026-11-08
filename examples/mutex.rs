//! Four native threads increment a value under libpagh's futex Mutex.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::sync::Arc;
use libpagh::sync::Mutex;
use libpagh::{print, thread};

#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8, _envp: *const *const u8) -> i32 {
    let counter = Arc::new(Mutex::new(0u32));
    let mut handles = alloc::vec::Vec::new();
    for _ in 0..4 {
        let counter = Arc::clone(&counter);
        handles.push(
            thread::Builder::new()
                .stack_size(32 * 1024)
                .spawn(move || {
                    for _ in 0..250 {
                        *counter.lock() += 1;
                    }
                    0
                })
                .expect("spawn"),
        );
    }
    for handle in handles {
        handle.join().expect("join");
    }
    let value = *counter.lock();
    print!("counter={}\n", value);
    if value == 1000 { 0 } else { 1 }
}
