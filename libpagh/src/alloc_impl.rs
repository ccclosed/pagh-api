//! A bump allocator over `brk`, enough for `Vec`, `String` and `format!`.
//!
//! The kernel owns the program break and only grows it (`brk` returns the new
//! break or `-ENOMEM`). This allocator never shrinks and never reuses memory:
//! [`GlobalAlloc::dealloc`] is a no-op. That is intentional for a one-shot CLI
//! process — it makes the implementation small enough to audit, and it means a
//! `Vec` that grows repeatedly leaves its old buffer behind.
//!
//! Growth is page-granular and chunked: one `brk` call extends the heap by at
//! least 64 KiB, so a loop that pushes bytes does not make one syscall per
//! allocation. The chunk size is a tradeoff (memory kept vs. syscalls), not an
//! ABI constant.
//!
//! Single-threaded: the kernel has no threads/fork and `docs/abi.md` §4 says so.
//! There is therefore no lock; the `Sync` assertion below is the whole
//! synchronisation argument.

use core::alloc::{GlobalAlloc, Layout};
use core::cell::UnsafeCell;

use crate::sys;

/// Page size used for `brk` rounding. The kernel maps 4 KiB pages.
const PAGE: usize = 4096;

/// Minimum heap extension per `brk` call.
const GROW_CHUNK: usize = 64 * 1024;

#[derive(Clone, Copy, Default)]
struct State {
    initialized: bool,
    /// Next address handed out by the bump.
    cursor: usize,
    /// End of the mapped heap (`brk` value).
    end: usize,
}

struct Bump {
    state: UnsafeCell<State>,
}

// SAFETY: the ABI guarantees a single-threaded process (no clone/fork, no
// signals), and every access to `state` happens inside `GlobalAlloc` methods on
// that one thread. No other code can observe or mutate it.
unsafe impl Sync for Bump {}

// SAFETY: `alloc` returns either null or a non-overlapping region that is valid
// for `layout.size()` bytes at `layout.align()` alignment; `dealloc` is a
// deliberate no-op (bump allocator, memory lives until exit). The implementor
// upholds the `GlobalAlloc` contract for the single-threaded process the ABI
// defines.
unsafe impl GlobalAlloc for Bump {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: single-threaded process; no reentrancy because none of the
        // operations below call back into the allocator except `brk`, which
        // takes a plain integer. The `Sync` comment above is the argument that
        // no other thread can alias this state.
        let state = unsafe { &mut *self.state.get() };

        if !state.initialized {
            match sys::brk(0) {
                Ok(current) => {
                    state.cursor = current;
                    state.end = current;
                    state.initialized = true;
                }
                Err(_) => return core::ptr::null_mut(),
            }
        }

        let Some(start) = align_up(state.cursor, layout.align()) else {
            return core::ptr::null_mut();
        };
        let Some(need_end) = start.checked_add(layout.size()) else {
            return core::ptr::null_mut();
        };

        if need_end > state.end {
            // Grow enough for this allocation, rounded to a page, and never by
            // less than one chunk. `brk` is absolute, not a delta.
            let Some(page_end) = align_up(need_end, PAGE) else {
                return core::ptr::null_mut();
            };
            let Some(chunk_end) = state.end.checked_add(GROW_CHUNK) else {
                return core::ptr::null_mut();
            };
            let target = page_end.max(chunk_end);

            // Prefer a whole chunk, but retry with the exact page-rounded end
            // if the larger request fails. The kernel heap window is finite
            // (1 MiB in the current ABI implementation), and refusing a small
            // allocation because the *chunk* would cross the top would waste
            // the last 64 KiB.
            let new_end = match sys::brk(target) {
                Ok(new_end) if new_end >= need_end => new_end,
                _ if target != page_end => match sys::brk(page_end) {
                    Ok(new_end) if new_end >= need_end => new_end,
                    // A kernel that returns a break below the requested end
                    // (or below the old one) did not satisfy the contract;
                    // treat it as ENOMEM rather than hand out unmapped memory.
                    _ => return core::ptr::null_mut(),
                },
                _ => return core::ptr::null_mut(),
            };
            state.end = new_end;
        }

        state.cursor = need_end;
        start as *mut u8
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {
        // Bump allocator: memory is reclaimed when the process exits.
    }
}

/// Round `value` up to the next multiple of `align`.
///
/// `align` must be a power of two (a `Layout` guarantees it). Returns `None`
/// on overflow instead of wrapping into a bogus address.
fn align_up(value: usize, align: usize) -> Option<usize> {
    debug_assert!(align.is_power_of_two());
    value.checked_add(align - 1).map(|v| v & !(align - 1))
}

#[global_allocator]
static GLOBAL: Bump = Bump {
    state: UnsafeCell::new(State {
        initialized: false,
        cursor: 0,
        end: 0,
    }),
};

/// Called by `alloc` when the heap cannot grow: there is no unwinding and no
/// way to recover, so report and exit with code 102.
///
/// Print without allocating (the allocator is the thing that just failed) and
/// use the low-level `write` wrapper directly.
#[alloc_error_handler]
fn alloc_error(layout: Layout) -> ! {
    crate::rt::fatal(
        102,
        core::format_args!(
            "out of memory (brk failed); requested {} bytes at alignment {}",
            layout.size(),
            layout.align()
        ),
    )
}