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

Maximum open files per process: **16**, including 0/1/2. fd 0 = console input,
1 = console output, 2 = console diagnostics (all three are the same UART/screen;
`read(0)` also drains the virtio-keyboard).

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
// getdents64 entry: one record per directory entry, 8-byte aligned:
struct Dirent { uint64 ino; uint8 kind; uint8 name_len; uint16 reserved;
                char name[name_len]; /* then padding to 8 */ };
```

Flags: `O_RDONLY=0`, `O_WRONLY=1`, `O_RDWR=2`, `O_CREAT=0x40`,
`O_TRUNC=0x80`, `O_APPEND=0x100`. `mode` on `openat` is applied only with
`O_CREAT` (subject to the kernel's umask).

## 3. Errors

`EPERM=1 ENOENT=2 EIO=5 EBADF=9 EAGAIN=11 ENOMEM=12 EACCES=13 EFAULT=14
EEXIST=17 ENOTDIR=20 EISDIR=21 EINVAL=22 ENOSPC=28 EROFS=30 ENOSYS=38
ENAMETOOLONG=36 ELOOP=40 EOVERFLOW=75 ENOTSUP=95`.

## 4. Runtime (`libpagh`)

* `_start` (assembly) → `libpagh::start(sp)` → program's
  `extern "C" fn main(argc: i32, argv: *const *const u8, envp: *const *const u8) -> i32`.
* `alloc`: a bump allocator on `brk`; `alloc::vec::Vec`, `String`, `format!` work.
* `print!`/`println!` write to fd 1; `eprint!` to fd 2.
* `fs`: `File::open`, `File::create`, `read_to_end`, `read_to_string`, `write_all`,
  `Dir::open`, iterator over `DirEntry`/`EntryKind`, `fs::read_link`, `fs::stat`.
* `env`: `args()` (with `argv[0]`), `env("PWD")`, `path::resolve` for relative paths.
* `io`: `Read`, `Write`, `Seek` traits and `Stdin`, `Stdout`.
* Panics print `program: panic at <loc>` to fd 2 and `exit(101)`; no unwinding.
* One process at a time: no threads, fork, signals, sockets, mmap, dynamic linking.

## 5. Program pack format (`pagh-programs`)

See `pagh-programs/docs/package-format.md`. A package is one ELF plus a manifest
with the commands it provides; the installer (`pkg`) downloads it over HTTP,
verifies the SHA-256 from the index, stores it under `/mnt/pkg/`, installs the ELF
under `/mnt/bin/` and creates command symlinks. Packages are independent of apt.

## 6. Compatibility and versioning

`libpagh` and the kernel ABI are versioned together: `base_version = 1`. Adding a
syscall or struct field is an ABI bump; the kernel keeps answering `-ENOSYS` for
numbers it does not know, so an old runtime fails with a clear error instead of
crashing.
