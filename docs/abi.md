# pagh native userland ABI (libpagh, RISC-V RV64)

Status: **frozen for the first ports**. The kernel (`src/user/syscall.rs`) and
`libpagh` must match this document; a mismatch is a bug in whichever side drifted.

This is **not a Linux layer**. The numbers below happen to follow the generic
RISC-V Linux syscall table because the kernel already used `write=64`/`exit=93`
for its fixtures; the semantics, structs and the runtime are pagh's own. Programs
are written against `libpagh`, not against a libc, and there is no ELF
interpreter, `auxv`, `ld.so` or Linux-specific `struct stat`.

## 1. Entry contract

The kernel loads a static `ET_EXEC`/`ET_DYN` ELF (`run <path> [args...]`), maps a
64 KiB user stack, builds the initial stack and enters U-mode with:

```
a0 = argc
a1 = argv        // *const *const u8, NULL-terminated
sp = initial_sp  // 16-byte aligned, points at argc (SysV-style layout):
                 //   [argc][argv0..argvN-1][NULL][env0..envK-1][NULL][strings...]
```

`argv[0]` is the program path as typed on the shell command line (`run` passes it).
The environment is minimal and always includes `PWD=<shell cwd>`; programs must
resolve relative paths themselves against `PWD` (the kernel path syscalls take
absolute paths only).

Registers `s0..s11`, `ra`, `gp`, `tp` are unspecified on entry; `_start` in
`libpagh` saves `sp`, calls the program's `main(argc, argv, envp) -> i32` and then
`exit(ret)`.

## 2. Syscalls

`a7` = number, `a0..a5` = arguments, result in `a0`. Negative values are `-errno`
(errno values below). Every pointer is validated by the kernel against the
current user address space; a bad pointer returns `-EFAULT` and never faults.

