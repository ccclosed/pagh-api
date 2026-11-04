//! Lexical path handling.
//!
//! pagh's kernel takes **absolute paths only** and has no per-process cwd
//! (`docs/abi.md` §1). Relative names must therefore be resolved against the
//! `PWD` environment variable by the program. These helpers do that without
//! touching the filesystem: no symlink resolution, no `stat`, no `..` checks
//! against the real tree. A path that does not exist still normalizes — the
//! syscall decides whether it is real.
//!
//! This is deliberately lexical (like `realpath -m`, or a shell's `cd`
//! bookkeeping), because a userland that called `stat` for every component
//! would be slower and would still race with the (cooperative) kernel.

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

/// Normalize `path` without touching the filesystem:
///
/// * collapse `//` and drop `.` components;
/// * resolve `..` against the previous component; at the root, extra `..` is
///   dropped (there is no way above `/`);
/// * a relative path keeps a leading `..` (there is no parent to pop);
/// * `""` becomes `"."`; an absolute path that collapses becomes `"/"`.
///
/// The result has no trailing slash (except `/` itself).
pub fn normalize(path: &str) -> String {
    let absolute = path.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();

    for component in path.split('/') {
        match component {
            "" | "." => {}
            ".." => match parts.last() {
                Some(&last) if last != ".." => {
                    parts.pop();
                }
                Some(_) => {
                    if !absolute {
                        parts.push("..");
                    }
                }
                None => {
                    if !absolute {
                        parts.push("..");
                    }
                }
            },
            other => parts.push(other),
        }
    }

    if parts.is_empty() {
        return if absolute {
            String::from("/")
        } else {
            String::from(".")
        };
    }

    let mut out = String::with_capacity(path.len() + 1);
    if absolute {
        out.push('/');
    }
    out.push_str(&parts.join("/"));
    out
}

/// Join `path` onto `base` and normalize, unless `path` is already absolute.
///
/// `base` itself is not assumed to be absolute; [`resolve`] is the version that
/// first resolves against `PWD`.
pub fn join(base: &str, path: &str) -> String {
    if path.starts_with('/') {
        return normalize(path);
    }
    if base.is_empty() {
        return normalize(path);
    }
    let mut joined = String::with_capacity(base.len() + path.len() + 2);
    joined.push_str(base);
    if !base.ends_with('/') {
        joined.push('/');
    }
    joined.push_str(path);
    normalize(&joined)
}

/// Resolve `path` against the process's `PWD`, then normalize.
///
/// PWD comes from [`crate::env::current_dir`], which falls back to `/` when the
/// variable is missing or not absolute (the kernel promises it is present, but
/// a program started with a hand-built environment should still work).
pub fn resolve(path: &str) -> String {
    if path.starts_with('/') {
        normalize(path)
    } else {
        join(&crate::env::current_dir(), path)
    }
}

/// The last component of `path`, with trailing slashes ignored.
///
/// `/` and `///` give `/`; an empty path gives `""`. No suffix handling — that
/// is `basename(1)` policy, implemented in the applet.
pub fn basename(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return if path.starts_with('/') { "/" } else { "" };
    }
    match trimmed.rfind('/') {
        Some(index) => &trimmed[index + 1..],
        None => trimmed,
    }
}

/// Everything before the last component of `path`, with trailing slashes
/// trimmed.
///
/// `/a` gives `/`; `a` gives `.`; an empty path gives `.`.
pub fn dirname(path: &str) -> &str {
    let mut end = path.len();
    while end > 1 && path.as_bytes()[end - 1] == b'/' {
        end -= 1;
    }
    let trimmed = &path[..end];
    if trimmed.is_empty() {
        return ".";
    }
    match trimmed.rfind('/') {
        None => ".",
        Some(0) => "/",
        Some(index) => {
            let mut dir_end = index;
            while dir_end > 1 && trimmed.as_bytes()[dir_end - 1] == b'/' {
                dir_end -= 1;
            }
            &trimmed[..dir_end]
        }
    }
}

/// An owned path with lexical `normalize`/`resolve`/`join`.
///
/// A thin convenience over the free functions; it stores the path as the
/// program spelled it until one of those methods is called.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Path {
    inner: String,
}

impl Path {
    /// Wrap a path.
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            inner: path.into(),
        }
    }

    /// The stored spelling.
    pub fn as_str(&self) -> &str {
        &self.inner
    }

    /// Consume the wrapper and return the string.
    pub fn into_string(self) -> String {
        self.inner
    }

    /// True when the path starts with `/`.
    pub fn is_absolute(&self) -> bool {
        self.inner.starts_with('/')
    }

    /// See [`normalize`].
    pub fn normalize(&self) -> Path {
        Path::new(normalize(&self.inner))
    }

    /// See [`resolve`].
    pub fn resolve(&self) -> Path {
        Path::new(resolve(&self.inner))
    }

    /// See [`join`].
    pub fn join(&self, other: &str) -> Path {
        Path::new(join(&self.inner, other))
    }

    /// See [`basename`].
    pub fn basename(&self) -> &str {
        basename(&self.inner)
    }

    /// See [`dirname`].
    pub fn dirname(&self) -> &str {
        dirname(&self.inner)
    }
}

impl fmt::Display for Path {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.inner)
    }
}