// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicUsize, Ordering};

use align_ext::AlignExt;
use aster_rights::Full;

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
    current_heap_end: AtomicUsize,
}

impl Heap {
    pub const fn new() -> Self {
        Heap {
            base: USER_HEAP_BASE,
            limit: USER_HEAP_SIZE_LIMIT,
            current_heap_end: AtomicUsize::new(USER_HEAP_BASE),
        }
    }

    /// Inits and maps the heap Vmo
    pub(super) fn alloc_and_map_vmo(&self, root_vmar: &Vmar<Full>) -> Result<()> {
        let vmar_map_options = {
            let perms = VmPerms::READ | VmPerms::WRITE;
            root_vmar
                // FIXME: Our current implementation of mapping resize cannot move
                // existing mappings within the new range, which may cause the resize
                // operation to fail. Therefore, if there are already mappings within
                // the heap expansion range, the brk operation will fail.
                .new_map(PAGE_SIZE, perms)
                .unwrap()
                .offset(self.base)
        };
        vmar_map_options.build()?;

        self.set_uninitialized();
        Ok(())
    }

    pub fn brk(&self, new_heap_end: Option<Vaddr>) -> Result<Vaddr> {
        let current = current!();
        let root_vmar = current.root_vmar();
        match new_heap_end {
            None => Ok(self.current_heap_end.load(Ordering::Relaxed)),
            Some(new_heap_end) => {
                if new_heap_end > self.base + self.limit {
                    return_errno_with_message!(Errno::ENOMEM, "heap size limit was met.");
                }
                let current_heap_end = self.current_heap_end.load(Ordering::Acquire);
                if new_heap_end <= current_heap_end {
                    // FIXME: should we allow shrink current user heap?
                    return Ok(current_heap_end);
                }
                let old_size = (current_heap_end - self.base).align_up(PAGE_SIZE);
                let new_size = (new_heap_end - self.base).align_up(PAGE_SIZE);

                root_vmar.resize_mapping(self.base, old_size, new_size)?;
                self.current_heap_end.store(new_heap_end, Ordering::Release);
                Ok(new_heap_end)
            }
        }
    }

    pub(super) fn set_uninitialized(&self) {
        self.current_heap_end
            .store(self.base + PAGE_SIZE, Ordering::Relaxed);
    }
}

impl Clone for Heap {
    fn clone(&self) -> Self {
        let current_heap_end = self.current_heap_end.load(Ordering::Relaxed);
        Self {
            base: self.base,
            limit: self.limit,
            current_heap_end: AtomicUsize::new(current_heap_end),
        }
    }
}

impl Default for Heap {
    fn default() -> Self {
        Self::new()
    }
}
