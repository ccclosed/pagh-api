//! Raw `ecall` wrappers for the frozen syscall table (`docs/abi.md` §2).
//!
//! Every wrapper is a thin, safe Rust function: the kernel validates user
//! pointers and returns `-EFAULT` instead of faulting, so a bad slice is an
//! error, never a machine fault. Numbers live in `SYS_*` constants and are
//! checked against the ABI document by `tools/check_abi.py`.
//!
//! Syscalls the kernel has not implemented yet still go through the same
//! `ecall`; the kernel answers `-ENOSYS`, which becomes [`Errno::ENOSYS`]. The
//! wrappers never panic on a missing syscall.

use alloc::vec::Vec;

use crate::abi::{Stat, Timespec, Uname};
use crate::errno::{check, Errno, Result};

// ---------------------------------------------------------------------------
// Syscall numbers (frozen; see docs/abi.md §2)
// ---------------------------------------------------------------------------

/// `read(fd, buf, len)`.
pub const SYS_READ: usize = 63;
/// `write(fd, buf, len)`.
pub const SYS_WRITE: usize = 64;
/// `openat(dirfd, path, flags, mode)`.
pub const SYS_OPENAT: usize = 56;
/// `close(fd)`.
pub const SYS_CLOSE: usize = 57;
/// `lseek(fd, offset, whence)`.
pub const SYS_LSEEK: usize = 62;
/// `fstat(fd, *Stat)`.
pub const SYS_FSTAT: usize = 80;
/// `getdents64(fd, buf, len)`.
pub const SYS_GETDENTS64: usize = 61;
/// `brk(addr)`.
pub const SYS_BRK: usize = 214;
/// `clock_gettime(clk, *Timespec)`.
pub const SYS_CLOCK_GETTIME: usize = 113;
/// `nanosleep(*Timespec, rem)`.
pub const SYS_NANOSLEEP: usize = 101;
/// `uname(*Uname)`.
pub const SYS_UNAME: usize = 160;
/// `readlinkat(dirfd, path, buf, len)`.
pub const SYS_READLINKAT: usize = 78;
/// `getpid()`.
pub const SYS_GETPID: usize = 172;
/// `sched_yield()`.
pub const SYS_SCHED_YIELD: usize = 124;
/// `exit(code)`.
pub const SYS_EXIT: usize = 93;

// ---------------------------------------------------------------------------
// openat flags and lseek whence (frozen)
// ---------------------------------------------------------------------------

/// `openat` flag: read-only.
pub const O_RDONLY: u32 = 0;
/// `openat` flag: write-only.
pub const O_WRONLY: u32 = 1;
/// `openat` flag: read-write.
pub const O_RDWR: u32 = 2;
/// `openat` flag: create the file if missing.
pub const O_CREAT: u32 = 0x40;
/// `openat` flag: truncate to zero length.
pub const O_TRUNC: u32 = 0x80;
/// `openat` flag: append writes at the end.
pub const O_APPEND: u32 = 0x100;

/// `lseek(…, SEEK_SET)`.
pub const SEEK_SET: u32 = 0;
/// `lseek(…, SEEK_CUR)`.
pub const SEEK_CUR: u32 = 1;
/// `lseek(…, SEEK_END)`.
pub const SEEK_END: u32 = 2;

/// `clock_gettime` clock 0: realtime / uptime.
pub const CLOCK_REALTIME: i32 = 0;
/// `clock_gettime` clock 1: monotonic.
pub const CLOCK_MONOTONIC: i32 = 1;

/// The conventional `AT_FDCWD` value.
///
/// The ABI says `openat`/`readlinkat` ignore `dirfd`, but a real value is still
/// passed to the kernel so a future implementation that starts using it has a
/// well-defined meaning. `-100` is Linux's value; the ABI does not assign one,
/// which is listed as an open question in the README.
pub const AT_FDCWD: i32 = -100;

/// An address every zero-length buffer can legally point at.
///
/// The kernel validates pointers even when `len == 0`; a dangling Rust pointer
/// (such as the one behind `&[]`) is not guaranteed to be mapped. Pointing at
/// this static byte keeps `write(fd, &[], 0)` well-defined.
static ZERO_BYTE: u8 = 0;

fn ptr_of(buf: &[u8]) -> usize {
    if buf.is_empty() {
        &ZERO_BYTE as *const u8 as usize
    } else {
        buf.as_ptr() as usize
    }
}

/// Like [`check`], but also refuses a byte count larger than the caller's
/// buffer. A kernel that claims to have written past the buffer would make
/// slicing panic; returning `EIO` keeps that a clean error.
fn check_len(ret: isize, max: usize) -> Result<usize> {
    let n = check(ret)?;
    if n > max {
        Err(Errno::EIO)
    } else {
        Ok(n)
    }
}

