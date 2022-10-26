use core::sync::atomic::{AtomicUsize, Ordering};

use crate::prelude::*;
use kxos_frame::vm::{VmPerm, VmSpace};

use super::vm_page::{VmPage, VmPageRange};

pub const USER_HEAP_BASE: Vaddr = 0x0000_0000_1000_0000;

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

    pub fn brk(&self, new_heap_end: Option<Vaddr>, vm_space: &VmSpace) -> Vaddr {
        match new_heap_end {
            None => return self.current_heap_end.load(Ordering::Relaxed),
            Some(new_heap_end) => {
                let current_heap_end = self.current_heap_end.load(Ordering::Acquire);
                if new_heap_end < current_heap_end {
                    return current_heap_end;
                }
                self.current_heap_end.store(new_heap_end, Ordering::Release);
                let start_page = VmPage::containing_address(current_heap_end - 1).next_page();
                let end_page = VmPage::containing_address(new_heap_end);
                if end_page >= start_page {
                    let vm_pages = VmPageRange::new_page_range(start_page, end_page);
                    let vm_perm = UserHeap::user_heap_perm();
                    vm_pages.map_zeroed(vm_space, vm_perm);
                    debug!(
                        "map address: 0x{:x} - 0x{:x}",
                        vm_pages.start_address(),
                        vm_pages.end_address()
                    );
                }
                return new_heap_end;
            }
        }
    }

    #[inline(always)]
    const fn user_heap_perm() -> VmPerm {
        VmPerm::RWXU
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
