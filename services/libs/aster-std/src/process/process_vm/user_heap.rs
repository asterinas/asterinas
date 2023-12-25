use core::sync::atomic::{AtomicUsize, Ordering};

use crate::vm::perms::VmPerms;
use crate::vm::vmar::Vmar;
use crate::{
    prelude::*,
    vm::vmo::{VmoFlags, VmoOptions},
};
use align_ext::AlignExt;
use aster_rights::{Full, Rights};

pub const USER_HEAP_BASE: Vaddr = 0x0000_0000_1000_0000;
pub const USER_HEAP_SIZE_LIMIT: usize = PAGE_SIZE * 1000;

#[derive(Debug)]
pub struct UserHeap {
    /// the low address of user heap
    heap_base: Vaddr,
    /// the max heap size
    heap_size_limit: usize,
    current_heap_end: AtomicUsize,
}

impl UserHeap {
    pub const fn new() -> Self {
        UserHeap {
            heap_base: USER_HEAP_BASE,
            heap_size_limit: USER_HEAP_SIZE_LIMIT,
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
            .offset(self.heap_base)
            .size(self.heap_size_limit);
        vmar_map_options.build().unwrap();
        self.current_heap_end.load(Ordering::Relaxed)
    }

    pub fn brk(&self, new_heap_end: Option<Vaddr>) -> Result<Vaddr> {
        let current = current!();
        let root_vmar = current.root_vmar();
        match new_heap_end {
            None => Ok(self.current_heap_end.load(Ordering::Relaxed)),
            Some(new_heap_end) => {
                if new_heap_end > self.heap_base + self.heap_size_limit {
                    return_errno_with_message!(Errno::ENOMEM, "heap size limit was met.");
                }
                let current_heap_end = self.current_heap_end.load(Ordering::Acquire);
                if new_heap_end < current_heap_end {
                    // FIXME: should we allow shrink current user heap?
                    return Ok(current_heap_end);
                }
                let new_size = (new_heap_end - self.heap_base).align_up(PAGE_SIZE);
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
        self.current_heap_end
            .store(self.heap_base, Ordering::Relaxed);
        self.init(root_vmar);
    }
}

impl Clone for UserHeap {
    fn clone(&self) -> Self {
        let current_heap_end = self.current_heap_end.load(Ordering::Relaxed);
        Self {
            heap_base: self.heap_base,
            heap_size_limit: self.heap_size_limit,
            current_heap_end: AtomicUsize::new(current_heap_end),
        }
    }
}