/// NUL-terminate a path for `openat`/`readlinkat`.
///
/// Interior NUL bytes are rejected with `EINVAL` before the syscall: silently
/// truncating a path at the first NUL would open a different file than the one
/// the user named.
fn c_path(path: &str) -> Result<Vec<u8>> {
    if path.as_bytes().contains(&0) {
        return Err(Errno::EINVAL);
    }
    let mut bytes = Vec::with_capacity(path.len() + 1);
    bytes.extend_from_slice(path.as_bytes());
    bytes.push(0);
    Ok(bytes)
}

/// Issue one six-argument syscall.
///
/// # Safety
///
/// Safe to call with any register values: the kernel validates every user
/// pointer itself and never trusts these arguments. The assembly has no
/// caller-visible invariants beyond the RISC-V Linux `ecall` register
/// convention documented in `docs/abi.md` §2. It may clobber memory and the
/// syscall-clobbered registers, which the compiler is told about by the
/// absence of `nomem`/`preserves_flags` options.
#[inline]
unsafe fn syscall6(
    nr: usize,
    a0: usize,
    a1: usize,
    a2: usize,
    a3: usize,
    a4: usize,
    a5: usize,
) -> isize {
    let ret: isize;
    // SAFETY: `ecall` with `a7` = syscall number and `a0..a5` = arguments is
    // the ABI from docs/abi.md §2. The block declares a memory clobber, so the
    // compiler cannot cache user memory across it.
    unsafe {
        core::arch::asm!(
            "ecall",
            inlateout("a0") a0 => ret,
            in("a1") a1,
            in("a2") a2,
            in("a3") a3,
            in("a4") a4,
            in("a5") a5,
            in("a7") nr,
        );
    }
    ret
}

/// `read(2)`: read up to `buf.len()` bytes.
///
/// The full buffer is passed to the kernel; it validates every byte of it, so
/// a buffer that spans an unmapped page returns `EFAULT` rather than faulting.
pub fn read(fd: i32, buf: &mut [u8]) -> Result<usize> {
    // SAFETY: `buf` is a live mutable slice for the duration of the call, and
    // the kernel copies into it (validating the range) before returning.
    let ret = unsafe {
        syscall6(
            SYS_READ,
            fd as isize as usize,
            ptr_of(buf),
            buf.len(),
            0,
            0,
            0,
        )
    };
    check_len(ret, buf.len())
}

/// `write(2)`: write up to `buf.len()` bytes.
///
/// A short result is possible; use [`crate::io::Write::write_all`] to loop.
pub fn write(fd: i32, buf: &[u8]) -> Result<usize> {
    // SAFETY: `buf` is a live shared slice; the kernel only reads from it.
    let ret = unsafe {
        syscall6(
            SYS_WRITE,
            fd as isize as usize,
            ptr_of(buf),
            buf.len(),
            0,
            0,
            0,
        )
    };
    check_len(ret, buf.len())
}

/// `openat(2)`: open `path` and return a file descriptor `>= 3`.
///
/// `dirfd` is ignored by the ABI; [`AT_FDCWD`] is passed as a documented
/// placeholder. `path` must be absolute (the kernel path syscalls take absolute
/// paths only); use [`crate::path::resolve`] for relative names.
///
/// `mode` is applied only with [`O_CREAT`] and is subject to the kernel umask.
pub fn openat(dirfd: i32, path: &str, flags: u32, mode: u32) -> Result<i32> {
    let path = c_path(path)?;
    // SAFETY: `path` is a live NUL-terminated byte vector; `dirfd`, `flags` and
    // `mode` are plain integers. The kernel validates the path pointer.
    let ret = unsafe {
        syscall6(
            SYS_OPENAT,
            dirfd as isize as usize,
            path.as_ptr() as usize,
            flags as usize,
            mode as usize,
            0,
            0,
        )
    };
    check(ret).map(|fd| fd as i32)
}

/// `close(2)`.
pub fn close(fd: i32) -> Result<()> {
    // SAFETY: no pointer arguments; `fd` is validated by the kernel.
    let ret = unsafe { syscall6(SYS_CLOSE, fd as isize as usize, 0, 0, 0, 0, 0) };
    check(ret).map(|_| ())
}

/// `lseek(2)`: move the file offset and return the new absolute offset.
pub fn lseek(fd: i32, offset: i64, whence: u32) -> Result<u64> {
    // SAFETY: no pointer arguments; the kernel validates the fd and whence.
    let ret = unsafe {
        syscall6(
            SYS_LSEEK,
            fd as isize as usize,
            offset as usize,
            whence as usize,
            0,
            0,
            0,
        )
    };
    check(ret).map(|n| n as u64)
}

/// `fstat(2)`: fill `st` with the metadata of `fd`.
pub fn fstat(fd: i32, st: &mut Stat) -> Result<()> {
    // SAFETY: `st` is a live, correctly aligned, writable `Stat` for the call.
    let ret = unsafe {
        syscall6(
            SYS_FSTAT,
            fd as isize as usize,
            st as *mut Stat as usize,
            0,
            0,
            0,
            0,
        )
    };
    check(ret).map(|_| ())
}

