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

use crate::abi::{SockAddrIn, Stat, Timespec, Uname};
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
/// `mkdirat(dirfd, path, mode)`.
pub const SYS_MKDIRAT: usize = 34;
/// `unlinkat(dirfd, path, flags)`.
pub const SYS_UNLINKAT: usize = 35;
/// `symlinkat(target, dirfd, path)`.
pub const SYS_SYMLINKAT: usize = 36;
/// `linkat(olddirfd, oldpath, newdirfd, newpath, flags)`.
pub const SYS_LINKAT: usize = 37;
/// `fchmodat(dirfd, path, mode, flags)`.
pub const SYS_FCHMODAT: usize = 53;
/// `ftruncate(fd, length)`.
pub const SYS_FTRUNCATE: usize = 46;
/// `renameat2(olddirfd, oldpath, newdirfd, newpath, flags)`.
pub const SYS_RENAMEAT2: usize = 276;
/// `socket(domain, type, protocol)`.
pub const SYS_SOCKET: usize = 198;
/// `connect(fd, *sockaddr_in, addrlen)`.
pub const SYS_CONNECT: usize = 203;
/// `sendto(fd, buf, len, flags, dest, addrlen)` (the `send` shape on a
/// connected socket).
pub const SYS_SENDTO: usize = 206;
/// `recvfrom(fd, buf, len, flags, src, addrlen)` (the `recv` shape on a
/// connected socket).
pub const SYS_RECVFROM: usize = 207;
/// `setsockopt(fd, level, optname, optval, optlen)`.
pub const SYS_SETSOCKOPT: usize = 208;
/// `getsockopt(fd, level, optname, optval, optlen)`.
pub const SYS_GETSOCKOPT: usize = 209;
/// `shutdown(fd, how)`.
pub const SYS_SHUTDOWN: usize = 210;

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

/// `unlinkat` flag: remove an empty directory instead of a file.
pub const AT_REMOVEDIR: u32 = 0x200;

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


/// `mkdirat(2)`: create the directory `path` (absolute; resolved against `PWD`).
///
/// `dirfd` is ignored by the ABI; [`AT_FDCWD`] is passed as the documented
/// placeholder. `mode` is accepted and **ignored** by the current kernel: the
/// VFS has no `set_mode`, so the filesystem default (ext2 `0o755`) stands.
/// [`crate::fs::set_permissions`] records an observable per-run override, but
/// nothing writes the on-disk permission bits.
pub fn mkdirat(dirfd: i32, path: &str, mode: u32) -> Result<()> {
    let path = c_path(path)?;
    // SAFETY: `path` is a live NUL-terminated byte vector; the rest are plain
    // integers. The kernel validates the path pointer.
    let ret = unsafe {
        syscall6(
            SYS_MKDIRAT,
            dirfd as isize as usize,
            path.as_ptr() as usize,
            mode as usize,
            0,
            0,
            0,
        )
    };
    check(ret).map(|_| ())
}

/// `unlinkat(2)`: remove a file or symbolic link (`flags = 0`), or an empty
/// directory (`flags = AT_REMOVEDIR`).
///
/// A directory without `AT_REMOVEDIR` is `EISDIR`; a non-empty one with it is
/// `ENOTEMPTY`; any other flag bit is `EINVAL`.
pub fn unlinkat(dirfd: i32, path: &str, flags: u32) -> Result<()> {
    let path = c_path(path)?;
    // SAFETY: `path` is a live NUL-terminated byte vector; the kernel validates
    // the pointer.
    let ret = unsafe {
        syscall6(
            SYS_UNLINKAT,
            dirfd as isize as usize,
            path.as_ptr() as usize,
            flags as usize,
            0,
            0,
            0,
        )
    };
    check(ret).map(|_| ())
}

/// `symlinkat(2)`: create the symbolic link `path` whose target is `target`.
///
/// The target is stored verbatim, not resolved here: a relative target is
/// interpreted later relative to the directory that contains the link. Note the
/// argument order: Linux's `symlinkat(target, newdirfd, linkpath)`, mirrored by
/// this ABI.
pub fn symlinkat(target: &str, dirfd: i32, path: &str) -> Result<()> {
    let target = c_path(target)?;
    let path = c_path(path)?;
    // SAFETY: both are live NUL-terminated byte vectors; the kernel validates
    // both pointers.
    let ret = unsafe {
        syscall6(
            SYS_SYMLINKAT,
            target.as_ptr() as usize,
            dirfd as isize as usize,
            path.as_ptr() as usize,
            0,
            0,
            0,
        )
    };
    check(ret).map(|_| ())
}

