//! Plain-old-data structures shared with the kernel, exactly as frozen in
//! `docs/abi.md` §2.
//!
//! They are native little-endian RV64 layouts with no libc padding rules
//! beyond C's: every field is naturally aligned, so the sizes below are also
//! compile-time asserted. The kernel and userland must agree byte for byte;
//! `tools/check_abi.py` checks the documented sizes, and the `const _`
//! assertions in this file make a Rust-side layout change fail the build.

/// `struct Stat` — 24 bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct Stat {
    /// Permission bits (`0o7777`). The kernel uses them for dir/exec checks.
    pub mode: u32,
    /// Entry kind: see [`EntryKind`].
    pub kind: u32,
    /// Size in bytes.
    pub size: u64,
    /// Inode number.
    pub ino: u64,
}

/// The `kind` field of [`Stat`] and of a directory entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum EntryKind {
    /// A regular file.
    File = 0,
    /// A directory.
    Dir = 1,
    /// A symbolic link.
    Symlink = 2,
    /// Anything else (device, fifo, socket, …).
    Other = 3,
}

impl EntryKind {
    /// Map the raw ABI `kind` value. Values outside 0..=3 are [`EntryKind::Other`].
    pub const fn from_raw(raw: u32) -> Self {
        match raw {
            0 => Self::File,
            1 => Self::Dir,
            2 => Self::Symlink,
            _ => Self::Other,
        }
    }

    /// The ABI `kind` value.
    pub const fn raw(self) -> u32 {
        self as u32
    }

    /// True for a directory.
    pub const fn is_dir(self) -> bool {
        matches!(self, Self::Dir)
    }

    /// True for a regular file.
    pub const fn is_file(self) -> bool {
        matches!(self, Self::File)
    }

    /// True for a symbolic link.
    pub const fn is_symlink(self) -> bool {
        matches!(self, Self::Symlink)
    }
}

impl Stat {
    /// The entry kind decoded from the raw field.
    pub const fn entry_kind(&self) -> EntryKind {
        EntryKind::from_raw(self.kind)
    }

    /// True for a directory.
    pub const fn is_dir(&self) -> bool {
        matches!(self.entry_kind(), EntryKind::Dir)
    }

    /// True for a regular file.
    pub const fn is_file(&self) -> bool {
        matches!(self.entry_kind(), EntryKind::File)
    }

    /// True for a symbolic link.
    pub const fn is_symlink(&self) -> bool {
        matches!(self.entry_kind(), EntryKind::Symlink)
    }
}

/// `struct Timespec` — 16 bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct Timespec {
    /// Seconds since the clock's epoch.
    pub seconds: i64,
    /// Nanoseconds within the second, `0..1_000_000_000`.
    pub nanos: i64,
}

/// `struct Uname` — 80 bytes, five 16-byte NUL-padded fields.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct Uname {
    /// Operating system name.
    pub sysname: [u8; 16],
    /// Network node name.
    pub nodename: [u8; 16],
    /// Kernel release.
    pub release: [u8; 16],
    /// Kernel version.
    pub version: [u8; 16],
    /// Hardware machine.
    pub machine: [u8; 16],
}

impl Default for Uname {
    fn default() -> Self {
        Self {
            sysname: [0; 16],
            nodename: [0; 16],
            release: [0; 16],
            version: [0; 16],
            machine: [0; 16],
        }
    }
}

/// Decode a NUL-padded fixed field as UTF-8.
///
/// The ABI does not promise that uname fields are valid UTF-8; a field that is
/// not valid text is reported as `""` rather than panicking. That is a
/// conservative choice: `uname` prints nothing for a corrupt field instead of
/// aborting the program.
fn field_str(field: &[u8; 16]) -> &str {
    let end = field.iter().position(|b| *b == 0).unwrap_or(field.len());
    core::str::from_utf8(&field[..end]).unwrap_or("")
}

