//! Spawn two native threads, rendezvous on a futex, join both.
#![no_std]
#![no_main]

extern crate alloc;

use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use libpagh::{println, thread};

static COUNT: AtomicUsize = AtomicUsize::new(0);
static GATE: AtomicU32 = AtomicU32::new(0);

#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8, _envp: *const *const u8) -> i32 {
    let a = thread::Builder::new()
        .stack_size(32 * 1024)
        .spawn(|| {
            println!("worker a tid={}", thread::current_id());
            COUNT.fetch_add(1, Ordering::SeqCst);
            while GATE.load(Ordering::SeqCst) == 0 {
                let _ = thread::futex_wait(&GATE, 0, None);
            }
            0
        })
        .expect("spawn a");
    let b = thread::Builder::new()
        .stack_size(32 * 1024)
        .spawn(|| {
            println!("worker b tid={}", thread::current_id());
            COUNT.fetch_add(1, Ordering::SeqCst);
            while GATE.load(Ordering::SeqCst) == 0 {
                let _ = thread::futex_wait(&GATE, 0, None);
            }
            0
        })
        .expect("spawn b");
    println!("main tid={} children {} {}", thread::current_id(), a.id(), b.id());
    while COUNT.load(Ordering::SeqCst) != 2 {
        thread::yield_now();
    }
    GATE.store(1, Ordering::SeqCst);
    println!("woke {} waiter(s)", thread::futex_wake(&GATE, 2).expect("wake"));
    let ca = a.join().expect("join a");
    let cb = b.join().expect("join b");
    println!("joined {ca} {cb}");
    if ca == 0 && cb == 0 {
        0
    } else {
        1
    }
}