| nr | name | args | result |
|---|---|---|---|
| 63 | `read` | `fd`, `buf`, `len` | bytes read; `-EAGAIN` when fd 0 has no byte queued yet |
| 64 | `write` | `fd`, `buf`, `len` | bytes written |
| 56 | `openat` | `dirfd` (ignored), `path` (absolute), `flags`, `mode` | fd ≥ 3 |
| 57 | `close` | `fd` | 0 |
| 62 | `lseek` | `fd`, `offset`, `whence` (0 SET, 1 CUR, 2 END) | new offset |
| 80 | `fstat` | `fd`, `*Stat` | 0 |
| 61 | `getdents64` | `fd`, `buf`, `len` | bytes; 0 at end of directory |
| 214 | `brk` | `addr` (0 = query current) | new break; `-ENOMEM` when exhausted |
| 113 | `clock_gettime` | `clk` (0 realtime/uptime, 1 monotonic), `*Timespec` | 0 |
| 101 | `nanosleep` | `*Timespec`, `rem` (ignored) | 0 |
| 160 | `uname` | `*Uname` | 0 |
| 78 | `readlinkat` | `dirfd`, `path`, `buf`, `len` | bytes placed (no NUL) |
| 172 | `getpid` | — | pid |
| 124 | `sched_yield` | — | 0 |
| 93 | `exit` | `code` | does not return |
| 34 | `mkdirat` | `dirfd` (ignored), `path`, `mode` (ignored; see below) | 0; `-EEXIST`, `-ENOENT`, `-ENOTDIR` |
| 35 | `unlinkat` | `dirfd` (ignored), `path`, `flags` | 0; `-EISDIR` (directory without `AT_REMOVEDIR`), `-ENOTDIR` (`AT_REMOVEDIR` on a file), `-ENOTEMPTY` (non-empty directory), `-EINVAL` (unknown flag bit) |
| 36 | `symlinkat` | `target` (raw, **not** resolved), `dirfd` (ignored), `path` | 0; `-EEXIST`, `-EINVAL` (empty target) |
| 37 | `linkat` | `olddirfd`, `oldpath`, `newdirfd`, `newpath`, `flags` (must be 0) | 0; `-EEXIST`, `-EISDIR` (directory target), `-ENOTSUP` (foreign filesystem) |
| 276 | `renameat2` | `olddirfd`, `oldpath`, `newdirfd`, `newpath`, `flags` (must be 0) | 0; `-EEXIST`, `-ENOENT`, `-ENOTEMPTY`, `-EISDIR`, `-ENOTSUP` |
| 53 | `fchmodat` | `dirfd` (ignored), `path`, `mode` (`& 0o7777`), `flags` (must be 0) | 0; mode is per-run and visible to `fstat`, not persisted; `-ENOTSUP` on a synthetic node |
| 46 | `ftruncate` | `fd`, `length` | 0; `-EINVAL` for fd 0/1/2, `-EBADF` (read-only fd), `-EISDIR` |
| 198 | `socket` | `domain` (`AF_INET=2`), `type` (`SOCK_STREAM=1`), `protocol` (0) | fd ≥ 3; `-EAFNOSUPPORT` for any other combination; a second socket while one is open is `-ENOSPC` |
| 203 | `connect` | `fd`, `*SockAddrIn`, `addrlen` (must be 16) | 0; `-EINVAL`, `-EAFNOSUPPORT`, `-ENOTSOCK`, `-EISCONN`, `-ENETUNREACH`, `-EHOSTUNREACH`, `-ECONNREFUSED`, `-ETIMEDOUT`, `-EAGAIN` |
| 206 | `sendto` | `fd`, `buf`, `len`, `flags` (must be 0), `dest` (0, or validated and ignored on a connected socket), `addrlen` | bytes sent; `-ENOTCONN`, `-EPIPE`, `-ECONNRESET`, `-ETIMEDOUT`; the `send` shape |
| 207 | `recvfrom` | `fd`, `buf`, `len`, `flags` (must be 0), `src` (0, or filled with the peer), `*addrlen` | bytes; **`-EAGAIN` when nothing is queued yet** (poll again), 0 = peer closed; the `recv` shape |
| 208 | `setsockopt` | `fd`, `level`, `optname`, `optval`, `optlen` | 0 for `SO_REUSEADDR`, `SO_KEEPALIVE`, `SO_RCVTIMEO`, `SO_SNDTIMEO`, `TCP_NODELAY` (validated and ignored); `-ENOPROTOOPT` otherwise |
| 209 | `getsockopt` | `fd`, `level`, `optname`, `optval`, `*optlen` | 0 for `SO_TYPE` (`SOCK_STREAM`), `SO_ERROR` (0), `SO_RCVTIMEO`/`SO_SNDTIMEO` (0), `TCP_NODELAY` (0); `-ENOPROTOOPT` otherwise |
| 210 | `shutdown` | `fd`, `how` (0 read, 1 write, 2 both) | 0; both directions close (no half-close), the descriptor stays open: later `read` is 0, `write` is `-EPIPE` |
| 222 | `mmap` | `addr` (hint, 0 = pick), `length`, `prot` (`PROT_READ=1`/`WRITE=2`/`EXEC=4`), `flags` (`MAP_PRIVATE=2\|MAP_ANONYMOUS=0x20`), `fd` (-1), `offset` (0) | mapped address; `-ENOMEM`, `-EINVAL`, `-ENOTSUP` (shared/file-backed) |
| 215 | `munmap` | `addr` (page-aligned), `length` | 0; `-EINVAL` for a range that is not this run's mapping |
| 226 | `mprotect` | `addr` (page-aligned), `length`, `prot` | 0; W^X is rejected with `-EINVAL` |
| 59 | `pipe2` | `*fds[2]`, `flags` (must be 0) | 0; read end non-blocking (`-EAGAIN` when empty), EOF once all write ends close |
| 23 | `dup` | `fd` | new fd |
| 24 | `dup2` | `oldfd`, `newfd` | `newfd` (asm-generic number 24 also answers `dup3`) |
| 7 | `poll` | `*PollFd`, `nfds`, `timeout_ms` | ready count; `POLLIN`/`POLLOUT`/`POLLERR`/`POLLHUP`/`POLLNVAL` |
| 72 | `ppoll` | `*PollFd`, `nfds`, `*timespec` (treated as ms), mask (ignored) | as `poll` |


Maximum open files per process: **16**, including 0/1/2. fd 0 = console input,
1 = console output, 2 = console diagnostics (all three are the same UART/screen;
`read(0)` also drains the virtio-keyboard).

On a socket descriptor, `read` is `recv` with `flags = 0`, `write` is `send`
with `flags = 0`, and `close` closes the TCP connection and releases the
descriptor. `lseek`/`ftruncate` answer `-EINVAL`, `fstat` reports
`kind = OTHER` like the console.

### Directory mutations

