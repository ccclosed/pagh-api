//! Child processes: `fork` + `execve` + `wait4`, and a small std-shaped
//! [`Command`] over them.
//!
//! The ABI (§2 in `docs/abi.md`) has the three calls this needs: `fork`
//! (pagh-native number 1000), `execve` (221) and `wait4` (260). There is no
//! `posix_spawn`, no `execvp` (no PATH search), no shell and no signal API
//! here, so this module is deliberately the "lite" shape:
//!
//! * [`Command`] resolves the program against `PWD` (the kernel takes absolute
//!   paths only), passes the program itself as `argv[0]`, and builds the
//!   environment from the current one plus [`env`](Command::env) overrides
//!   (or from nothing after [`env_clear`](Command::env_clear)).
//! * [`Command::spawn`] forks; the child calls `execve`, and if that fails it
//!   writes `<path>: <error>` to fd 2 and exits with **127** (the shell's
//!   convention for "cannot execute"). The parent gets a [`Child`].
//! * [`Child::wait`]/[`Child::try_wait`] use `wait4`, with `WNOHANG` for the
//!   non-blocking form, and decode the ABI status: a normal exit is
//!   `code << 8`, a terminating signal is the signal number.
//!
//! Dropping a [`Child`] does **not** kill or reap it: an unreaped child is a
//! zombie until this process exits, so call [`Child::wait`] unless the child
//! is meant to outlive the parent. Output plumbing (pipes, redirection) is
//! not implemented yet; `spawn`/`status` are the supported surface.
//!
//! **There is no `Child::kill`.** The ABI's `kill` resolves targets only
//! inside the calling *run* (a forked child is a different run), so
//! signalling a child answers `ESRCH`; exposing a `kill` that never works
//! would be a fake feature. A child can only be waited for, or outlive the
//! parent.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::errno::Result;
use crate::path;
use crate::sys;

/// How a child ended: an exit code, else the signal that terminated it.
///
/// The ABI's `wait4` status is `code << 8` for a normal exit (only the low
/// byte of the code is kept, as on Linux) and the signal number otherwise;
/// [`code`](Self::code) and [`signal`](Self::signal) decode that.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExitStatus {
    raw: i32,
}

impl ExitStatus {
    /// Wrap a raw `wait4` status.
    pub const fn from_raw(raw: i32) -> Self {
        Self { raw }
    }

    /// The raw `wait4` status.
    pub const fn raw(self) -> i32 {
        self.raw
    }

    /// The exit code, when the child exited normally (`0..=255`).
    pub const fn code(self) -> Option<i32> {
        // A signal is a small positive status that is not shifted; an exit
        // code always leaves the low byte zero.
        if self.raw > 0 && self.raw < 256 {
            None
        } else {
            Some(self.raw >> 8)
        }
    }

    /// The terminating signal, when the child was killed by one.
    pub const fn signal(self) -> Option<i32> {
        if self.raw > 0 && self.raw < 256 {
            Some(self.raw)
        } else {
            None
        }
    }

    /// True for a normal exit with code 0.
    pub const fn success(self) -> bool {
        matches!(self.code(), Some(0))
    }
}

impl core::fmt::Display for ExitStatus {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match (self.code(), self.signal()) {
            (Some(code), _) => write!(f, "exit code {code}"),
            (None, Some(signal)) => write!(f, "signal {signal}"),
            (None, None) => write!(f, "raw status {}", self.raw),
        }
    }
}

/// A spawned child process.
#[derive(Debug)]
pub struct Child {
    pid: i32,
    status: Option<ExitStatus>,
}

impl Child {
    /// The child's process id.
    pub fn id(&self) -> i32 {
        self.pid
    }

    /// Wait for the child to exit and return its status.
    ///
    /// A second call returns the already-reaped status. `EINTR` (a signal
    /// arriving while blocked) is retried, like a POSIX `waitpid` loop.
    pub fn wait(&mut self) -> Result<ExitStatus> {
        if let Some(status) = self.status {
            return Ok(status);
        }
        loop {
            match sys::wait4(self.pid, 0) {
                Ok((0, _)) => continue, // interrupted/spurious: ask again
                Ok((_, raw)) => {
                    let status = ExitStatus::from_raw(raw);
                    self.status = Some(status);
                    return Ok(status);
                }
                Err(crate::errno::Errno::EINTR) => continue,
                Err(error) => return Err(error),
            }
        }
    }

