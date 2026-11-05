//! The frozen error set from `docs/abi.md` §3.
//!
//! The kernel returns `-errno` in `a0`; [`Errno::from_raw`] maps the positive
//! value back. The variants and their numbers are checked against the ABI
//! document by `tools/check_abi.py`, so adding a number here without adding it
//! to the contract (or vice versa) fails the check.

use core::fmt;

/// An error returned by a syscall.
///
/// `Unknown` is deliberately not a POSIX name: the frozen ABI only lists the
/// numbers below, and a kernel that invents a new one is a contract violation.
/// Keeping the raw value out of the enum makes that visible instead of silently
/// turning it into a plausible-looking error.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum Errno {
    /// Operation not permitted.
    EPERM = 1,
    /// No such file or directory.
    ENOENT = 2,
    /// I/O error.
    EIO = 5,
    /// Bad file descriptor.
    EBADF = 9,
    /// Resource temporarily unavailable (notably `read(0)` with no queued byte).
    EAGAIN = 11,
    /// Cannot allocate memory.
    ENOMEM = 12,
    /// Permission denied.
    EACCES = 13,
    /// Bad address.
    EFAULT = 14,
    /// File exists.
    EEXIST = 17,
    /// Not a directory.
    ENOTDIR = 20,
    /// Is a directory.
    EISDIR = 21,
    /// Invalid argument.
    EINVAL = 22,
    /// No space left on device.
    ENOSPC = 28,
    /// Read-only filesystem.
    EROFS = 30,
    /// Function not implemented.
    ENOSYS = 38,
    /// Directory not empty (`rmdir` on a directory that still has entries).
    ENOTEMPTY = 39,
    /// File name too long.
    ENAMETOOLONG = 36,
    /// Too many levels of symbolic links.
    ELOOP = 40,
    /// Value too large for the data type.
    EOVERFLOW = 75,
    /// Operation not supported.
    ENOTSUP = 95,
    /// A negative result whose magnitude is not in the frozen table.
    ///
    /// This is a kernel/libpagh drift bug, not a normal error path. Callers
    /// should surface it as a fatal diagnostic rather than retry.
    Unknown = 0,
}

impl Errno {
    /// Map a positive errno number (the magnitude of the kernel's `-errno`)
    /// to a variant. Numbers outside the frozen table become
    /// [`Errno::Unknown`].
    pub const fn from_raw(raw: i32) -> Self {
        match raw {
            1 => Self::EPERM,
            2 => Self::ENOENT,
            5 => Self::EIO,
            9 => Self::EBADF,
            11 => Self::EAGAIN,
            12 => Self::ENOMEM,
            13 => Self::EACCES,
            14 => Self::EFAULT,
            17 => Self::EEXIST,
            20 => Self::ENOTDIR,
            21 => Self::EISDIR,
            22 => Self::EINVAL,
            28 => Self::ENOSPC,
            30 => Self::EROFS,
            38 => Self::ENOSYS,
            39 => Self::ENOTEMPTY,
            36 => Self::ENAMETOOLONG,
            40 => Self::ELOOP,
            75 => Self::EOVERFLOW,
            95 => Self::ENOTSUP,
            _ => Self::Unknown,
        }
    }

    /// The positive number used on the ABI wire. `Unknown` is 0, which no
    /// syscall returns as an error.
    pub const fn raw(self) -> i32 {
        self as i32
    }

    /// A stable, human-readable message. These strings are the contract for
    /// CLI diagnostics (`cat: /x: No such file or directory`), so they are
    /// spelled out rather than taken from `core`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::EPERM => "Operation not permitted",
            Self::ENOENT => "No such file or directory",
            Self::EIO => "I/O error",
            Self::EBADF => "Bad file descriptor",
            Self::EAGAIN => "Resource temporarily unavailable",
            Self::ENOMEM => "Cannot allocate memory",
            Self::EACCES => "Permission denied",
            Self::EFAULT => "Bad address",
            Self::EEXIST => "File exists",
            Self::ENOTDIR => "Not a directory",
            Self::EISDIR => "Is a directory",
            Self::EINVAL => "Invalid argument",
            Self::ENOSPC => "No space left on device",
            Self::EROFS => "Read-only filesystem",
            Self::ENOSYS => "Function not implemented",
            Self::ENOTEMPTY => "Directory not empty",
            Self::ENAMETOOLONG => "File name too long",
            Self::ELOOP => "Too many levels of symbolic links",
            Self::EOVERFLOW => "Value too large for the data type",
            Self::ENOTSUP => "Operation not supported",
            Self::Unknown => "Unknown error (outside the pagh ABI)",
        }
    }
}

impl fmt::Display for Errno {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<Errno> for i32 {
    fn from(err: Errno) -> Self {
        err.raw()
    }
}

/// The result type every fallible `libpagh` operation uses.
pub type Result<T> = core::result::Result<T, Errno>;

/// Turn the kernel's raw `a0` into `Result<usize>`.
///
/// A negative value is `-errno`; everything else is a byte count, an fd, a
/// break address or a count of entries, depending on the caller.
pub(crate) fn check(ret: isize) -> Result<usize> {
    if ret < 0 {
        // `ret == isize::MIN` cannot occur: the kernel's errno is a small
        // positive constant, but use wrapping_neg so even that would map to a
        // huge positive rather than panicking in debug builds.
        Err(Errno::from_raw(ret.wrapping_neg() as i32))
    } else {
        Ok(ret as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_documented_numbers() {
        for raw in [
            1, 2, 5, 9, 11, 12, 13, 14, 17, 20, 21, 22, 28, 30, 38, 39, 36, 40, 75, 95,
        ] {
            assert_eq!(Errno::from_raw(raw).raw(), raw);
        }
    }

    #[test]
    fn unknown_is_explicit() {
        assert_eq!(Errno::from_raw(9999), Errno::Unknown);
    }
}