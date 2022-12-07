use core::sync::atomic::{AtomicUsize, Ordering};

use crate::vm::perms::VmPerms;
use crate::{
    prelude::*,
    rights::Rights,
    vm::vmo::{VmoFlags, VmoOptions},
};
use jinux_frame::AlignExt;

pub const USER_HEAP_BASE: Vaddr = 0x0000_0000_1000_0000;
pub const USER_HEAP_SIZE_LIMIT: usize = PAGE_SIZE * 1000;

#[derive(Debug)]
pub struct UserHeap {
    /// the low address of user heap
    heap_base: Vaddr,
    current_heap_end: AtomicUsize,
}

impl UserHeap {
    pub const fn new() -> Self {
        UserHeap {
            heap_base: USER_HEAP_BASE,
            current_heap_end: AtomicUsize::new(USER_HEAP_BASE),
        }
    }

    pub fn brk(&self, new_heap_end: Option<Vaddr>) -> Result<Vaddr> {
        let current = current!();
        let root_vmar = current.root_vmar().unwrap();
        match new_heap_end {
            None => {
                // create a heap vmo for current process
                let perms = VmPerms::READ | VmPerms::WRITE;
                let vmo_options = VmoOptions::<Rights>::new(0).flags(VmoFlags::RESIZABLE);
                let heap_vmo = vmo_options.alloc().unwrap();
                let vmar_map_options = root_vmar
                    .new_map(heap_vmo, perms)
                    .unwrap()
                    .offset(USER_HEAP_BASE)
                    .size(USER_HEAP_SIZE_LIMIT);
                vmar_map_options.build().unwrap();
                return Ok(self.current_heap_end.load(Ordering::Relaxed));
            }
            Some(new_heap_end) => {
                let current_heap_end = self.current_heap_end.load(Ordering::Acquire);
                if new_heap_end < current_heap_end {
                    // FIXME: should we allow shrink current user heap?
                    return Ok(current_heap_end);
                }
                let new_size = (new_heap_end - self.heap_base).align_up(PAGE_SIZE);
                let heap_vmo = root_vmar.get_mapped_vmo(USER_HEAP_BASE)?;
                heap_vmo.resize(new_size)?;
                self.current_heap_end.store(new_heap_end, Ordering::Release);
                return Ok(new_heap_end);
            }
        }
    }

    /// Set heap to the default status. i.e., point the heap end to heap base.
    pub fn set_default(&self) {
        self.current_heap_end
            .store(self.heap_base, Ordering::Relaxed);
    }
}

impl Clone for UserHeap {
    fn clone(&self) -> Self {
        let current_heap_end = self.current_heap_end.load(Ordering::Relaxed);
        Self {
            heap_base: self.heap_base.clone(),
            current_heap_end: AtomicUsize::new(current_heap_end),
        }
    }
}
