//! The initial environment and argument vector.
//!
//! Both are re-read from the stack pointer captured by `_start`, so there is no
//! global `Vec<String>` and no allocation until the caller asks for a value.
//! The kernel promises NUL-terminated UTF-8-ish strings; invalid UTF-8 is
//! converted lossily rather than aborting, because a filename that is not
//! UTF-8 must still be printable.

use alloc::string::String;
use core::ffi::CStr;

use crate::rt;

/// Iterator over `argv`, including `argv[0]` (the program path).
#[derive(Clone, Copy)]
pub struct Args {
    argv: *const *const u8,
    argc: i32,
    index: i32,
}

impl Iterator for Args {
    type Item = String;

    fn next(&mut self) -> Option<String> {
        if self.index >= self.argc || self.argv.is_null() {
            return None;
        }
        // SAFETY: the entry contract says argv has `argc` valid pointers.
        let ptr = unsafe { *self.argv.add(self.index as usize) };
        self.index += 1;
        if ptr.is_null() {
            return None;
        }
        // SAFETY: each argv entry is a NUL-terminated string owned by the
        // initial stack, which lives for the whole process.
        Some(unsafe { cstr_to_string(ptr) })
    }
}

/// Iterator over `envp` as `(key, value)` pairs.
#[derive(Clone, Copy)]
pub struct Vars {
    envp: *const *const u8,
    index: usize,
}

impl Iterator for Vars {
    type Item = (String, String);

    fn next(&mut self) -> Option<(String, String)> {
        loop {
            if self.envp.is_null() {
                return None;
            }
            // SAFETY: the environment is a NULL-terminated pointer array in
            // the initial stack; we stop at the first NULL.
            let ptr = unsafe { *self.envp.add(self.index) };
            if ptr.is_null() {
                return None;
            }
            self.index += 1;
            // SAFETY: environment entries are NUL-terminated strings.
            let entry = unsafe { CStr::from_ptr(ptr as *const core::ffi::c_char) };
            let bytes = entry.to_bytes();
            if let Some(eq) = bytes.iter().position(|b| *b == b'=') {
                let key = String::from_utf8_lossy(&bytes[..eq]).into_owned();
                let value = String::from_utf8_lossy(&bytes[eq + 1..]).into_owned();
                return Some((key, value));
            }
            // A malformed entry without `=` is skipped, not fatal.
        }
    }
}

/// The `argv` of this process, `argv[0]` first.
pub fn args() -> Args {
    let sp = rt::initial_sp();
    if sp == 0 {
        return Args {
            argv: core::ptr::null(),
            argc: 0,
            index: 0,
        };
    }
    let (argc, argv, _envp) = rt::parse_stack(sp);
    Args {
        argv,
        argc,
        index: 0,
    }
}

/// The `envp` of this process as key/value pairs.
pub fn vars() -> Vars {
    Vars {
        envp: envp_ptr(),
        index: 0,
    }
}

/// Look up one environment variable (for example `"PWD"`).
///
/// Returns an owned `String`; the scan itself does not allocate unless the key
/// matches.
pub fn var(name: &str) -> Option<String> {
    let envp = envp_ptr();
    if envp.is_null() {
        return None;
    }
    let mut index = 0usize;
    loop {
        // SAFETY: NULL-terminated environment array; see `Vars::next`.
        let ptr = unsafe { *envp.add(index) };
        if ptr.is_null() {
            return None;
        }
        index += 1;
        // SAFETY: environment entries are NUL-terminated strings.
        let entry = unsafe { CStr::from_ptr(ptr as *const core::ffi::c_char) };
        let bytes = entry.to_bytes();
        if bytes.len() > name.len()
            && &bytes[..name.len()] == name.as_bytes()
            && bytes[name.len()] == b'='
        {
            return Some(String::from_utf8_lossy(&bytes[name.len() + 1..]).into_owned());
        }
    }
}

/// The process's working directory as the environment reports it.
///
/// `PWD` is guaranteed by the entry contract. If it is absent or not absolute
/// (a hand-built environment, or a kernel bug), `/` is returned so a program
/// still resolves relative paths instead of building `PWD/...` nonsense. The
/// fallback is documented in the README as a conservative choice.
pub fn current_dir() -> String {
    match var("PWD") {
        Some(pwd) if pwd.starts_with('/') => pwd,
        _ => String::from("/"),
    }
}

/// The basename of `argv[0]` as recorded at startup, or `"pagh"` if unset.
pub fn program_name() -> &'static str {
    rt::program_name()
}

fn envp_ptr() -> *const *const u8 {
    let sp = rt::initial_sp();
    if sp == 0 {
        return core::ptr::null();
    }
    rt::parse_stack(sp).2
}

/// # Safety
///
/// `ptr` must point at a NUL-terminated string in memory that stays mapped for
/// the process lifetime (the initial stack does).
unsafe fn cstr_to_string(ptr: *const u8) -> String {
    // SAFETY: guaranteed by the caller; `CStr::from_ptr` additionally requires
    // a single NUL terminator, which the kernel places.
    let cstr = unsafe { CStr::from_ptr(ptr as *const core::ffi::c_char) };
    cstr.to_string_lossy().into_owned()
}