impl Uname {
    /// `sysname` as text (normally `"pagh"`).
    pub fn sysname(&self) -> &str {
        field_str(&self.sysname)
    }
    /// `nodename` as text.
    pub fn nodename(&self) -> &str {
        field_str(&self.nodename)
    }
    /// `release` as text.
    pub fn release(&self) -> &str {
        field_str(&self.release)
    }
    /// `version` as text.
    pub fn version(&self) -> &str {
        field_str(&self.version)
    }
    /// `machine` as text (normally `"riscv64"`).
    pub fn machine(&self) -> &str {
        field_str(&self.machine)
    }
}

/// `AF_INET`: the only address family this ABI has.
pub const AF_INET: u16 = 2;

/// `SOCK_STREAM`: the only socket type this ABI has.
pub const SOCK_STREAM: u32 = 1;

/// `struct sockaddr_in` — 16 bytes, the native BSD layout.
///
/// `family` is a little-endian `u16` (`AF_INET`); `port` is stored in **network
/// byte order** in the low two bytes of a `u16` (the `htons` convention, so on
/// this little-endian target the in-memory bytes are big-endian); `address` is
/// the four IPv4 octets in wire order; `zero` is padding the kernel ignores.
///
/// The kernel accepts exactly this 16-byte structure and validates the family;
/// use [`SockAddrIn::new`] rather than filling the fields by hand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct SockAddrIn {
    /// Address family: always [`AF_INET`].
    pub family: u16,
    /// Port in network byte order (`port.to_be()`).
    pub port: u16,
    /// IPv4 address octets, in wire order.
    pub address: [u8; 4],
    /// Padding to the ABI's 16 bytes; the kernel ignores it.
    pub zero: [u8; 8],
}

impl SockAddrIn {
    /// Build a `sockaddr_in` for `address:port`.
    pub const fn new(address: [u8; 4], port: u16) -> Self {
        Self {
            family: AF_INET,
            // `to_be` on a little-endian target puts the high byte first in
            // memory, which is exactly the wire order the kernel parses.
            port: port.to_be(),
            address,
            zero: [0; 8],
        }
    }

    /// The port in host byte order.
    pub const fn port(&self) -> u16 {
        u16::from_be(self.port)
    }

    /// The IPv4 address octets.
    pub const fn address(&self) -> [u8; 4] {
        self.address
    }
}

/// Size of the fixed part of one `getdents64` record: `ino` (8) + `kind` (1) +
/// `name_len` (1) + `reserved` (2).
///
/// The ABI's `struct Dirent` is packed this way; it is **not** Linux's 19-byte
/// stateful header and must not be parsed as such.
pub const DIRENT_HEADER: usize = 12;

/// Records start on an 8-byte boundary and are padded to a multiple of 8.
pub const DIRENT_ALIGN: usize = 8;

/// The total on-wire size of a record whose name is `name_len` bytes.
///
/// Returns `None` on overflow (a hostile/corrupt header); callers turn that
/// into [`crate::errno::Errno::EIO`].
pub const fn dirent_size(name_len: usize) -> Option<usize> {
    let Some(unpadded) = DIRENT_HEADER.checked_add(name_len) else {
        return None;
    };
    let padding = (DIRENT_ALIGN - (unpadded % DIRENT_ALIGN)) % DIRENT_ALIGN;
    unpadded.checked_add(padding)
}

// The kernel and this crate must agree on these sizes; a field added to any of
// these structs without an ABI bump breaks the build here.
const _: () = assert!(core::mem::size_of::<Stat>() == 24);
const _: () = assert!(core::mem::size_of::<Timespec>() == 16);
const _: () = assert!(core::mem::size_of::<Uname>() == 80);
const _: () = assert!(core::mem::size_of::<SockAddrIn>() == 16);
const _: () = assert!(core::mem::size_of::<u64>() == 8);


/// `struct PollFd` — 8 bytes (the `poll(2)` descriptor).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct PollFd {
    /// The descriptor being watched.
    pub fd: i32,
    /// Requested events (`POLLIN`/`POLLOUT`).
    pub events: i16,
    /// Returned events.
    pub revents: i16,
}

const _: () = assert!(core::mem::size_of::<PollFd>() == 8);


