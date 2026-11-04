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
const _: () = assert!(core::mem::size_of::<u64>() == 8);