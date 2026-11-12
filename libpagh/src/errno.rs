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
    /// No such process: `kill`/`tgkill` named a pid or thread that does not
    /// exist.
    ESRCH = 3,
    /// Interrupted system call: a blocking wait returned early because a signal
    /// handler is about to run. The caller decides whether to restart it.
    EINTR = 4,
    /// I/O error.
    EIO = 5,
    /// Argument list too long: `execve` was given an over-long argv/envp.
    E2BIG = 7,
    /// Exec format error: `execve` was given an image the kernel cannot load.
    ENOEXEC = 8,
    /// Bad file descriptor.
    EBADF = 9,
    /// No child processes: `wait4` found no child of the caller to reap.
    ECHILD = 10,
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
    /// Broken pipe: a write on a socket whose peer has closed.
    EPIPE = 32,
    /// Directory not empty (`rmdir` on a directory that still has entries).
    ENOTEMPTY = 39,
    /// A socket operation on a non-socket descriptor.
    ENOTSOCK = 88,
    /// Destination address required: `sendto` on a UDP socket needs a peer.
    EDESTADDRREQ = 89,
    /// Message too long: a UDP payload above the ABI's 1024-byte limit.
    EMSGSIZE = 90,
    /// Protocol option not recognized.
    ENOPROTOOPT = 92,
    /// Address family not supported (the ABI has only `AF_INET`).
    EAFNOSUPPORT = 97,
    /// Address already in use: `bind` on a port another socket owns.
    EADDRINUSE = 98,
    /// Cannot assign requested address: `bind` to an address this interface
    /// does not hold.
    EADDRNOTAVAIL = 99,
    /// Network is unreachable (no DHCP lease or no route).
    ENETUNREACH = 101,
    /// Connection reset by peer.
    ECONNRESET = 104,
    /// Socket is already connected.
    EISCONN = 106,
    /// Socket is not connected.
    ENOTCONN = 107,
    /// Connection timed out.
    ETIMEDOUT = 110,
    /// Connection refused (the peer sent a RST during the handshake).
    ECONNREFUSED = 111,
    /// No route to the host (ARP failed).
    EHOSTUNREACH = 113,
    /// File name too long.
    ENAMETOOLONG = 36,
    /// Too many levels of symbolic links.
    ELOOP = 40,
    /// Value too large for the data type.
    EOVERFLOW = 75,
    /// Operation not supported.
    ENOTSUP = 95,
    /// Protocol error: a TLS handshake or record that did not follow TLS 1.3.
    EPROTO = 71,
    /// The server's certificate was rejected: it does not chain to the
    /// committed CA bundle, does not authorize the requested name, is outside
    /// its validity window, or failed `CertificateVerify`.
    EKEYREJECTED = 129,
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
            3 => Self::ESRCH,
            4 => Self::EINTR,
            5 => Self::EIO,
            7 => Self::E2BIG,
            8 => Self::ENOEXEC,
            9 => Self::EBADF,
            10 => Self::ECHILD,
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
            32 => Self::EPIPE,
            88 => Self::ENOTSOCK,
            89 => Self::EDESTADDRREQ,
            90 => Self::EMSGSIZE,
            92 => Self::ENOPROTOOPT,
            97 => Self::EAFNOSUPPORT,
            98 => Self::EADDRINUSE,
            99 => Self::EADDRNOTAVAIL,
            101 => Self::ENETUNREACH,
            104 => Self::ECONNRESET,
            106 => Self::EISCONN,
            107 => Self::ENOTCONN,
            110 => Self::ETIMEDOUT,
            111 => Self::ECONNREFUSED,
            113 => Self::EHOSTUNREACH,
            36 => Self::ENAMETOOLONG,
            40 => Self::ELOOP,
            75 => Self::EOVERFLOW,
            95 => Self::ENOTSUP,
            71 => Self::EPROTO,
            129 => Self::EKEYREJECTED,
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
            Self::ESRCH => "No such process",
            Self::EINTR => "Interrupted system call",
            Self::EIO => "I/O error",
            Self::E2BIG => "Argument list too long",
            Self::ENOEXEC => "Exec format error",
            Self::EBADF => "Bad file descriptor",
            Self::ECHILD => "No child processes",
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
            Self::EPIPE => "Broken pipe",
            Self::ENOTSOCK => "Socket operation on non-socket",
            Self::EDESTADDRREQ => "Destination address required",
            Self::EMSGSIZE => "Message too long",
            Self::ENOPROTOOPT => "Protocol option not available",
            Self::EAFNOSUPPORT => "Address family not supported",
            Self::EADDRINUSE => "Address already in use",
            Self::EADDRNOTAVAIL => "Cannot assign requested address",
            Self::ENETUNREACH => "Network is unreachable",
            Self::ECONNRESET => "Connection reset by peer",
            Self::EISCONN => "Transport endpoint is already connected",
            Self::ENOTCONN => "Transport endpoint is not connected",
            Self::ETIMEDOUT => "Connection timed out",
            Self::ECONNREFUSED => "Connection refused",
            Self::EHOSTUNREACH => "No route to host",
            Self::ENAMETOOLONG => "File name too long",
            Self::ELOOP => "Too many levels of symbolic links",
            Self::EOVERFLOW => "Value too large for the data type",
            Self::ENOTSUP => "Operation not supported",
            Self::EPROTO => "Protocol error",
            Self::EKEYREJECTED => "Certificate rejected by the trust decision",
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

    /// Every number in `docs/abi.md` §3, in the order the document lists it.
    const DOCUMENTED: [(i32, Errno); 42] = [
        (1, Errno::EPERM),
        (2, Errno::ENOENT),
        (3, Errno::ESRCH),
        (4, Errno::EINTR),
        (5, Errno::EIO),
        (7, Errno::E2BIG),
        (8, Errno::ENOEXEC),
        (9, Errno::EBADF),
        (10, Errno::ECHILD),
        (11, Errno::EAGAIN),
        (12, Errno::ENOMEM),
        (13, Errno::EACCES),
        (14, Errno::EFAULT),
        (17, Errno::EEXIST),
        (20, Errno::ENOTDIR),
        (21, Errno::EISDIR),
        (22, Errno::EINVAL),
        (28, Errno::ENOSPC),
        (30, Errno::EROFS),
        (32, Errno::EPIPE),
        (36, Errno::ENAMETOOLONG),
        (38, Errno::ENOSYS),
        (39, Errno::ENOTEMPTY),
        (40, Errno::ELOOP),
        (71, Errno::EPROTO),
        (75, Errno::EOVERFLOW),
        (88, Errno::ENOTSOCK),
        (89, Errno::EDESTADDRREQ),
        (90, Errno::EMSGSIZE),
        (92, Errno::ENOPROTOOPT),
        (95, Errno::ENOTSUP),
        (97, Errno::EAFNOSUPPORT),
        (98, Errno::EADDRINUSE),
        (99, Errno::EADDRNOTAVAIL),
        (101, Errno::ENETUNREACH),
        (104, Errno::ECONNRESET),
        (106, Errno::EISCONN),
        (107, Errno::ENOTCONN),
        (110, Errno::ETIMEDOUT),
        (111, Errno::ECONNREFUSED),
        (113, Errno::EHOSTUNREACH),
        (129, Errno::EKEYREJECTED),
    ];

    #[test]
    fn round_trips_documented_numbers() {
        for (raw, error) in DOCUMENTED {
            assert_eq!(Errno::from_raw(raw), error);
            assert_eq!(error.raw(), raw);
            assert!(!error.as_str().is_empty());
        }
    }

    #[test]
    fn unknown_is_explicit() {
        assert_eq!(Errno::from_raw(9999), Errno::Unknown);
    }
}