`mkdirat`, `unlinkat`, `symlinkat`, `linkat`, `renameat2`, `fchmodat` and
`ftruncate` change the tree. All paths are absolute UTF-8 strings (≤ 255 bytes
plus the NUL), all `dirfd` arguments are ignored, and a `flags` argument the
kernel does not implement must be 0 — it is `-EINVAL`, not ignored. The final
path component is never followed by `unlinkat`/`linkat` (removing or linking a
symbolic link acts on the link itself); `fchmodat` follows it, like `chmod`.
Every mutation is one filesystem transaction, so a failed call leaves the tree
as it was; the file-descriptor table is not modified except by `openat`.

`mkdirat`'s `mode` and the `mode` argument of `openat(O_CREAT)` are accepted and
**ignored**: the VFS has no `set_mode`, so ext2's defaults (`0o644` file,
`0o755` directory, `0o777` symlink) stand. `fchmodat` narrows the gap for the
current run: the kernel records `mode & 0o7777` keyed by inode and `fstat`
reports it until the program exits. It is **not** written to the inode, not
consulted by any access check, and not visible to the next program — a
`chmod` applet can report what `stat` says in the same run, and must not promise
persistence. `fchmodat` on a synthetic node (the root) is `-ENOTSUP`.

`ftruncate` requires a writable regular-file descriptor: 0/1/2 are `-EINVAL`,
a directory `-EISDIR`, a descriptor opened read-only `-EBADF`. Growing fills the
new range with zeroes; the open offset does not move.

### Structures (native, little-endian, no padding surprises)

```c
struct Stat {            // 24 bytes
    uint32_t mode;       // permission bits 0o7777, dir/exec checks use it
    uint32_t kind;       // PAGH_KIND_FILE=0, DIR=1, SYMLINK=2, OTHER=3
    uint64_t size;
    uint64_t ino;
};
struct Timespec { int64 seconds; int64 nanos; };          // 16 bytes
struct Uname { char sysname[16]; char nodename[16]; char release[16];
               char version[16]; char machine[16]; };     // 80 bytes
struct SockAddrIn {      // 16 bytes, native BSD layout
    uint16_t family;     // AF_INET=2, little-endian
    uint16_t port;       // network byte order (send port.to_be())
    uint8_t  address[4]; // IPv4 octets, wire order
    uint8_t  zero[8];    // padding, ignored by the kernel
};
// getdents64 entry: one record per directory entry, 8-byte aligned:
struct Dirent { uint64 ino; uint8 kind; uint8 name_len; uint16 reserved;
                char name[name_len]; /* then padding to 8 */ };
```

Flags: `O_RDONLY=0`, `O_WRONLY=1`, `O_RDWR=2`, `O_CREAT=0x40`,
`O_TRUNC=0x80`, `O_APPEND=0x100`; `AT_REMOVEDIR=0x200` (the `unlinkat` flag).
`mode` on `openat` is applied only with `O_CREAT` and is currently ignored (see
"Directory mutations").

Socket constants: `AF_INET=2`, `SOCK_STREAM=1`, `SHUT_RD=0`, `SHUT_WR=1`,
`SHUT_RDWR=2`; `SOL_SOCKET=1`, `IPPROTO_TCP=6`, `SO_REUSEADDR=2`, `SO_TYPE=3`,
`SO_ERROR=4`, `SO_KEEPALIVE=9`, `SO_RCVTIMEO=20`, `SO_SNDTIMEO=21`,
`TCP_NODELAY=1`.

### BSD sockets

The kernel implements one **active-open IPv4 TCP client**, so the socket ABI is
deliberately one connection wide:

* `AF_INET` + `SOCK_STREAM` + protocol 0 only. A second `socket()` while a
  socket descriptor is open is `-ENOSPC`; `close` frees the slot, and the next
  `socket()` gets a fresh local port.
* `connect` takes a 16-byte `SockAddrIn`; the kernel validates the user range,
  the exact length, the family, and reads the port big-endian. **IPv4 literals
  only** — there is no `getaddrinfo`, DNS syscall or `/etc/hosts` in this layer.
  `connect` polls the NIC until the handshake completes or the TCP client's own
  deadline fires; it returns only after success or a mapped errno.
* `send`/`sendto` are synchronous: the kernel waits for the peer's
  acknowledgement, polling the NIC, and a completed call means the bytes left
  the machine. `write` on a socket is the same path. `flags` must be 0; a
  non-zero `dest` is validated and ignored (the stream is already connected).
* `recv`/`recvfrom` are **non-blocking**: `-EAGAIN` means "nothing queued yet,
  call again", and 0 means the peer closed. `read` on a socket is the same path.
  A finite `src` is filled with the peer address and `*addrlen` is set to 16.
