// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicUsize, Ordering};

use align_ext::AlignExt;
use aster_rights::{Full, Rights};

use crate::{
    prelude::*,
    process::constants::{USER_HEAP_BASE, USER_HEAP_SIZE_LIMIT},
    vm::{
        perms::VmPerms,
        vmar::Vmar,
        vmo::{VmoFlags, VmoOptions},
    },
};

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

    pub fn init(&self, root_vmar: &Vmar<Full>) -> Vaddr {
        let perms = VmPerms::READ | VmPerms::WRITE;
        let vmo_options = VmoOptions::<Rights>::new(0).flags(VmoFlags::RESIZABLE);
        let heap_vmo = vmo_options.alloc().unwrap();
        let vmar_map_options = root_vmar
            .new_map(heap_vmo, perms)
            .unwrap()
            .offset(self.base)
            .size(self.limit);
        vmar_map_options.build().unwrap();
        self.current_heap_end.load(Ordering::Relaxed)
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
                if new_heap_end < current_heap_end {
                    // FIXME: should we allow shrink current user heap?
                    return Ok(current_heap_end);
                }
                let new_size = (new_heap_end - self.base).align_up(PAGE_SIZE);
                let heap_mapping = root_vmar.get_vm_mapping(USER_HEAP_BASE)?;
                let heap_vmo = heap_mapping.vmo();
                heap_vmo.resize(new_size)?;
                self.current_heap_end.store(new_heap_end, Ordering::Release);
                Ok(new_heap_end)
            }
        }
    }

    /// Set heap to the default status. i.e., point the heap end to heap base.
    /// This function will we called in execve.
    pub fn set_default(&self, root_vmar: &Vmar<Full>) {
        self.current_heap_end.store(self.base, Ordering::Relaxed);
        self.init(root_vmar);
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
