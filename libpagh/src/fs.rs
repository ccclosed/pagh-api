//! Files, directories and metadata over the syscalls in `docs/abi.md`.
//!
//! There is no `statat`, so [`stat`] is `openat(O_RDONLY)` + `fstat` + `close`.
//! That is the only sequence the frozen ABI allows; its consequence is that
//! `stat` needs read permission on the file (a mode-`000` file cannot be
//! stat'ed by a program that cannot open it). This is listed as an ABI gap in
//! the README.
//!
//! Every path-taking function resolves relative names against `PWD` first,
//! because the kernel accepts absolute paths only ([`crate::path::resolve`]).

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::abi::{dirent_size, Stat, DIRENT_HEADER};
use crate::errno::{Errno, Result};
use crate::io::{Read, Seek, SeekFrom, Write};
use crate::path;
use crate::sys;

/// An owned file descriptor.
///
/// Dropping the value calls `close`; a close error is ignored (there is no
/// recovery from it, and the kernel has no deferred close semantics).
#[derive(Debug)]
pub struct File {
    fd: i32,
}

impl File {
    /// Open an existing file read-only.
    pub fn open(path: &str) -> Result<File> {
        Self::open_with(path, sys::O_RDONLY, 0)
    }

    /// Create (or truncate) a file for writing with mode `0o666` (subject to
    /// the kernel umask).
    pub fn create(path: &str) -> Result<File> {
        Self::open_with(
            path,
            sys::O_WRONLY | sys::O_CREAT | sys::O_TRUNC,
            0o666,
        )
    }

    /// Open with explicit `openat` flags and mode.
    ///
    /// `path` is resolved against `PWD`; `mode` is used only with
    /// [`sys::O_CREAT`].
    pub fn open_with(path: &str, flags: u32, mode: u32) -> Result<File> {
        let absolute = path::resolve(path);
        let fd = sys::openat(sys::AT_FDCWD, &absolute, flags, mode)?;
        Ok(File { fd })
    }

    /// The raw descriptor, for syscalls that take an fd directly (`getdents64`,
    /// `fstat`, the `EAGAIN`-aware read loop).
    pub fn as_raw_fd(&self) -> i32 {
        self.fd
    }

    /// Give up ownership of the descriptor; the caller must close it.
    pub fn into_raw_fd(mut self) -> i32 {
        let fd = self.fd;
        self.fd = -1;
        fd
    }

    /// Take ownership of an existing descriptor.
    ///
    /// # Safety
    ///
    /// `fd` must be open and owned by this process, and must not be closed or
    /// wrapped by another `File` while this value lives.
    pub unsafe fn from_raw_fd(fd: i32) -> File {
        // SAFETY: constructing `File` performs no unsafe operation; the
        // caller's guarantee (a live, owned descriptor) is documented above and
        // is what makes the eventual `Drop`/I/O sound.
        File { fd }
    }
}

impl Drop for File {
    fn drop(&mut self) {
        if self.fd >= 0 {
            // An ignored close error is the only sane option in `Drop`; the
            // data path has already reported any write error.
            let _ = sys::close(self.fd);
        }
    }
}

impl Read for File {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        sys::read(self.fd, buf)
    }
}

impl Write for File {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        sys::write(self.fd, buf)
    }
}

impl Seek for File {
    fn seek(&mut self, pos: SeekFrom) -> Result<u64> {
        let (offset, whence) = match pos {
            SeekFrom::Start(value) => {
                let offset = i64::try_from(value).map_err(|_| Errno::EINVAL)?;
                (offset, sys::SEEK_SET)
            }
            SeekFrom::Current(delta) => (delta, sys::SEEK_CUR),
            SeekFrom::End(delta) => (delta, sys::SEEK_END),
        };
        sys::lseek(self.fd, offset, whence)
    }
}

/// Read an entire file.
pub fn read_to_end(path: &str) -> Result<Vec<u8>> {
    let mut file = File::open(path)?;
    crate::io::Read::read_to_end(&mut file)
}

/// Read an entire file as UTF-8.
///
/// Invalid UTF-8 is `EINVAL`: the ABI has no `EILSEQ`, and silently replacing
/// bytes would make `cat`-like tools lie about file contents. Callers that
/// display arbitrary bytes use [`read_to_end`] instead.
pub fn read_to_string(path: &str) -> Result<String> {
    let bytes = read_to_end(path)?;
    String::from_utf8(bytes).map_err(|_| Errno::EINVAL)
}

/// Create/truncate `path` and write all of `contents`.
pub fn write(path: &str, contents: &[u8]) -> Result<()> {
    let mut file = File::create(path)?;
    file.write_all(contents)
}

/// Alias for [`write`], matching the name used by the rest of the API
/// (`io::Write::write_all`).
pub fn write_all(path: &str, contents: &[u8]) -> Result<()> {
    write(path, contents)
}