* `shutdown` closes both directions (no `SHUT_WR` half-close): later reads
  return 0 and later writes `-EPIPE`; the descriptor stays open until `close`.
* `setsockopt` accepts the common BSD options and ignores them (the kernel's
  bounds are fixed); `getsockopt` reports `SO_TYPE = SOCK_STREAM` and zero for
  the rest. Unknown level/option is `-ENOPROTOOPT`, never silently accepted.
* `bind`, `listen`, `accept`, `socketpair`, UDP (`SOCK_DGRAM`), IPv6, `poll`/
  `select`, `sendmsg`/`recvmsg` and TLS are **not implemented**; their numbers
  are unknown to the kernel and answer `-ENOSYS`.
* The kernel state happens to survive a program exit, but a run ends by closing
  any socket the program left open: the descriptor table is per run, and the
  next program starts with no connection.
* `connect` errno mapping: no DHCP lease → `-ENETUNREACH`; ARP/no route →
  `-EHOSTUNREACH`; peer RST during the handshake → `-ECONNREFUSED`; its deadline
  → `-ETIMEDOUT`; an already-connected descriptor → `-EISCONN`. After a reset or
  a peer FIN, `send` is `-EPIPE`/`-ECONNRESET` and `recv` returns 0 at EOF.
* The one connection is shared with the kernel's own HTTP client (`http`,
  `pkg`); a program cannot hold it while a shell command uses it, and one
  program runs at a time.

## 3. Errors

`EPERM=1 ENOENT=2 EIO=5 EBADF=9 EAGAIN=11 ENOMEM=12 EACCES=13 EFAULT=14
EEXIST=17 ENOTDIR=20 EISDIR=21 EINVAL=22 ENOSPC=28 EROFS=30 EPIPE=32
ENOSYS=38 ENOTEMPTY=39 ENAMETOOLONG=36 ELOOP=40 EOVERFLOW=75 ENOTSOCK=88
ENOPROTOOPT=92 ENOTSUP=95 EAFNOSUPPORT=97 ENETUNREACH=101 ECONNRESET=104
EISCONN=106 ENOTCONN=107 ETIMEDOUT=110 ECONNREFUSED=111 EHOSTUNREACH=113`.

## 4. Runtime (`libpagh`)

* `_start` (assembly) → `libpagh::start(sp)` → program's
  `extern "C" fn main(argc: i32, argv: *const *const u8, envp: *const *const u8) -> i32`.
* `alloc`: a bump allocator on `brk`; `alloc::vec::Vec`, `String`, `format!` work.
* `print!`/`println!` write to fd 1; `eprint!` to fd 2.
* `fs`: `File::open`, `File::create`, `File::set_len`, `read_to_end`,
  `read_to_string`, `write_all`, `create_dir`, `create_dir_all`, `remove_file`,
  `remove_dir`, `rename`, `hard_link`, `symlink`, `truncate`, `set_permissions`,
  `Dir::open`, iterator over `DirEntry`/`EntryKind`, `fs::read_link`, `fs::stat`.
* `env`: `args()` (with `argv[0]`), `env("PWD")`, `path::resolve` for relative paths.
* `io`: `Read`, `Write`, `Seek` traits and `Stdin`, `Stdout`.
* `net`: `Ipv4` (literal, `Ipv4::parse`) and `TcpStream::connect(host, port)`
  with `Read`/`Write` (non-blocking `read`: retry on `EAGAIN`), `read_to_end`
  and `shutdown`. No listener, UDP, DNS or TLS; see the socket subsection above.
* Panics print `program: panic at <loc>` to fd 2 and `exit(101)`; no unwinding.
* One process at a time: no threads, fork, signals, mmap, poll/select or dynamic linking; the one socket family is the connected IPv4 TCP client above.

## 5. Program pack format (`pagh-programs`)

See `pagh-programs/docs/package-format.md`. A package is one ELF plus a manifest
with the commands it provides; the installer (`pkg`) downloads it over HTTP,
verifies the SHA-256 from the index, stores it under `/mnt/pkg/`, installs the ELF
under `/mnt/bin/` and creates command symlinks. Packages are independent of apt.

## 6. Compatibility and versioning

`libpagh` and the kernel ABI are versioned together: `base_version = 1`. A
breaking change to an existing number, struct or flag is an ABI bump; this
file-mutation family and this socket family are additive (seven plus seven new
numbers, no changed field or flag on an existing call), so the base version
stays 1. The kernel keeps answering
`-ENOSYS` for numbers it does not know, so a program built against this table
fails with a clear error on an older kernel instead of crashing.
