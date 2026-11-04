//! The program entry contract from `docs/abi.md` §1.
//!
//! The kernel enters U-mode at `_start` with:
//!
//! ```text
//! a0 = argc
//! a1 = argv
//! sp -> [argc][argv0..argvN-1][NULL][env0..envK-1][NULL][strings...]
//! ```
//!
//! `_start` saves `sp` and calls [`start`], which parses the stack, calls the
//! program's `extern "C" fn main(argc, argv, envp) -> i32`, and exits with the
//! returned status. No unwinding, no constructors, no dynamic linker.

use core::cell::UnsafeCell;
use core::ffi::CStr;
use core::fmt;
use core::panic::PanicInfo;
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::path;
use crate::print;
use crate::sys;

/// `sp` at `_start`, used by [`crate::env`] to re-read `argv`/`envp` on demand.
static INITIAL_SP: AtomicUsize = AtomicUsize::new(0);

/// Length of the program name in [`NAME_BUF`] (0 means "not set yet").
static NAME_LEN: AtomicUsize = AtomicUsize::new(0);

/// How much of `argv[0]`'s basename a panic message keeps.
const NAME_CAP: usize = 63;

struct NameBuf(UnsafeCell<[u8; NAME_CAP + 1]>);

// SAFETY: written once in `start` before any user code runs, then read-only.
// The ABI guarantees a single-threaded process, so there is no concurrent
// writer; `NAME_LEN` publishes the initialized prefix.
unsafe impl Sync for NameBuf {}

static NAME_BUF: NameBuf = NameBuf(UnsafeCell::new([0; NAME_CAP + 1]));

// SAFETY: this declaration only names the symbol the program must provide
// (`#[no_mangle] extern "C" fn main`). The kernel's entry contract guarantees
// the stack layout and the lifetime of the pointers; `main` is called exactly
// once, from `start`, before any other user code runs.
//
// The program's entry point, defined by the program itself as
// `#[no_mangle] pub extern "C" fn main(...) -> i32`.
unsafe extern "C" {
    fn main(argc: i32, argv: *const *const u8, envp: *const *const u8) -> i32;
}

core::arch::global_asm!(
    r#"
    .section .text.entry, "ax", @progbits
    .globl _start
    .type  _start, @function
_start:
    /* The kernel enters here with sp pointing at argc; a0/a1 also carry
       argc/argv, but the stack is the documented source of truth. Save sp in
       the first argument and call the Rust half. The call does not adjust sp,
       so the 16-byte alignment the ABI promises is preserved. */
    mv      a0, sp
    call    {start}
1:
    /* start() ends in exit(2); if a broken kernel ever returns, park. */
    j       1b
    .size _start, . - _start
    "#,
    start = sym start
);

/// Rust half of `_start`. Called only from the assembly above.
///
/// Parses the initial stack, calls the program's `main`, and exits with its
/// status. Never returns.
pub(crate) extern "C" fn start(sp: usize) -> ! {
    INITIAL_SP.store(sp, Ordering::Relaxed);
    let (argc, argv, envp) = parse_stack(sp);
    set_program_name(argv, argc);

    // SAFETY: `main` is the program's entry symbol, required by the contract
    // this crate implements; the kernel guarantees the stack layout. The
    // program's `main` may not unwind: the crate graph is built with
    // `panic = "abort"` and `libpagh` supplies the only panic handler.
    let code = unsafe { main(argc, argv, envp) };
    sys::exit(code)
}

/// The maximum `argc` this runtime will believe.
///
/// Not an ABI limit; a sanity bound so a corrupt stack cannot make
/// `argv.add(argc + 1)` wrap. A real command line is far below it.
const MAX_ARGC: usize = 4096;

/// Split the initial stack into `(argc, argv, envp)`.
///
/// The layout is the SysV-style one from the contract, not the Linux `auxv`
/// layout: `[argc][argv…][NULL][env…][NULL][strings…]`.
pub(crate) fn parse_stack(sp: usize) -> (i32, *const *const u8, *const *const u8) {
    // SAFETY: the kernel guarantees `sp` points at a readable `usize` argc on
    // entry. We read exactly one word before computing the argv pointer; a
    // corrupt stack is outside the contract.
    let raw_argc = unsafe { core::ptr::read(sp as *const usize) };
    let argc = if raw_argc > MAX_ARGC { 0 } else { raw_argc as i32 };

    let argv = sp.wrapping_add(core::mem::size_of::<usize>()) as *const *const u8;
    // SAFETY: argv has `argc + 1` pointer-sized slots by contract (the last is
    // NULL); one-past-the-end is not dereferenced here, only stored.
    let envp = unsafe { argv.add(argc as usize + 1) };
    (argc, argv, envp)
}

/// The stack pointer captured by `_start`, or 0 before initialization.
pub(crate) fn initial_sp() -> usize {
    INITIAL_SP.load(Ordering::Relaxed)
}

/// Copy the basename of `argv[0]` into the fixed buffer used by diagnostics.
fn set_program_name(argv: *const *const u8, argc: i32) {
    if argc <= 0 || argv.is_null() {
        return;
    }
    // SAFETY: the contract says argv has argc valid pointers; argc > 0.
    let first = unsafe { *argv };
    if first.is_null() {
        return;
    }
    // SAFETY: argv[0] is a NUL-terminated string maintained by the kernel.
    let bytes = unsafe { CStr::from_ptr(first as *const core::ffi::c_char) }.to_bytes();
    let text = core::str::from_utf8(bytes).unwrap_or("");
    let base = path::basename(text);
    let base = if base.is_empty() { "pagh" } else { base };

    let mut len = base.len().min(NAME_CAP);
    while len > 0 && !base.is_char_boundary(len) {
        len -= 1;
    }
    // SAFETY: `NAME_BUF` is `NAME_CAP + 1` bytes and `len <= NAME_CAP`; this is
    // the only writer and it runs before user code. The source is a live &str.
    unsafe {
        core::ptr::copy_nonoverlapping(base.as_ptr(), NAME_BUF.0.get() as *mut u8, len);
    }
    NAME_LEN.store(len, Ordering::Relaxed);
}

/// The program name used in panic/OOM messages (`argv[0]` basename), or
/// `"pagh"` before initialization.
pub(crate) fn program_name() -> &'static str {
    let len = NAME_LEN.load(Ordering::Relaxed);
    if len == 0 {
        return "pagh";
    }
    // SAFETY: `NAME_BUF` is initialized up to `len` by `set_program_name` from
    // a valid UTF-8 `&str`; it is not modified afterwards. The returned
    // reference is valid for the process lifetime (the buffer is static).
    let bytes = unsafe { core::slice::from_raw_parts(NAME_BUF.0.get() as *const u8, len) };
    core::str::from_utf8(bytes).unwrap_or("pagh")
}

/// Print `program: <args>` to fd 2 and exit with `code`.
///
/// Does not allocate: this is also the allocator-failure path.
pub(crate) fn fatal(code: i32, args: fmt::Arguments<'_>) -> ! {
    let _ = print::write_fmt_fd(2, format_args!("{}: {}\n", program_name(), args));
    sys::exit(code)
}

/// Panic handler: `program: panic at <location>`, fd 2, exit 101.
///
/// `docs/abi.md` §4 requires exactly this shape; there is no unwinding and no
/// backtrace (the kernel has no debug-info consumer yet).
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    match info.location() {
        Some(location) => fatal(101, format_args!("panic at {location}")),
        None => fatal(101, format_args!("panic at <unknown location>")),
    }
}