/// Metadata for `path` (`openat` + `fstat` + `close`).
///
/// Returns [`Errno::EPERM`] if the kernel does not allow opening the target
/// (for example a directory) read-only; see the module docs.
pub fn stat(path: &str) -> Result<Stat> {
    let file = File::open(path)?;
    let mut stat = Stat::default();
    sys::fstat(file.as_raw_fd(), &mut stat)?;
    Ok(stat)
}

/// The target of a symbolic link.
///
/// The ABI does not NUL-terminate the result, and a full buffer may mean
/// truncation. This starts at 256 bytes and doubles until the result fits,
/// returning [`Errno::ENAMETOOLONG`] beyond 4096 bytes (a conservative cap; the
/// ABI does not define `PATH_MAX`).
pub fn read_link(path: &str) -> Result<String> {
    let absolute = path::resolve(path);
    let mut capacity = 256usize;
    loop {
        let mut buf = vec![0u8; capacity];
        let n = sys::readlinkat(sys::AT_FDCWD, &absolute, &mut buf)?;
        if n > buf.len() {
            return Err(Errno::EIO);
        }
        if n < capacity {
            buf.truncate(n);
            return Ok(String::from_utf8_lossy(&buf).into_owned());
        }
        if capacity >= 4096 {
            return Err(Errno::ENAMETOOLONG);
        }
        capacity *= 2;
    }
}

/// A directory entry returned by [`Dir`].
///
/// The name is decoded lossily: the ABI does not promise UTF-8 names, and a
/// listing should still show a non-UTF-8 byte as U+FFFD rather than abort.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirEntry {
    ino: u64,
    kind: EntryKind,
    name: String,
}

impl DirEntry {
    /// The entry's inode number.
    pub fn ino(&self) -> u64 {
        self.ino
    }

    /// The entry's kind.
    pub fn kind(&self) -> EntryKind {
        self.kind
    }

    /// The entry's name (no path, no NUL).
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// An open directory being read with `getdents64`.
///
/// The iterator yields `.` and `..` if the kernel does; filtering is a policy
/// decision for the applet (`ls`), not for the filesystem layer.
pub struct Dir {
    fd: i32,
    buf: Vec<u8>,
    start: usize,
    end: usize,
    done: bool,
}

impl Dir {
    /// Open a directory read-only.
    ///
    /// The ABI does not spell out that `openat` accepts a directory; the
    /// kernel must allow `O_RDONLY` opens of directories for `getdents64` to
    /// be usable at all (listed in the README as an assumption).
    pub fn open(path: &str) -> Result<Dir> {
        let file = File::open(path)?;
        let fd = file.into_raw_fd();
        Ok(Dir {
            fd,
            buf: vec![0u8; 4096],
            start: 0,
            end: 0,
            done: false,
        })
    }

    /// Parse one record from the current buffer.
    fn parse_one(&mut self) -> Option<Result<DirEntry>> {
        if self.done {
            return None;
        }
        if self.start >= self.end {
            match sys::getdents64(self.fd, &mut self.buf) {
                Ok(0) => {
                    self.done = true;
                    return None;
                }
                Ok(n) if n <= self.buf.len() => {
                    self.start = 0;
                    self.end = n;
                }
                Ok(_) => {
                    self.done = true;
                    return Some(Err(Errno::EIO));
                }
                Err(err) => {
                    self.done = true;
                    return Some(Err(err));
                }
            }
        }

        let start = self.start;
        let end = self.end;
        let record = &self.buf[start..end];
        if record.len() < DIRENT_HEADER {
            self.done = true;
            return Some(Err(Errno::EIO));
        }

        let name_len = record[9] as usize;
        let Some(size) = dirent_size(name_len) else {
            self.done = true;
            return Some(Err(Errno::EIO));
        };
        // A zero-length name cannot be advanced past; a record longer than the
        // buffer is a kernel bug. Both stop the iterator with EIO instead of
        // looping forever or reading out of bounds.
        if name_len == 0 || size > record.len() {
            self.done = true;
            return Some(Err(Errno::EIO));
        }

        let mut ino_bytes = [0u8; 8];
        ino_bytes.copy_from_slice(&record[..8]);
        let ino = u64::from_le_bytes(ino_bytes);
        let kind = EntryKind::from_raw(record[8] as u32);
        let name = String::from_utf8_lossy(&record[DIRENT_HEADER..DIRENT_HEADER + name_len])
            .into_owned();

        self.start = start + size;
        Some(Ok(DirEntry { ino, kind, name }))
    }
}

impl Iterator for Dir {
    type Item = Result<DirEntry>;

    fn next(&mut self) -> Option<Self::Item> {
        self.parse_one()
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        if self.fd >= 0 {
            let _ = sys::close(self.fd);
        }
    }
}

// Re-exported so `fs::EntryKind` is available next to `DirEntry` without
// forcing callers to import `abi`.
pub use crate::abi::EntryKind;