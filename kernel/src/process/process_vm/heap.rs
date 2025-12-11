// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicUsize, Ordering};

use align_ext::AlignExt;

use crate::{
    prelude::*,
    vm::{perms::VmPerms, vmar::Vmar},
};

/// The base address of user heap
pub const USER_HEAP_BASE: Vaddr = 0x0000_0000_1000_0000;
/// The max allowed size of user heap
pub const USER_HEAP_SIZE_LIMIT: usize = 16 * 1024 * PAGE_SIZE; // 16 * 4MB

#[derive(Debug)]
pub struct Heap {
    /// The lowest address of the heap
    base: Vaddr,
    /// The heap size limit
    limit: usize,
    /// The current heap highest address
    current_program_break: AtomicUsize,
}

impl Heap {
    pub const fn new() -> Self {
        Heap {
            base: USER_HEAP_BASE,
            limit: USER_HEAP_SIZE_LIMIT,
            current_program_break: AtomicUsize::new(USER_HEAP_BASE),
        }
    }

    /// Initializes and maps the heap virtual memory.
    pub(super) fn alloc_and_map(&self, vmar: &Vmar) -> Result<()> {
        let vmar_map_options = {
            let perms = VmPerms::READ | VmPerms::WRITE;
            vmar.new_map(PAGE_SIZE, perms).unwrap().offset(self.base)
        };
        vmar_map_options.build()?;

        // If we touch another mapped range when we are trying to expand the
        // heap, we fail.
        //
        // So a simple solution is to reserve enough space for the heap by
        // mapping without any permissions and allow it to be overwritten
        // later by `brk`. New mappings from `mmap` that overlaps this range
        // may be moved to another place.
        let vmar_reserve_options = {
            let perms = VmPerms::empty();
            vmar.new_map(USER_HEAP_SIZE_LIMIT - PAGE_SIZE, perms)
                .unwrap()
                .offset(self.base + PAGE_SIZE)
        };
        vmar_reserve_options.build()?;

        self.set_uninitialized();
        Ok(())
    }

    /// Returns the current program break.
    pub fn program_break(&self) -> Vaddr {
        self.current_program_break.load(Ordering::Relaxed)
    }

    /// Sets the program break to `new_heap_end`.
    ///
    /// Returns the new program break on success, or the current program break on failure.
    /// This behavior is consistent with the Linux `brk` syscall.
    pub fn set_program_break(
        &self,
        new_program_break: Vaddr,
        ctx: &Context,
    ) -> core::result::Result<Vaddr, Vaddr> {
        let user_space = ctx.user_space();
        let vmar = user_space.vmar();

        let current_program_break = self.current_program_break.load(Ordering::Acquire);

        // According to the Linux source code, when the `brk` value is more than the
        // rlimit, the `brk` syscall returns the current `brk` value.
        // Reference: <https://elixir.bootlin.com/linux/v6.16.9/source/mm/mmap.c#L152>
        if new_program_break > self.base + self.limit {
            return Err(current_program_break);
        }
        if new_program_break < current_program_break {
            // FIXME: should we allow shrink current user heap?
            return Ok(current_program_break);
        }

        let current_program_break_aligned = current_program_break.align_up(PAGE_SIZE);
        let new_program_break_aligned = new_program_break.align_up(PAGE_SIZE);

        // No need to expand the heap.
        if new_program_break_aligned == current_program_break_aligned {
            self.current_program_break
                .store(new_program_break, Ordering::Release);
            return Ok(new_program_break);
        }

        // Remove the reserved space.
        vmar.remove_mapping(current_program_break_aligned..new_program_break_aligned)
            .map_err(|_| current_program_break)?;

        let old_size = current_program_break_aligned - self.base;
        let new_size = new_program_break_aligned - self.base;
        // Expand the heap.
        vmar.resize_mapping(self.base, old_size, new_size, false)
            .map_err(|_| current_program_break)?;

        self.current_program_break
            .store(new_program_break, Ordering::Release);
        Ok(new_program_break)
    }

    pub(super) fn set_uninitialized(&self) {
        self.current_program_break
            .store(self.base + PAGE_SIZE, Ordering::Relaxed);
    }
}

impl Clone for Heap {
    fn clone(&self) -> Self {
        let current_program_break = self.current_program_break.load(Ordering::Relaxed);
        Self {
            base: self.base,
            limit: self.limit,
            current_program_break: AtomicUsize::new(current_program_break),
        }
    }
}

impl Default for Heap {
    fn default() -> Self {
        Self::new()
    }
}