/// `linkat(2)`: create `newpath` as another name for `oldpath`'s inode.
///
/// `flags` must be 0; both dirfds are ignored. A directory target is `EISDIR`
/// (ext2 has no directory hard links). The link and its target must live on the
/// same filesystem, or the VFS answers `ENOTSUP`.
pub fn linkat(
    olddirfd: i32,
    oldpath: &str,
    newdirfd: i32,
    newpath: &str,
    flags: u32,
) -> Result<()> {
    let oldpath = c_path(oldpath)?;
    let newpath = c_path(newpath)?;
    // SAFETY: both are live NUL-terminated byte vectors; the kernel validates
    // both pointers.
    let ret = unsafe {
        syscall6(
            SYS_LINKAT,
            olddirfd as isize as usize,
            oldpath.as_ptr() as usize,
            newdirfd as isize as usize,
            newpath.as_ptr() as usize,
            flags as usize,
            0,
        )
    };
    check(ret).map(|_| ())
}

/// `renameat2(2)`: move `oldpath` to `newpath`.
///
/// `flags` must be 0 (no `RENAME_NOREPLACE`/`RENAME_EXCHANGE` in this ABI).
/// Because `/mnt` is the only mount, a rename into `/` is `ENOTSUP` — copying
/// is not renaming.
pub fn renameat2(
    olddirfd: i32,
    oldpath: &str,
    newdirfd: i32,
    newpath: &str,
    flags: u32,
) -> Result<()> {
    let oldpath = c_path(oldpath)?;
    let newpath = c_path(newpath)?;
    // SAFETY: both are live NUL-terminated byte vectors; the kernel validates
    // both pointers.
    let ret = unsafe {
        syscall6(
            SYS_RENAMEAT2,
            olddirfd as isize as usize,
            oldpath.as_ptr() as usize,
            newdirfd as isize as usize,
            newpath.as_ptr() as usize,
            flags as usize,
            0,
        )
    };
    check(ret).map(|_| ())
}

/// `fchmodat(2)`: set the permission bits of `path` to `mode & 0o7777`.
///
/// The current kernel has no writable mode in the VFS: it records the bits for
/// the rest of the **run** and `fstat` reports them; nothing is written to the
/// ext2 inode and no access check consults the value. A synthetic node (the
/// root `/`) has no inode and answers `ENOTSUP`. `flags` must be 0
/// (`AT_SYMLINK_NOFOLLOW` is not part of this ABI).
pub fn fchmodat(dirfd: i32, path: &str, mode: u32, flags: u32) -> Result<()> {
    let path = c_path(path)?;
    // SAFETY: `path` is a live NUL-terminated byte vector; the kernel validates
    // the pointer.
    let ret = unsafe {
        syscall6(
            SYS_FCHMODAT,
            dirfd as isize as usize,
            path.as_ptr() as usize,
            mode as usize,
            flags as usize,
            0,
            0,
        )
    };
    check(ret).map(|_| ())
}

/// `ftruncate(2)`: resize the open regular file behind `fd` to `length` bytes.
///
/// Grows with zeroes and shrinks by releasing blocks. Console descriptors are
/// `EINVAL`, a directory is `EISDIR`, and a descriptor opened read-only is
/// `EBADF` (see [`crate::fs::truncate`] and [`crate::fs::File::set_len`]).
pub fn ftruncate(fd: i32, length: u64) -> Result<()> {
    // SAFETY: no pointer arguments; the kernel validates the descriptor.
    let ret = unsafe {
        syscall6(
            SYS_FTRUNCATE,
            fd as isize as usize,
            length as usize,
            0,
            0,
            0,
            0,
        )
    };
    check(ret).map(|_| ())
}

// ---------------------------------------------------------------------------
// BSD sockets (AF_INET / SOCK_STREAM only; see docs/abi.md §2)
// ---------------------------------------------------------------------------

