// SPDX-License-Identifier: MPL-2.0

//! User heap management.
//!
//! This is for retrocompatibility with `brk`/`sbrk` system calls.
//!
//! A user heap is a contiguous region of memory between the initial program
//! break and the current program break.
//!
//! All methods of [`ProgramBreak`] are not thread-safe.

use core::sync::atomic::{AtomicUsize, Ordering};

use align_ext::AlignExt;
use aster_rights::Full;

use crate::{
    prelude::*,
    vm::{perms::VmPerms, vmar::Vmar},
};

/// The max allowed size of the process heap.
///
/// TODO: Allow it to be configured by `setrlimit`.
pub const USER_HEAP_SIZE_LIMIT: usize = 0x10_0000_0000; // 64 GiB

/// This is to prevent allocating `mmap` ranges directly after the program
/// break. Otherwise, the `brk` syscall will fail.
const USER_HEAP_VM_CLEARANCE: usize = 0x10_0000_0000; // 64 GiB

/// Uninitialized heap end.
const UNINITIALIZED: usize = usize::MAX;

/// Tracking the end of the user's data segment.
///
/// See <https://man7.org/linux/man-pages/man2/brk.2.html> for more details.
#[derive(Debug)]
pub struct ProgramBreak {
    // Both fields:
    //  - are initialized to `UNINITIALIZED` when the process is created;
    //  - can be not aligned. And if it is not aligned, the actual mapped
    //    address will be aligned to the next page boundary.
    /// The end of the data segment when the process was created.
    init_break: AtomicUsize,
    /// The current end of the data segment.
    current_break: AtomicUsize,
}

impl ProgramBreak {
    /// Creates a new `ProgramBreak` instance that is not initialized.
    ///
    /// The program break should be initialized with [`Self::init`] when doing
    /// `exec`, i.e., loading a program to the process.
    pub const fn new_uninit() -> Self {
        ProgramBreak {
            init_break: AtomicUsize::new(UNINITIALIZED),
            current_break: AtomicUsize::new(UNINITIALIZED),
        }
    }

    /// Initializes with an initial program break inside a `Vmar`.
    ///
    /// It also maps a clearance region to prevent allocating `mmap` ranges
    /// directly after the program break. This is to prevent `brk` syscall
    /// from failing.
    pub(super) fn init_and_map_clearance(
        &self,
        root_vmar: &Vmar<Full>,
        program_break: Vaddr,
    ) -> Result<()> {
        self.init_break
            .compare_exchange(
                UNINITIALIZED,
                program_break,
                Ordering::Relaxed,
                Ordering::Relaxed,
            )
            .expect("heap is already initialized");
        self.current_break
            .compare_exchange(
                UNINITIALIZED,
                program_break,
                Ordering::Relaxed,
                Ordering::Relaxed,
            )
            .expect("heap is already initialized");

        // Map the clearance region.
        let offset = program_break.align_up(PAGE_SIZE);
        root_vmar
            .new_map(USER_HEAP_VM_CLEARANCE, VmPerms::empty())
            .unwrap()
            .offset(offset)
            .build()?;

        Ok(())
    }

    /// Forks a new instance of `ProgramBreak` from the current one.
    ///
    /// The new instance will have the same initial program break and current
    /// program break as the provided one. After that, operations on the new
    /// instance will not affect the original one, and vice versa.
    pub fn fork(&self) -> Self {
        let init_break = self.init_break.load(Ordering::Relaxed);
        let current_break = self.current_break.load(Ordering::Relaxed);

        ProgramBreak {
            init_break: AtomicUsize::new(init_break),
            current_break: AtomicUsize::new(current_break),
        }
    }

    /// Clears the program break into the uninitialized state.
    pub fn clear(&self) {
        self.init_break.store(UNINITIALIZED, Ordering::Relaxed);
        self.current_break.store(UNINITIALIZED, Ordering::Relaxed);
    }

    /// Does the `brk` system call to expand the user heap.
    ///
    /// If the provided `new_break` is `None`, it returns the current program break.
    pub fn brk(&self, root_vmar: &Vmar<Full>, new_break: Option<Vaddr>) -> Result<Vaddr> {
        match new_break {
            None => Ok(self.current_break.load(Ordering::Relaxed)),
            Some(new_break) => {
                let base = self.init_break.load(Ordering::Relaxed);
                if base == UNINITIALIZED {
                    panic!("heap is not initialized");
                }
                if new_break > base + USER_HEAP_SIZE_LIMIT {
                    return_errno_with_message!(Errno::ENOMEM, "heap size limit was met.");
                }
                let current_break = self.current_break.load(Ordering::Acquire);

                if new_break <= current_break {
                    // TODO: Allow shrinking the user heap.
                    return Ok(current_break);
                }

                let current_aligned_break = current_break.align_up(PAGE_SIZE);
                let new_aligned_break = new_break.align_up(PAGE_SIZE);

                let extra_size_aligned = new_aligned_break - current_aligned_break;

                if extra_size_aligned == 0 {
                    return Ok(new_break);
                }

                // Expand the heap.
                root_vmar
                    .new_map(extra_size_aligned, VmPerms::READ | VmPerms::WRITE)
                    .unwrap()
                    .offset(current_aligned_break)
                    .can_overwrite(true)
                    .build()?;

                self.current_break.store(new_break, Ordering::Release);

                Ok(new_break)
            }
        }
    }
}