/// `struct sigaction` — 32 bytes (pagh native layout: handler, flags, mask, restorer).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct SigAction {
    /// Handler address, [`SIG_DFL`]/[`SIG_IGN`], or a user function.
    pub handler: usize,
    /// Flags (only `SA_RESTORER` is accepted when a real handler is installed).
    pub flags: usize,
    /// Signals blocked while the handler runs.
    pub mask: u64,
    /// User restorer called by the handler's `ret`; required for a handler.
    pub restorer: usize,
}

/// `SIG_DFL`: default action (terminate the run for fatal signals).
pub const SIG_DFL: usize = 0;
/// `SIG_IGN`: discard the signal.
pub const SIG_IGN: usize = 1;
/// `sigaction` flag: the handler returns through `restorer`.
pub const SA_RESTORER: usize = 0x0400_0000;
/// `rt_sigprocmask` how: add to the blocked mask.
pub const SIG_BLOCK: usize = 0;
/// `rt_sigprocmask` how: remove from the blocked mask.
pub const SIG_UNBLOCK: usize = 1;
/// `rt_sigprocmask` how: replace the blocked mask.
pub const SIG_SETMASK: usize = 2;

const _: () = assert!(core::mem::size_of::<SigAction>() == 32);


/// `struct sigaltstack` — 24 bytes (`ss_sp`, `ss_flags`, padding, `ss_size`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct SigAltStack {
    /// Stack base, or 0 with `SS_DISABLE`.
    pub sp: usize,
    /// `SS_ONSTACK`/`SS_DISABLE`.
    pub flags: u32,
    /// Reserved padding (keeps `size` 8-byte aligned).
    pub pad: u32,
    /// Stack size in bytes.
    pub size: usize,
}

/// Full 128-byte `siginfo_t` as delivered in `a1` (pagh layout).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct SigInfo {
    /// Signal number.
    pub signo: i32,
    /// `errno` associated with the signal (0 here).
    pub errno: i32,
    /// `SI_USER`/`SI_KERNEL`/`SI_TKILL`/fault codes.
    pub code: i32,
    /// Reserved to 16 bytes.
    pub pad0: i32,
    /// Sender pid or fault address (union payload).
    pub value: u64,
    /// `si_status` for SIGCHLD (0: no child-exit source yet).
    pub status: i32,
    /// Reserved padding to 128 bytes.
    pub pad1: [u8; 100],
}

/// The `mcontext` inside a delivered `ucontext_t`: 32 GPRs and the PC.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct MContext {
    /// x0..x31 as saved in the trap frame.
    pub gregs: [u64; 32],
    /// Interrupted program counter.
    pub pc: u64,
}

/// The 288-byte `ucontext_t` a `SA_SIGINFO` handler receives in `a2`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct UContext {
    /// Flags (0 today).
    pub flags: u64,
    /// `uc_link` (0).
    pub link: u64,
    /// Blocked mask at delivery.
    pub sigmask: u64,
    /// Register snapshot.
    pub mcontext: MContext,
}

/// `SS_DISABLE`: no alternate stack.
pub const SS_DISABLE: u32 = 2;
/// `SS_ONSTACK`: a signal handler is currently running on the alternate stack.
pub const SS_ONSTACK: u32 = 1;
/// `SA_SIGINFO`: the handler takes `(signo, *siginfo, *ucontext)`.
pub const SA_SIGINFO: usize = 0x0000_0004;
/// `SA_ONSTACK`: deliver on the thread's alternate stack.
pub const SA_ONSTACK: usize = 0x0800_0000;
/// `SA_RESTART`: restart futex/nanosleep after an interrupted wait.
pub const SA_RESTART: usize = 0x1000_0000;
/// `SA_NODEFER`: do not block the delivered signal inside its handler.
pub const SA_NODEFER: usize = 0x4000_0000;
/// `SA_RESETHAND`: reset the handler to `SIG_DFL` on delivery.
pub const SA_RESETHAND: usize = 0x8000_0000;

const _: () = assert!(core::mem::size_of::<SigAltStack>() == 24);
const _: () = assert!(core::mem::size_of::<SigInfo>() == 128);
const _: () = assert!(core::mem::size_of::<MContext>() == 264);
const _: () = assert!(core::mem::size_of::<UContext>() == 288);