/// `getdents64(2)`: read directory entries into `buf`.
///
/// The record layout is pagh's own; see [`crate::abi::DIRENT_HEADER`]. Zero
/// means end of directory.
pub fn getdents64(fd: i32, buf: &mut [u8]) -> Result<usize> {
    // SAFETY: `buf` is a live writable slice for the duration of the call; the
    // kernel writes at most `buf.len()` bytes into it.
    let ret = unsafe {
        syscall6(
            SYS_GETDENTS64,
            fd as isize as usize,
            ptr_of(buf),
            buf.len(),
            0,
            0,
            0,
        )
    };
    check_len(ret, buf.len())
}

/// `brk(2)`: set the program break to `addr` and return the new break.
///
/// `addr == 0` queries the current break without moving it. The ABI says
/// `-ENOMEM` is returned when the heap is exhausted; this wrapper also treats
/// a successful-but-short break as `ENOMEM` (see the allocator).
pub fn brk(addr: usize) -> Result<usize> {
    // SAFETY: no pointer arguments.
    let ret = unsafe { syscall6(SYS_BRK, addr, 0, 0, 0, 0, 0) };
    check(ret)
}

/// `clock_gettime(2)`: fill `ts` from clock `clk`.
///
/// Clock 0 is the ABI's realtime/uptime clock, clock 1 is monotonic.
pub fn clock_gettime(clk: i32, ts: &mut Timespec) -> Result<()> {
    // SAFETY: `ts` is a live, aligned, writable `Timespec` for the call.
    let ret = unsafe {
        syscall6(
            SYS_CLOCK_GETTIME,
            clk as isize as usize,
            ts as *mut Timespec as usize,
            0,
            0,
            0,
            0,
        )
    };
    check(ret).map(|_| ())
}

/// `nanosleep(2)`: sleep for `req`.
///
/// The ABI ignores the second argument. A pointer to a zeroed `Timespec` is
/// passed anyway — never NULL — so a kernel that validates both pointers
/// before noticing `rem` is ignored cannot turn the call into `EFAULT`. This is
/// a conservative reading of the contract, listed in the README.
pub fn nanosleep(req: &Timespec) -> Result<()> {
    let mut ignored = Timespec::default();
    // SAFETY: both pointers are live, aligned `Timespec`s for the call; `req`
    // is only read and `ignored` is only (potentially) written.
    let ret = unsafe {
        syscall6(
            SYS_NANOSLEEP,
            req as *const Timespec as usize,
            &mut ignored as *mut Timespec as usize,
            0,
            0,
            0,
            0,
        )
    };
    check(ret).map(|_| ())
}

/// `uname(2)`: fill `u` with the kernel identity.
pub fn uname(u: &mut Uname) -> Result<()> {
    // SAFETY: `u` is a live, aligned, writable `Uname` for the call.
    let ret = unsafe {
        syscall6(
            SYS_UNAME,
            u as *mut Uname as usize,
            0,
            0,
            0,
            0,
            0,
        )
    };
    check(ret).map(|_| ())
}

/// `readlinkat(2)`: read the target of a symbolic link into `buf`.
///
/// Returns the number of bytes placed in `buf`; the ABI does not NUL-terminate
/// the result. A full buffer may mean truncation; use a larger buffer and
/// retry (see [`crate::fs::read_link`]).
pub fn readlinkat(dirfd: i32, path: &str, buf: &mut [u8]) -> Result<usize> {
    let path = c_path(path)?;
    // SAFETY: `path` is a live NUL-terminated byte vector and `buf` is a live
    // writable slice; the kernel validates both ranges.
    let ret = unsafe {
        syscall6(
            SYS_READLINKAT,
            dirfd as isize as usize,
            path.as_ptr() as usize,
            ptr_of(buf),
            buf.len(),
            0,
            0,
        )
    };
    check_len(ret, buf.len())
}

/// `getpid(2)`: the scheduler task id of the calling process.
pub fn getpid() -> Result<i32> {
    // SAFETY: no arguments.
    let ret = unsafe { syscall6(SYS_GETPID, 0, 0, 0, 0, 0, 0) };
    check(ret).map(|pid| pid as i32)
}

/// `sched_yield(2)`: voluntarily give up the CPU.
///
/// Used by the console readers to wait for a queued byte without a poll
/// syscall (see the README's "reading stdin" note).
pub fn sched_yield() -> Result<()> {
    // SAFETY: no arguments.
    let ret = unsafe { syscall6(SYS_SCHED_YIELD, 0, 0, 0, 0, 0, 0) };
    check(ret).map(|_| ())
}

/// `exit(2)`: terminate the process. Does not return.
///
/// Kept as a normal (safe) function with a `!` return: the kernel never returns
/// to the instruction after `ecall`, and the trailing loop is only a belt-and-
/// braces park if a broken kernel ever did.
pub fn exit(code: i32) -> ! {
    // SAFETY: `exit` never returns; the final infinite loop keeps the function
    // well-formed even if the kernel wrongly returns. `a0` is the status.
    unsafe {
        core::arch::asm!(
            "ecall",
            "1:",
            "j 1b",
            in("a0") code as isize as usize,
            in("a7") SYS_EXIT,
            options(noreturn)
        );
    }
}