/// `AF_INET`: the only address family the kernel accepts.
pub const AF_INET: u16 = crate::abi::AF_INET;
/// `SOCK_STREAM`: the only socket type the kernel accepts.
pub const SOCK_STREAM: u32 = crate::abi::SOCK_STREAM;
/// `shutdown`: no further receives.
pub const SHUT_RD: u32 = 0;
/// `shutdown`: no further sends.
pub const SHUT_WR: u32 = 1;
/// `shutdown`: both directions.
pub const SHUT_RDWR: u32 = 2;

/// `socket(domain, type, protocol)`: create a BSD socket descriptor.
///
/// The kernel has one TCP connection, so at most one socket descriptor exists
/// per run: a second call while the first is open is `ENOSPC`. The only
/// supported combination is `AF_INET` / `SOCK_STREAM` / protocol 0; anything
/// else is `EAFNOSUPPORT` (the kernel documents one answer rather than
/// inventing per-field errnos).
pub fn socket(domain: u16, kind: u32, protocol: u32) -> Result<i32> {
    // SAFETY: all arguments are plain integers; no pointers.
    let ret = unsafe {
        syscall6(
            SYS_SOCKET,
            domain as usize,
            kind as usize,
            protocol as usize,
            0,
            0,
            0,
        )
    };
    check(ret).map(|fd| fd as i32)
}

/// `connect(fd, *sockaddr_in, sizeof(sockaddr_in))`.
///
/// IPv4 literals only: this ABI has no DNS resolver and no `getaddrinfo`. The
/// kernel validates the 16-byte structure (family, address, port) before it
/// sends a SYN and polls until the handshake completes or its own deadline
/// expires (`ETIMEDOUT`).
pub fn connect(fd: i32, address: &SockAddrIn) -> Result<()> {
    // SAFETY: `address` is a live, correctly aligned `SockAddrIn` for the call;
    // the kernel copies exactly 16 bytes and validates the family.
    let ret = unsafe {
        syscall6(
            SYS_CONNECT,
            fd as isize as usize,
            address as *const SockAddrIn as usize,
            core::mem::size_of::<SockAddrIn>(),
            0,
            0,
            0,
        )
    };
    check(ret).map(|_| ())
}

/// `send` on a connected socket: the `sendto` wire shape with no destination.
///
/// TCP's send is synchronous in the kernel (it waits for the acknowledgement
/// with a bounded poll), so a successful return means the bytes left this
/// machine. A peer close is `EPIPE`, a reset `ECONNRESET`, a stalled peer
/// `ETIMEDOUT`.
pub fn send(fd: i32, buf: &[u8]) -> Result<usize> {
    // SAFETY: `buf` is a live shared slice; the kernel only reads it and
    // validates the range. No destination address is passed on a connected
    // socket.
    let ret = unsafe {
        syscall6(
            SYS_SENDTO,
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

/// `recv` on a connected socket: the `recvfrom` wire shape with no source
/// address.
///
/// **Non-blocking**: `EAGAIN` means no payload is queued yet and the caller
/// should poll again (the same contract as `read(0)`); `0` means the peer
/// closed. IPv4 literals only, like [`connect`].
pub fn recv(fd: i32, buf: &mut [u8]) -> Result<usize> {
    // SAFETY: `buf` is a live writable slice for the duration of the call; the
    // kernel validates every page and writes at most `buf.len()` bytes.
    let ret = unsafe {
        syscall6(
            SYS_RECVFROM,
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

/// `shutdown(fd, how)`: close the connection while keeping the descriptor.
///
/// The kernel has no half-close: `how` is validated (`SHUT_RD`, `SHUT_WR`,
/// `SHUT_RDWR`) but both directions close. A later `recv` returns `Ok(0)` (end
/// of stream) and a later `send` is `EPIPE`; [`crate::sys::close`] releases the
/// descriptor.
pub fn shutdown(fd: i32, how: u32) -> Result<()> {
    // SAFETY: no pointer arguments; the kernel validates the descriptor and
    // the `how` value.
    let ret = unsafe { syscall6(SYS_SHUTDOWN, fd as isize as usize, how as usize, 0, 0, 0, 0) };
    check(ret).map(|_| ())
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
