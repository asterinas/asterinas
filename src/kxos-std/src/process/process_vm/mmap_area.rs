use core::sync::atomic::{AtomicUsize, Ordering};

use crate::{memory::vm_page::VmPageRange, prelude::*, process::elf::init_stack::INIT_STACK_BASE};
use kxos_frame::vm::{VmPerm, VmSpace};

use crate::syscall::mmap::MMapFlags;

#[derive(Debug)]
pub struct MmapArea {
    base_addr: Vaddr,
    current: AtomicUsize,
}

impl MmapArea {
    pub const fn new() -> MmapArea {
        MmapArea {
            base_addr: INIT_STACK_BASE,
            current: AtomicUsize::new(INIT_STACK_BASE),
        }
    }

    pub fn mmap(
        &self,
        len: usize,
        offset: usize,
        vm_perm: VmPerm,
        flags: MMapFlags,
        vm_space: &VmSpace,
    ) -> Vaddr {
        // TODO: how to respect flags?
        if flags.complement().contains(MMapFlags::MAP_ANONYMOUS)
            | flags.complement().contains(MMapFlags::MAP_PRIVATE)
        {
            panic!("Unsupported mmap flags {:?} now", flags);
        }

        if len % PAGE_SIZE != 0 {
            panic!("Mmap only support page-aligned len");
        }
        if offset % PAGE_SIZE != 0 {
            panic!("Mmap only support page-aligned offset");
        }

        let current = self.current.load(Ordering::Relaxed);
        let vm_page_range = VmPageRange::new_range(current..(current + len));
        vm_page_range.map_zeroed(vm_space, vm_perm);
        self.current.store(current + len, Ordering::Relaxed);
        debug!("mmap area start: 0x{:x}, size: {}", current, len);
        current
    }

    /// Set mmap area to the default status. i.e., point current to base.
    pub fn set_default(&self) {
        self.current.store(self.base_addr, Ordering::Relaxed);
    }
}

impl Clone for MmapArea {
    fn clone(&self) -> Self {
        let current = self.current.load(Ordering::Relaxed);
        Self {
            base_addr: self.base_addr.clone(),
            current: AtomicUsize::new(current),
        }
    }
}
