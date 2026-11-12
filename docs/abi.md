# pagh native userland ABI (libpagh, RISC-V RV64)

Status: **frozen for the first ports**. The kernel (`src/user/syscall.rs`) and
`libpagh` must match this document; a mismatch is a bug in whichever side drifted.

This contract is the **pagh-riscv** kernel's
([`ccclosed/pagh-riscv`](https://github.com/ccclosed/pagh-riscv), the RV64 kernel
this repository's examples and live tests run on). It is *not* the ABI of
`Pagh-OS`, the separate x86_64 Limine/UEFI kernel with a Linux x86_64
compatibility layer: that layer implements Linux, not this document, and the two
are deliberately unrelated.

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
| 198 | `socket` | `domain` (`AF_INET=2`), `type` (`SOCK_STREAM=1` or `SOCK_DGRAM=2`, optionally ORed with `SOCK_NONBLOCK=0x800`), `protocol` (0) | fd ≥ 3; `-EAFNOSUPPORT` for any other combination; one active-open client stream descriptor at a time (a second `socket()` while it exists is `-ENOSPC`; listeners and accepted connections have their own descriptors); any number of datagram sockets |
| 200 | `bind` | `fd` (UDP or a not-yet-listening stream socket), `*SockAddrIn`, `addrlen` (must be 16) | 0; port 0 = ephemeral; UDP: `-EADDRINUSE`, `-EADDRNOTAVAIL`, `-EINVAL` (already bound); stream: the port is reserved for `listen`, a second listener reservation is `-EADDRINUSE`, binding a connected/listening descriptor is `-EINVAL`; `-EFAULT` |
| 201 | `listen` | `fd` (stream socket bound with 200), `backlog` | 0; `backlog` is clamped to `1..=4` (one listener, fixed four-entry accept queue); a second `listen` updates the backlog; unbound/connected descriptor `-EINVAL`; UDP `-ENOTSUP` |
| 202 | `accept` | `fd` (listening stream), `*SockAddrIn` (optional), `*socklen_t` (optional, capacity ≥ 16) | new fd ≥ 3 and the peer's 16-byte address with `*addrlen = 16`; blocks polling up to 30 s then `-EAGAIN`; a `SOCK_NONBLOCK` listener answers `-EAGAIN` immediately; `-EINVAL`/`-ENOTSUP`/`-EFAULT` |
| 203 | `connect` | `fd`, `*SockAddrIn`, `addrlen` (must be 16) | 0; `-EINVAL`, `-EAFNOSUPPORT`, `-ENOTSOCK`, `-EISCONN`, `-ENETUNREACH`, `-EHOSTUNREACH`, `-ECONNREFUSED`, `-ETIMEDOUT`, `-EAGAIN`; a UDP descriptor is `-ENOTSUP` |
| 206 | `sendto` | `fd`, `buf`, `len`, `flags` (0; UDP also accepts `MSG_DONTWAIT=0x40`), `dest` (TCP: 0 or validated/ignored; UDP: mandatory), `addrlen` | bytes sent; TCP: `-ENOTCONN`, `-EPIPE`, `-ECONNRESET`, `-ETIMEDOUT`; UDP: `-EDESTADDRREQ`, `-EMSGSIZE` (>1024 B), `-EHOSTUNREACH`, `-ENETUNREACH`; the `send` shape |
| 207 | `recvfrom` | `fd`, `buf`, `len`, `flags` (TCP: 0; UDP: 0 or `MSG_DONTWAIT`), `src` (0, or filled with the peer/source), `*addrlen` | bytes; TCP: **`-EAGAIN` when nothing is queued yet**, 0 = peer closed; UDP: blocks (bounded ~10 s) or `-EAGAIN` with `MSG_DONTWAIT`/`SOCK_NONBLOCK`; the `recv` shape |
| 208 | `setsockopt` | `fd`, `level`, `optname`, `optval`, `optlen` | 0 for `SO_REUSEADDR`, `SO_KEEPALIVE`, `SO_RCVTIMEO`, `SO_SNDTIMEO`, `TCP_NODELAY` (validated and ignored); `-ENOPROTOOPT` otherwise |
| 209 | `getsockopt` | `fd`, `level`, `optname`, `optval`, `*optlen` | 0 for `SO_TYPE` (`SOCK_STREAM`/`SOCK_DGRAM`), `SO_ERROR` (0), `SO_RCVTIMEO`/`SO_SNDTIMEO` (0), `TCP_NODELAY` (0); `-ENOPROTOOPT` otherwise |
| 210 | `shutdown` | `fd`, `how` (0 read, 1 write, 2 both) | 0; both directions close (no half-close), the descriptor stays open: later `read` is 0, `write` is `-EPIPE` |
| 1000 | `fork` | — | pagh-native number: duplicate the process (deep-copied user pages into a new Sv39 root/ASID, fd table with pipe refs/socket ENOTSUP, handlers/masks/altstack; no pending/siblings). Parent gets pid > 0, child 0, both continue at `sepc+4`; `-EAGAIN`/`-EINVAL` |
| 221 | `execve` | `path` (absolute), `argv` (NULL-terminated user array), `envp` | replaces image/stack/heap/mmap; pid/ppid/fds/blocked mask kept, handlers reset, altstack off, pending cleared, siblings retired; returns only on error (`-ENOENT`/`-ENOEXEC`/`-EISDIR`/`-E2BIG`/`-EFAULT`/`-ENOMEM`) without changing the image |
| 260 | `wait4` | `pid` (>0 specific, 0/-1 any), `*status`, `options` (`WNOHANG=1`; other bits `-EINVAL`), `rusage` (ignored) | pid of the reaped child; 0 when `WNOHANG` has no exited child; status `code<<8` or signal number; `-ECHILD`, `-EFAULT` |
| 173 | `getppid` | — | parent pid (0 for the shell-started program; the fork parent's pid after fork; preserved by execve) |

| 222 | `mmap` | `addr` (hint, 0 = pick), `length`, `prot` (`PROT_READ=1`/`WRITE=2`/`EXEC=4`), `flags` (`MAP_PRIVATE=2\|MAP_ANONYMOUS=0x20`), `fd` (-1), `offset` (0) | mapped address; `-ENOMEM`, `-EINVAL`, `-ENOTSUP` (shared/file-backed) |
| 215 | `munmap` | `addr` (page-aligned), `length` | 0; `-EINVAL` for a range that is not this run's mapping |
| 226 | `mprotect` | `addr` (page-aligned), `length`, `prot` | 0; W^X is rejected with `-EINVAL` |
| 59 | `pipe2` | `*fds[2]`, `flags` (must be 0) | 0; read end non-blocking (`-EAGAIN` when empty), EOF once all write ends close |
| 23 | `dup` | `fd` | new fd |
| 24 | `dup2` | `oldfd`, `newfd` | `newfd` (asm-generic number 24 also answers `dup3`) |
| 7 | `poll` | `*PollFd` (at most 64 entries), `nfds`, `timeout_ms` (0 = scan, negative = wait) | ready count (`0` = timeout); `POLLIN`/`POLLOUT`/`POLLERR`/`POLLHUP`/`POLLNVAL`; `nfds > 64` is `-EINVAL`, a negative `fd` is skipped |
| 72 | `ppoll` | `*PollFd`, `nfds`, `*timespec` (treated as ms), mask (ignored) | as `poll` |
| 220 | `clone` | `flags` (`CLONE_VM=0x100\|CLONE_THREAD=0x10000` required, `CLONE_FS/FILES/SIGHAND` accepted, `SETTLS` → `-ENOTSUP`), `stack` (16-aligned, mapped R/W), `entry` (mapped X), `arg`, `gp`, `tp` | child tid; child returns 0 and runs `entry(arg)`; `-EINVAL`, `-EAGAIN` |
| 98 | `futex` | `uaddr` (4-aligned), `op` (`WAIT=0`/`WAKE=1` plus `PRIVATE=128`), `val`, `timeout` (0 or `*Timespec`) | `WAIT`: 0, `-EAGAIN` (value changed), `-ETIMEDOUT`; `WAKE`: number woken; other ops `-ENOSYS` |
| 178 | `gettid` | — | thread id |
| 96 | `set_tid_address` | `ptr` | tid; the word is not cleared on exit |
| 94 | `exit_group` | `code` | ends every thread of the run, does not return |
| 129 | `kill` | `pid` (>0 thread id, 0 thread group, -1 run), `sig` | 0; `-ESRCH`, `-EINVAL`, `-ENOSYS` for job-control signals |
| 131 | `tgkill` | `tgid`, `tid`, `sig` | 0; `-ESRCH` |
| 134 | `rt_sigaction` | `signum` (1..=31), `*SigAction` (`handler`/`flags`/`mask`/`restorer`), `*oldact`, `sigsetsize=8` | 0; real handlers require `SA_RESTORER`; `SIGKILL`/`SIGSTOP` → `-EINVAL`; unknown flags `-EINVAL` |
| 135 | `rt_sigprocmask` | `how` (`BLOCK=0`/`UNBLOCK=1`/`SETMASK=2`), `*mask` (u64), `*oldmask`, `sigsetsize=8` | 0 |
| 139 | `rt_sigreturn` | — | restores the frame the kernel armed for this thread; `-EINVAL` outside a handler |
| 132 | `sigaltstack` | `*SigAltStack` (`ss_sp`/`ss_flags`/`ss_size`), `*old` | 0; `SS_DISABLE`/`SS_ONSTACK`; `-ENOMEM`/`-EINVAL` for a bad range |
| 278 | `getrandom` | `buf`, `len`, `flags` (must be 0) | bytes written; `-EINVAL` (unknown flag), `-EFAULT` (bad buffer, nothing written), `-EAGAIN` (the pool was never seeded); returns at most 65536 bytes per call — a **short read** otherwise, which the caller repeats |
| 1100 | `tls_connect` | `tcp_fd` (a **connected** stream socket or an accepted connection), `host` (bytes, not NUL-terminated), `host_len` (1..=255), `flags` (must be 0) | 0; the same descriptor now carries a TLS 1.3 session. Fail-closed: the handshake aborts unless the chain reaches the committed CA bundle, the SAN authorizes `host`, the clock gate passes and `CertificateVerify` checks out. `-EBADF`/`-ENOTSOCK`/`-ENOTCONN`/`-EISCONN`/`-ENOTSUP` (wrong descriptor), `-EINVAL` (flags, host name), `-EFAULT`, `-EAGAIN` (no entropy seed), `-ENOTSUP` (no wall clock), `-EPROTO` (handshake), `-EKEYREJECTED` (certificate), `-ENOSPC` (no free session) |
| 1101 | `tls_write` | `fd` (a TLS session), `buf`, `len` | plaintext bytes encrypted and handed to the record layer (short only after a failure); `-EINVAL` (not a TLS fd), `-EFAULT`, `-EPIPE` (closed), `-ECONNRESET`, `-ETIMEDOUT`, `-EAGAIN` (a concurrent call on the same session) |
| 1102 | `tls_read` | `fd` (a TLS session), `buf`, `len` | plaintext bytes decrypted; **blocks** (bounded by the kernel's ~20 s idle budget) until some are available, returns at most 4096 per call; `0` = end of stream (`close_notify`); `-ECONNRESET` when the stream ended **without** one (truncation is not EOF), `-ETIMEDOUT`, `-EINVAL`, `-EFAULT`, `-EAGAIN` |
| 1103 | `tls_close` | `fd` (a TLS session) | 0; sends `close_notify` (best effort), releases the session buffers and closes the descriptor. `-EINVAL` when the fd is not a session, `-EBADF` when it is closed |




Maximum open files per process: **16**, including 0/1/2. fd 0 = console input,
1 = console output, 2 = console diagnostics (all three are the same UART/screen;
`read(0)` also drains the virtio-keyboard).

On a socket descriptor, `read` is `recv` with `flags = 0`, `write` is `send`
with `flags = 0`, and `close` closes the TCP connection and releases the
descriptor. `lseek`/`ftruncate` answer `-EINVAL`, `fstat` reports
`kind = OTHER` like the console.

### TLS descriptors

`tls_connect` turns a connected stream descriptor into a **TLS session**; the fd
number does not change and everything above the record layer is plaintext:

* `read`/`recv` decrypt, `write`/`send` encrypt (they are the same code as
  1102/1101, so `send` on a TLS fd with a non-zero `dest` is `-ENOTSUP` — the
  session already has a peer);
* `poll` reports `POLLIN` when plaintext is buffered **or** the socket has bytes
  (which may be a partial record, so a `read` after `POLLIN` can still wait for
  the rest of it, bounded by the idle budget) and `POLLHUP` once the peer sent
  `close_notify`;
* `shutdown` sends `close_notify`: both directions close, the descriptor stays
  open, a later `read` is `0` and a later `write` is `-EPIPE`;
* `tls_close` (1103) is the graceful close. `close(2)` releases the session
  *without* the alert, because it also runs at process teardown and may not wait
  for a peer;
* `dup`/`fork` do **not** duplicate a session: the copy is `-ENOTSUP` (a session
  owns its buffers and key schedule, and two owners would tear it down under
  each other). At most **4** sessions exist kernel-wide at once (`-ENOSPC`);
  each costs ~50 KiB of kernel heap until it is closed;
* a **concurrent** call on the same session from another thread of the process
  is `-EAGAIN` rather than a second reader of one record stream;
* there is no flag anywhere that relaxes verification, and no session is created
  unless a certificate was verified during that handshake.

A failed handshake abandons the connection (RFC 8446 §6.1): the socket is torn
down and the descriptor is left as a tombstone — `-ENOTSUP` for I/O, `close`
releases it — so a caller that wants to retry opens a new socket.

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
struct PollFd {          // 8 bytes, the poll(2) descriptor
    int32_t  fd;         // negative: skipped, revents stays 0
    int16_t  events;     // requested: POLLIN=0x1, POLLOUT=0x4
    int16_t  revents;    // returned: POLLIN/POLLOUT/POLLERR/POLLHUP/POLLNVAL
};
struct SigAction {       // 32 bytes, pagh native layout (not Linux's sigaction)
    uint64_t handler;    // SIG_DFL=0, SIG_IGN=1, or a user function address
    uint64_t flags;      // SA_* below; a real handler requires SA_RESTORER
    uint64_t mask;       // signals blocked while the handler runs
    uint64_t restorer;   // user code the handler's `ret` jumps to (rt_sigreturn)
};
struct SigAltStack {     // 24 bytes
    uint64_t ss_sp;      // stack base
    int32_t  ss_flags;   // SS_ONSTACK=1, SS_DISABLE=2
    int32_t  pad;
    uint64_t ss_size;    // >= MINSIGSTKSZ (4 KiB)
};
struct SigInfo {         // 128 bytes, the SA_SIGINFO `siginfo_t`
    int32_t  si_signo;   // 0
    int32_t  si_errno;   // 4, always 0
    int32_t  si_code;    // 8: SI_USER=0, SI_KERNEL=0x80, SI_TKILL=-6, SEGV_MAPERR=1, TRAP_BRKPT=1, BUS_ADRALN=1
    int32_t  pad;        // 12
    uint64_t payload;    // 16: si_pid|si_uid (u32+u32) for kill/tgkill, si_addr for a fault
    int32_t  si_status;  // 24: SIGCHLD only (there is no child-exit producer, so 0)
    uint8_t  pad1[100];  // 28..128
};
struct MContext {        // 264 bytes, inside ucontext_t
    uint64_t gregs[32];  // x0..x31 in RISC-V numeric order (x0 = 0)
    uint64_t pc;         // interrupted sepc
};
struct UContext {        // 288 bytes (pagh layout, this is not Linux's ucontext_t)
    uint64_t flags;      // 0
    uint64_t link;       // &ucontext of the previous frame, 0 outermost
    uint64_t sigmask;    // blocked mask in force inside the handler
    struct MContext mcontext;  // 24..288
};
// getdents64 entry: one record per directory entry, 8-byte aligned:
struct Dirent { uint64 ino; uint8 kind; uint8 name_len; uint16 reserved;
                char name[name_len]; /* then padding to 8 */ };
```

`SigAction`, `SigAltStack`, `SigInfo` and `UContext` are the pagh-native layouts;
they are deliberately **not** the Linux RISC-V ones. `rt_sigaction`'s `sigsetsize`
is exactly 8 (a `u64` mask), signals are `1..=31` with `SIGKILL`/`SIGSTOP`
unkillable, and a real handler must carry `SA_RESTORER` (`0x0400_0000`) plus an
executable restorer. Accepted `SA_*` bits are exactly `SA_RESTORER`,
`SA_ONSTACK=0x0800_0000`, `SA_RESTART=0x1000_0000`, `SA_NODEFER=0x4000_0000`,
`SA_RESETHAND=0x8000_0000`, `SA_SIGINFO=0x4`; any other bit is `-EINVAL`.
`rt_sigprocmask` how: `SIG_BLOCK=0`, `SIG_UNBLOCK=1`, `SIG_SETMASK=2`.

`poll`/`ppoll` accept at most 64 `PollFd` entries (`nfds > 64` is `-EINVAL`);
`timeout_ms == 0` scans once, a negative timeout waits, a zero return is a
timeout (not an error), and every entry's `revents` is written on return.

Flags: `O_RDONLY=0`, `O_WRONLY=1`, `O_RDWR=2`, `O_CREAT=0x40`,
`O_TRUNC=0x80`, `O_APPEND=0x100`; `AT_REMOVEDIR=0x200` (the `unlinkat` flag).
`mode` on `openat` is applied only with `O_CREAT` and is currently ignored (see
"Directory mutations").

Socket constants: `AF_INET=2`, `SOCK_STREAM=1`, `SOCK_DGRAM=2`,
`SOCK_NONBLOCK=0x800`, `MSG_DONTWAIT=0x40`, `SHUT_RD=0`, `SHUT_WR=1`,
`SHUT_RDWR=2`; `SOL_SOCKET=1`, `IPPROTO_TCP=6`, `SO_REUSEADDR=2`, `SO_TYPE=3`,
`SO_ERROR=4`, `SO_KEEPALIVE=9`, `SO_RCVTIMEO=20`, `SO_SNDTIMEO=21`,
`TCP_NODELAY=1`. The largest UDP payload is 1024 bytes.

### BSD sockets

The kernel implements one **active-open IPv4 TCP client** and userland
**UDP datagram sockets**:

* `AF_INET` + `SOCK_STREAM` + protocol 0 is TCP. One **active-open client**
  descriptor may exist at a time (a second `socket()` while it exists is
  `-ENOSPC`); listeners and accepted connections are separate descriptor kinds,
  so a server can hold a listener and several accepted streams. `close` frees
  the slot, and the next client `socket()` gets a fresh local port.
  `AF_INET` + `SOCK_DGRAM` creates a UDP socket; any number may be open, and
  `SOCK_NONBLOCK` may be ORed into the type.
* `bind` (200) gives a UDP socket a local port: `addrlen` is exactly 16, the
  address is `0.0.0.0` or the interface's DHCP lease, and port 0 picks an
  ephemeral port. A port owned by another socket is `-EADDRINUSE`; binding
  twice is `-EINVAL`. The first `sendto` on an unbound socket auto-binds it.
  On a stream socket the same call reserves the local port for `listen`
  (one listener reservation at a time). Binding a connected socket or a
  listener is `-EINVAL`.
* `listen` (201) turns that reserved port into a passive socket. The kernel has
  one listener and a fixed four-entry accept queue; `backlog` is clamped to
  `1..=4`, and a handshake that completes while the queue is full is reset.
  `accept` (202) returns the oldest queued connection as a new `SOCK_STREAM`
  descriptor, with the peer's `SockAddrIn`; the wait polls the NIC for up to
  30 s and then returns `-EAGAIN`, while a listener created with
  `SOCK_NONBLOCK` returns `-EAGAIN` immediately on an empty queue. Accepted
  streams use the ordinary `read`/`write`/`shutdown` paths, and closing one
  sends FIN (or RST for a half-open passive connection).
* `connect` takes a 16-byte `SockAddrIn`; the kernel validates the user range,
  the exact length, the family, and reads the port big-endian. **IPv4 literals
  only** — there is no `getaddrinfo`, DNS syscall or `/etc/hosts` in this layer.
  `connect` polls the NIC until the handshake completes or the TCP client's own
  deadline fires; it returns only after success or a mapped errno.
* TCP `send`/`sendto` are synchronous: the kernel waits for the peer's
  acknowledgement, polling the NIC, and a completed call means the bytes left
  the machine. `write` on a TCP socket is the same path. `flags` must be 0; a
  non-zero `dest` is validated and ignored (the stream is already connected).
* TCP `recv`/`recvfrom` are **non-blocking**: `-EAGAIN` means "nothing queued
  yet, call again", and 0 means the peer closed. `read` on a TCP socket is the
  same path. A finite `src` is filled with the peer address and `*addrlen` is
  set to 16.
* UDP `sendto` requires a destination, accepts 0 or `MSG_DONTWAIT`, and a payload
  up to 1024 bytes (`-EMSGSIZE` above that; no fragmentation). UDP `recvfrom`
  blocks by default — the kernel polls the NIC with interrupts enabled under a
  ~10 s budget, then answers `-EAGAIN` — and `MSG_DONTWAIT` (or the descriptor's
  `SOCK_NONBLOCK`) makes it a single poll. The source address is filled and
  `*addrlen` is set to 16. Each bound port has a 4-datagram queue; a fifth
  arrival drops the oldest. `read` on a UDP socket is `recv` with no source and
  no flags; `write` is `-EDESTADDRREQ` (no UDP `connect`).
* `shutdown` closes both directions (no `SHUT_WR` half-close): later reads
  return 0 and later writes `-EPIPE`; the descriptor stays open until `close`.
* `setsockopt` accepts the common BSD options and ignores them (the kernel's
  bounds are fixed); `getsockopt` reports `SO_TYPE = SOCK_STREAM` and zero for
  the rest. Unknown level/option is `-ENOPROTOOPT`, never silently accepted.
* `socketpair`, UDP `connect`, IPv6, `select`, `sendmsg`/`recvmsg` and TLS are
  **not implemented**; their numbers are unknown to the kernel and answer
  `-ENOSYS`. UDP `connect` is `-ENOTSUP`. The TCP limits are exact: one
  listener, one bind reservation, backlog ≤ 4, eight connection slots shared by
  the active client and passive connections, one 4 KiB receive buffer per
  connection, no out-of-order reassembly, no congestion control and no
  TIME_WAIT. A listener close resets connections that were never accepted and
  leaves accepted streams alone.
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

`EPERM=1 ENOENT=2 ESRCH=3 EINTR=4 EIO=5 E2BIG=7 ENOEXEC=8 EBADF=9
ECHILD=10
EAGAIN=11 ENOMEM=12 EACCES=13 EFAULT=14 EEXIST=17 ENOTDIR=20 EISDIR=21
EINVAL=22 ENOSPC=28 EROFS=30 EPIPE=32 ENAMETOOLONG=36 ENOSYS=38
ENOTEMPTY=39 ELOOP=40 EPROTO=71 EOVERFLOW=75 ENOTSOCK=88 EDESTADDRREQ=89
EMSGSIZE=90 ENOPROTOOPT=92 ENOTSUP=95 EAFNOSUPPORT=97 EADDRINUSE=98
EADDRNOTAVAIL=99 ENETUNREACH=101 ECONNRESET=104 EISCONN=106 ENOTCONN=107
ETIMEDOUT=110 ECONNREFUSED=111 EHOSTUNREACH=113 EKEYREJECTED=129`.

`ESRCH`, `EINTR`, `E2BIG`, `ENOEXEC`, `ECHILD`, `EDESTADDRREQ`, `EMSGSIZE`,
`EADDRINUSE` and `EADDRNOTAVAIL` were already named by the syscall rows in §2
(`kill`/`tgkill`, the blocking calls interrupted by a handler, `execve`
— `E2BIG` is its over-long argv/envp refusal —, `wait4`, `bind` and `sendto`),
and the kernel has always returned them; §3
simply omitted their numbers. Recording them here is a documentation fix, not a
new ABI shape: no existing number or semantic changed.

`EPROTO` is a TLS handshake or record that did not follow TLS 1.3;
`EKEYREJECTED` is the trust decision refusing the server — the chain does not
reach the committed CA bundle, the SAN does not authorize the requested name,
the validity window fails the clock gate, or `CertificateVerify` did not check
out. They are distinct on purpose: "the peer is not who it claims" is not "the
protocol broke".

## 4. Runtime (`libpagh`)

* `_start` (assembly) → `libpagh::start(sp)` → program's
  `extern "C" fn main(argc: i32, argv: *const *const u8, envp: *const *const u8) -> i32`.
* `alloc`: a bump allocator on `brk`; `alloc::vec::Vec`, `String`, `format!` work.
* `print!`/`println!` write to fd 1; `eprint!` to fd 2.
* `fs`: `File::open`, `File::create`, `File::set_len`, `read_to_end`,
  `read_to_string`, `write`/`write_all`, `create_dir`, `create_dir_all`,
  `remove_file`, `remove_dir`, `rename`, `hard_link`, `symlink`, `truncate`,
  `set_permissions`, `Dir::open`, `read_dir` (std-shaped iterator over
  `DirEntry`: `file_name`, `path`, `file_type`, `metadata`), `fs::read_link`,
  `fs::stat`.
* `env`: `args()` (with `argv[0]`), `env("PWD")`, `path::resolve` for relative paths.
* `io`: `Read`, `Write`, `Seek` traits and `Stdin`, `Stdout`.
* `net`: `Ipv4` (literal, `Ipv4::parse`) and `TcpStream::connect(host, port)`
  with `Read`/`Write` (non-blocking `read`: retry on `EAGAIN`), `read_to_end`
  and `shutdown`, plus `bind`/`listen`/`accept` for a TCP server and
  `bind`/`sendto`/`recvfrom` for UDP. No `getaddrinfo`; see the socket
  subsection above. `dns::resolve` sends one `A` query over a UDP socket.
* `tls`: `TlsStream::connect(TcpStream, host)` performs the handshake **in the
  kernel** (syscall 1100) and then reads/writes plaintext through the descriptor;
  there is no API here that weakens the verification, because there is no such
  flag in the ABI. `random::fill` is `getrandom`.
* Panics print `program: panic at <loc>` to fd 2 and `exit(101)`; no unwinding.
* The ABI has threads, fork/exec/wait, signals, mmap/mprotect, poll, futexes and a bounded TCP server API. What it still lacks is a dynamic loader in the kernel (userland `ld-pagh` covers that) and file-backed mmap.

## 5. Program pack format (`pagh-programs`)

See `pagh-programs/docs/package-format.md`. A package is one ELF plus a manifest
with the commands it provides; the installer (`pkg`) downloads it over HTTP,
verifies the SHA-256 from the index, stores it under `/mnt/pkg/`, installs the ELF
under `/mnt/bin/` and creates command symlinks. Packages are independent of apt.

## 6. Compatibility and versioning

`libpagh` and the kernel ABI are versioned together: `base_version = 1`. A
breaking change to an existing number, struct or flag is an ABI bump; the
file-mutation, socket, thread, signal, process and (0.21.0) entropy/TLS families
are additive — new numbers, no changed field or flag on an existing call — so
the base version stays 1. The kernel keeps answering `-ENOSYS` for numbers it
does not know, so a program built against this table fails with a clear error on
an older kernel instead of crashing.

The 0.21.0 additions are `getrandom` (278) and the four TLS calls (1100–1103).
The one semantic addition to an *existing* shape is that `read`/`write`/`send`/
`recv`/`poll`/`shutdown`/`close` now also accept a TLS descriptor; a descriptor
that never had `tls_connect` on it behaves exactly as before, so the addition is
backward compatible.

The §3 error list is additive in the same way. The numbers added in the 0.6.1
documentation fix (`ESRCH`, `EINTR`, `E2BIG`, `ENOEXEC`, `ECHILD`,
`EDESTADDRREQ`, `EMSGSIZE`, `EADDRINUSE`, `EADDRNOTAVAIL`) were always
returned by the kernel;
listing them changes no syscall's behaviour and `base_version` stays 1.
