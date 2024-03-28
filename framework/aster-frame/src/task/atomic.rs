// SPDX-License-Identifier: MPL-2.0

//! # Atomic Execution Mode
//!
//! When a CPU core runs in the **atomic mode**,
//! the kernel code must be executed _atomically_.
//! Specifically, a kernel panic would be triggered
//! if such forbidden behaviors are captured from the kernel code in the atomic mode:
//!
//! - Any operation that can incur scheduling / context switching:
//!     - Sleeping;
//!     - Waiting for I/O;
//!     - Yielding: yield the processor to other tasks voluntarily;
//!     - Preemption: get preempted by other tasks;
//! - And switching to the user space.
//!
//! Note that the kernel code is allowed to handle interrupts within the atomic mode.
//!
//! The concept of the atomic mode is connected with those of process and interrupt contexts.
//! A broad classification of execution contexts in a classic monolithic kernel can be:
//!
//! | Context Type in Kernel | Process                                 | Interrupt                 |
//! | ---------------------- | --------------------------------------- | ------------------------- |
//! | Execution Target       | kernel code on behalf of a user process | interrupt handler         |
//! | Atomic Requirement     | may switch into or out on demand        | always in the atomic mode |
//!

use core::{
    panic,
    sync::atomic::{AtomicUsize, Ordering::Relaxed},
};
extern crate alloc;
use alloc::{format, string::String};

// pub use atomic_mode_proc_macro::{atomic_procedure, might_break};

crate::cpu_local! {
    static ATOMIC_MODE: AtomicMode = AtomicMode::new(PreemptInfo::new());
}

/// When `PreemptInfo` is non-zero, the CPU cannot call `schedule()`.
#[derive(Debug, Default)]
struct PreemptInfo {
    /// The number of locks and IRQ-disabled contexts held by the current CPU.
    num_locks: AtomicUsize,
}

impl PreemptInfo {
    const fn new() -> Self {
        Self {
            num_locks: AtomicUsize::new(0),
        }
    }
    fn inc_num(&self) {
        self.num_locks.fetch_add(1, Relaxed);
    }
    fn dec_num(&self) {
        self.num_locks.fetch_sub(1, Relaxed);
    }
    fn num(&self) -> usize {
        self.num_locks.load(Relaxed)
    }
}

pub struct AtomicMode {
    delegate: PreemptInfo,
}

impl AtomicMode {
    const fn new(delegate: PreemptInfo) -> Self {
        Self { delegate }
    }

    #[must_use]
    pub fn new_guard(&self) -> AtomicModeGuard {
        self.delegate.inc_num();
        AtomicModeGuard {
            atomic_info: self,
            _thread_unsafe: core::marker::PhantomData,
        }
    }

    pub fn in_atomic_mode(&self) -> bool {
        self.delegate.num() != 0
    }

    fn on_guard_drop(&self) {
        self.delegate.dec_num();
    }

    pub fn info(&self) -> String {
        format!("PREEMPT_COUNT = {}", self.delegate.num())
    }
}

/// The guard of the atomic mode.
/// The CPU shall exit the atomic mode only after the last guard object is dropped.
pub struct AtomicModeGuard<'a> {
    atomic_info: &'a AtomicMode,
    _thread_unsafe: core::marker::PhantomData<*const ()>,
}

impl<'a> Drop for AtomicModeGuard<'a> {
    fn drop(&mut self) {
        self.atomic_info.on_guard_drop();
    }
}

// FIXME: remove this usage by `MannuallyDrop`
/// Transfer out the onwership of the given guard.
pub fn transferred<'a>(_orgin: &mut AtomicModeGuard<'a>) -> AtomicModeGuard<'a> {
    enter_atomic_mode()
}

/// Enter the atomic mode, and return a guard for exiting the current atomic mode.
/// Nested calls of this method are permitted.
/// The CPU shall exit the atomic mode only after the last guard object is dropped.
///
/// # Example
///
/// ```rust
/// // enters the atomic mode
/// let guard = enter_atomic_mode();
/// {
///     let nested_guard = enter_atomic_mode();
///     assert!(is_in_atomic_mode());
///     // do something in atomic mode
/// }
/// assert!(is_in_atomic_mode());
/// drop(guard);    // drop to exit the current atomic mode
/// assert!(!is_in_atomic_mode());
/// ```
#[must_use]
pub fn enter_atomic_mode<'a>() -> AtomicModeGuard<'a> {
    ATOMIC_MODE.new_guard()
}

/// Whether the current context is in atomic mode.
pub fn is_in_atomic_mode() -> bool {
    ATOMIC_MODE.in_atomic_mode()
}

/// Mark a point after which the code in scope may break the rules of the atomic mode.
/// Nothing will happen if the current context is not in atomic mode.
///
/// # Panic
///
/// If the current context is in atomic mode,
/// a panic will be triggered to indicate the violation of the atomic mode rules.
pub fn might_break_atomic_mode() {
    if is_in_atomic_mode() {
        panic!("Break atomic mode: {}", ATOMIC_MODE.info());
    }
}

#[cfg(ktest)]
mod test {
    use super::*;
    mod preempt_info {
        use super::*;
        #[ktest]
        fn inc_num() {
            let info = PreemptInfo::new();
            info.inc_num();
            assert_eq!(info.num(), 1);
        }
        #[ktest]
        fn dec_num() {
            let info = PreemptInfo::new();
            info.inc_num();
            info.inc_num();
            info.dec_num();
            assert_eq!(info.num(), 1);
        }
    }
    mod atomic_mode_env {
        use super::*;
        #[ktest]
        fn is_in() {
            let guard = enter_atomic_mode();
            assert!(is_in_atomic_mode());
        }
        #[ktest]
        fn is_not_in() {
            assert!(!is_in_atomic_mode());
        }

        #[ktest]
        fn entry_multiple_times() {
            assert!(!is_in_atomic_mode());
            let guard1 = enter_atomic_mode();
            let guard2 = enter_atomic_mode();
            assert!(is_in_atomic_mode());
            drop(guard1);
            assert!(is_in_atomic_mode());
            drop(guard2);
            assert!(!is_in_atomic_mode());
        }
    }
    mod break_atomic {
        use super::*;
        #[ktest]
        #[should_panic]
        fn panic_in_atomic_mode() {
            let _guard = enter_atomic_mode();
            might_break_atomic_mode();
        }
        #[ktest]
        fn no_panic_out_of_atomic_mode() {
            assert!(
                !is_in_atomic_mode(),
                "Test environment should not be in atomic mode"
            );
            might_break_atomic_mode();
        }
    }
    // mod macros {
    //     use super::*;
    //     #[might_break]
    //     fn breakable_func() {
    //         // do something might break atomic mode
    //     }

    //     #[atomic_procedure]
    //     fn breakable_func_in_atomic_context() {
    //         breakable_func();
    //     }

    //     #[ktest]
    //     #[should_panic]
    //     fn panics_at_func_in_atomic_mode() {
    //         breakable_func_in_atomic_context();
    //     }
    // }
}
