//! Native threads over `clone`/`futex` (ABI 0.13.0).
//!
//! A thread shares this run's address space, heap, mmap regions and file
//! descriptors; it gets a stack allocated with `mmap`. The kernel has no
//! `join`, so the handle joins through a futex word and the child's exit code.
//!
//! ```ignore
//! let handle = libpagh::thread::Builder::new().spawn(|| 42).unwrap();
//! assert_eq!(handle.join().unwrap(), 42);
//! ```
//!
//! The child's stack is released by the kernel when the run exits; `join`
//! deliberately does not `munmap` it, because the exiting child is still
//! executing its final syscall when the parent observes `done` (there is no
//! waitpid). One mapping per thread is the price of that race-free rule.

use alloc::boxed::Box;
use alloc::sync::Arc;
use core::sync::atomic::{AtomicI32, AtomicU32, Ordering};

use crate::abi::Timespec;
use crate::errno::Result;
use crate::sys;

/// Default user stack for a spawned thread (64 KiB).
pub const DEFAULT_STACK_SIZE: usize = 64 * 1024;

/// Per-thread state shared by the handle and the child.
struct JoinState {
    /// 0 while running, 1 after the function returned (futex word).
    done: AtomicU32,
    /// The function's return code.
    code: AtomicI32,
}

/// The child's entry argument: the closure plus the join state.
struct Start {
    f: Box<dyn FnOnce() -> i32 + Send + 'static>,
    state: Arc<JoinState>,
}

/// Runs in the new thread. It must not return: the kernel starts the child at
/// this address with `ra = 0`.
extern "C" fn trampoline(arg: usize) -> ! {
    // SAFETY: `arg` is the `Box<Start>` raw pointer handed to `clone` by
    // `Builder::spawn`, and this thread is the only owner of it.
    let start = unsafe { Box::from_raw(arg as *mut Start) };
    let Start { f, state } = *start;
    let code = f();
    state.code.store(code, Ordering::SeqCst);
    state.done.store(1, Ordering::SeqCst);
    let word = &state.done as *const AtomicU32 as usize;
    let _ = sys::futex(
        word,
        sys::FUTEX_WAKE | sys::FUTEX_PRIVATE_FLAG,
        u32::MAX,
        0,
    );
    sys::exit(code)
}

/// A spawned thread.
pub struct JoinHandle {
    state: Arc<JoinState>,
    tid: i32,
}

impl JoinHandle {
    /// The kernel thread id.
    pub fn id(&self) -> i32 {
        self.tid
    }

    /// Wait for the thread to finish and return its exit code.
    ///
    /// The futex word is a `u32`; the wait loop handles spurious returns (the
    /// ABI's `futex` is a single wait, not a condition variable).
    pub fn join(self) -> Result<i32> {
        let word = &self.state.done as *const AtomicU32 as usize;
        loop {
            if self.state.done.load(Ordering::SeqCst) != 0 {
                return Ok(self.state.code.load(Ordering::SeqCst));
            }
            let _ = sys::futex(
                word,
                sys::FUTEX_WAIT | sys::FUTEX_PRIVATE_FLAG,
                0,
                0,
            );
        }
    }
}

/// Builder for [`spawn`](Builder::spawn).
pub struct Builder {
    stack_size: usize,
}

impl Builder {
    /// A builder with [`DEFAULT_STACK_SIZE`].
    pub const fn new() -> Builder {
        Builder {
            stack_size: DEFAULT_STACK_SIZE,
        }
    }

    /// Set the child's user stack size (rounded up to a page).
    pub const fn stack_size(mut self, bytes: usize) -> Builder {
        self.stack_size = bytes;
        self
    }

    /// Spawn a thread running `f`; the value it returns becomes the join code.
    ///
    /// The closure must be `Send` and live for the whole run (`'static`); in a
    /// no-allocator-free program that usually means an owned closure or a
    /// function pointer.
    pub fn spawn<F>(self, f: F) -> Result<JoinHandle>
    where
        F: FnOnce() -> i32 + Send + 'static,
    {
        let bytes = self.stack_size.max(4096).div_ceil(4096) * 4096;
        let base = sys::mmap(
            0,
            bytes,
            sys::PROT_READ | sys::PROT_WRITE,
            sys::MAP_PRIVATE | sys::MAP_ANONYMOUS,
        )?;
        let state = Arc::new(JoinState {
            done: AtomicU32::new(0),
            code: AtomicI32::new(0),
        });
        let start = Box::new(Start {
            f: Box::new(f),
            state: Arc::clone(&state),
        });
        let arg = Box::into_raw(start) as usize;
        let stack_top = base + bytes;
        let flags = sys::CLONE_VM
            | sys::CLONE_FS
            | sys::CLONE_FILES
            | sys::CLONE_SIGHAND
            | sys::CLONE_THREAD;
        match sys::clone(flags, stack_top, trampoline as usize, arg, 0, 0) {
            Ok(tid) => Ok(JoinHandle { state, tid }),
            Err(error) => {
                // SAFETY: the clone failed, so no child owns `arg`.
                drop(unsafe { Box::from_raw(arg as *mut Start) });
                let _ = sys::munmap(base, bytes);
                Err(error)
            }
        }
    }
}

impl Default for Builder {
    fn default() -> Self {
        Builder::new()
    }
}

/// The calling thread's kernel id.
pub fn current_id() -> i32 {
    sys::gettid().unwrap_or(-1)
}

/// Voluntarily yield the CPU.
pub fn yield_now() {
    let _ = sys::sched_yield();
}

/// Terminate the calling thread.
pub fn exit(code: i32) -> ! {
    sys::exit(code)
}

/// Terminate every thread of the run.
pub fn exit_group(code: i32) -> ! {
    sys::exit_group(code)
}

/// Sleep while `word == expected` (a futex wait). Returns `EAGAIN` if the value
/// changed first, `ETIMEDOUT` on a timeout, 0 after a wake.
pub fn futex_wait(
    word: &AtomicU32,
    expected: u32,
    timeout: Option<core::time::Duration>,
) -> Result<()> {
    let deadline = match timeout {
        Some(duration) => {
            let ts = Timespec {
                seconds: duration.as_secs() as i64,
                nanos: duration.subsec_nanos() as i64,
            };
            &ts as *const Timespec as usize
        }
        None => 0,
    };
    sys::futex(
        word as *const AtomicU32 as usize,
        sys::FUTEX_WAIT | sys::FUTEX_PRIVATE_FLAG,
        expected,
        deadline,
    )
    .map(|_| ())
}

/// Wake at most `count` waiters on `word`.
pub fn futex_wake(word: &AtomicU32, count: u32) -> Result<usize> {
    sys::futex(
        word as *const AtomicU32 as usize,
        sys::FUTEX_WAKE | sys::FUTEX_PRIVATE_FLAG,
        count,
        0,
    )
    .map(|woken| woken as usize)
}
