// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicUsize, Ordering};

use align_ext::AlignExt;
use aster_rights::Full;

use crate::{
    prelude::*,
    vm::{perms::VmPerms, vmar::Vmar},
};

/// The max allowed size of user heap
pub const USER_HEAP_SIZE_LIMIT: usize = 16 * 1026 * 1024 * PAGE_SIZE; // 64 GB

#[derive(Debug)]
pub struct Heap {
    /// The base address of the heap
    base: AtomicUsize,
    /// The heap size limit
    limit: usize,
    /// The current heap highest address
    current_heap_end: AtomicUsize,
}

impl Heap {
    pub const fn new() -> Self {
        Heap {
            base: AtomicUsize::new(0),
            limit: USER_HEAP_SIZE_LIMIT,
            current_heap_end: AtomicUsize::new(0),
        }
    }

    /// Initializes and maps the heap virtual memory.
    pub(super) fn init(&self, root_vmar: &Vmar<Full>, program_break: Vaddr) -> Result<()> {
        self.base
            .compare_exchange(0, program_break, Ordering::AcqRel, Ordering::Relaxed)
            .expect("heap is already initialized");
        self.current_heap_end
            .compare_exchange(
                0,
                program_break + PAGE_SIZE,
                Ordering::AcqRel,
                Ordering::Relaxed,
            )
            .expect("heap is already initialized");

        let vmar_map_options = {
            let perms = VmPerms::READ | VmPerms::WRITE;
            root_vmar
                .new_map(PAGE_SIZE, perms)
                .unwrap()
                .offset(program_break)
        };
        vmar_map_options.build()?;

        Ok(())
    }

    pub fn fork(&self) -> Self {
        let base = self.base.load(Ordering::Relaxed);
        let current_heap_end = self.current_heap_end.load(Ordering::Relaxed);
        Self {
            base: AtomicUsize::new(base),
            limit: self.limit,
            current_heap_end: AtomicUsize::new(current_heap_end),
        }
    }

    pub fn clear(&self) {
        self.base.store(0, Ordering::Relaxed);
        self.current_heap_end.store(0, Ordering::Relaxed);
    }

    pub fn brk(&self, root_vmar: &Vmar<Full>, new_heap_end: Option<Vaddr>) -> Result<Vaddr> {
        match new_heap_end {
            None => Ok(self.current_heap_end.load(Ordering::Relaxed)),
            Some(new_heap_end) => {
                let base = self.base.load(Ordering::Relaxed);
                if base == 0 {
                    panic!("heap is not initialized");
                }
                if new_heap_end > base + self.limit {
                    return_errno_with_message!(Errno::ENOMEM, "heap size limit was met.");
                }
                let current_heap_end = self.current_heap_end.load(Ordering::Acquire);

                if new_heap_end <= current_heap_end {
                    // FIXME: should we allow shrink current user heap?
                    return Ok(current_heap_end);
                }

                let old_size = current_heap_end - base;
                let new_size = new_heap_end - base;

                let extra_size_aligned =
                    new_size.align_up(PAGE_SIZE) - old_size.align_up(PAGE_SIZE);

                if extra_size_aligned == 0 {
                    return Ok(current_heap_end);
                }

                // Expand the heap.
                root_vmar
                    .new_map(extra_size_aligned, VmPerms::READ | VmPerms::WRITE)
                    .unwrap()
                    .offset(current_heap_end.align_up(PAGE_SIZE))
                    .build()?;

                self.current_heap_end.store(new_heap_end, Ordering::Release);
                Ok(new_heap_end)
            }
        }
    }
}

impl Default for Heap {
    fn default() -> Self {
        Self::new()
    }
}