    /// Reap the child if it has already exited, without blocking.
    ///
    /// `Ok(None)` means "still running" (`wait4` with `WNOHANG`); a status is
    /// remembered, so a later [`wait`](Self::wait) returns it immediately.
    pub fn try_wait(&mut self) -> Result<Option<ExitStatus>> {
        if let Some(status) = self.status {
            return Ok(Some(status));
        }
        match sys::wait4(self.pid, sys::WNOHANG)? {
            (0, _) => Ok(None),
            (_, raw) => {
                let status = ExitStatus::from_raw(raw);
                self.status = Some(status);
                Ok(Some(status))
            }
        }
    }
}

/// A std-shaped command builder: `Command::new(prog).arg(..).status()`.
///
/// `new`, `arg`, `args`, `env`, `env_clear`, `spawn` and `status` mirror the
/// names in `std::process`, but this is a userland wrapper over `fork` +
/// `execve`, not a platform abstraction.
#[derive(Clone, Debug)]
pub struct Command {
    program: String,
    args: Vec<String>,
    env_clear: bool,
    env: Vec<(String, String)>,
}

impl Command {
    /// A command for `program`. The path is resolved against `PWD` when the
    /// child is spawned and is passed as `argv[0]` (the kernel does not
    /// synthesize a program name).
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            env_clear: false,
            env: Vec::new(),
        }
    }

    /// Append one argument.
    pub fn arg(&mut self, arg: impl Into<String>) -> &mut Self {
        self.args.push(arg.into());
        self
    }

    /// Append several arguments.
    pub fn args<I, S>(&mut self, args: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        for arg in args {
            self.args.push(arg.into());
        }
        self
    }

    /// Set (or replace) one environment variable for the child.
    pub fn env(&mut self, key: impl Into<String>, value: impl Into<String>) -> &mut Self {
        let key = key.into();
        let value = value.into();
        if let Some(slot) = self.env.iter_mut().find(|(k, _)| *k == key) {
            slot.1 = value;
        } else {
            self.env.push((key, value));
        }
        self
    }

    /// Start the child with an empty environment; later [`env`](Self::env)
    /// calls are the only variables it sees.
    pub fn env_clear(&mut self) -> &mut Self {
        self.env_clear = true;
        self.env.clear();
        self
    }

    /// The environment the child will receive: the current one (unless
    /// `env_clear`) with this command's overrides applied.
    fn build_env(&self) -> Vec<String> {
        let mut environment: Vec<String> = Vec::new();
        if !self.env_clear {
            for (key, value) in crate::env::vars() {
                environment.push(format!("{key}={value}"));
            }
        }
        for (key, value) in &self.env {
            let rendered = format!("{key}={value}");
            let prefix_len = key.len() + 1;
            match environment.iter_mut().find(|entry| {
                entry.len() >= prefix_len
                    && entry.as_bytes()[key.len()] == b'='
                    && entry.starts_with(key.as_str())
            }) {
                Some(slot) => *slot = rendered,
                None => environment.push(rendered),
            }
        }
        environment
    }

    /// Fork and exec the program, returning the running child.
    pub fn spawn(&mut self) -> Result<Child> {
        let absolute = path::resolve(&self.program);
        let argv: Vec<String> = core::iter::once(self.program.clone())
            .chain(self.args.iter().cloned())
            .collect();
        let environment = self.build_env();

        let pid = sys::fork()?;
        if pid == 0 {
            // Child: replace the image. Nothing below runs on success.
            let arg_refs: Vec<&str> = argv.iter().map(String::as_str).collect();
            let env_refs: Vec<&str> = environment.iter().map(String::as_str).collect();
            match sys::execve(&absolute, &arg_refs, &env_refs) {
                Ok(()) => sys::exit_group(127), // the wrapper cannot return on success
                Err(error) => {
                    let message = format!("{absolute}: {error}\n");
                    let _ = sys::write(2, message.as_bytes());
                    sys::exit_group(127)
                }
            }
        }
        Ok(Child { pid, status: None })
    }

    /// Spawn the child, wait for it and return its status.
    pub fn status(&mut self) -> Result<ExitStatus> {
        let mut child = self.spawn()?;
        child.wait()
    }
}
