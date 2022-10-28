use core::sync::atomic::{AtomicUsize, Ordering};

use crate::{memory::vm_page::VmPageRange, prelude::*, process::elf::init_stack::INIT_STACK_BASE};
use kxos_frame::vm::{VmPerm, VmSpace};

// The definition of MMapFlags is from occlum
bitflags! {
    pub struct MMapFlags : u32 {
        const MAP_FILE            = 0x0;
        const MAP_SHARED          = 0x1;
        const MAP_PRIVATE         = 0x2;
        const MAP_SHARED_VALIDATE = 0x3;
        const MAP_TYPE            = 0xf;
        const MAP_FIXED           = 0x10;
        const MAP_ANONYMOUS       = 0x20;
        const MAP_GROWSDOWN       = 0x100;
        const MAP_DENYWRITE       = 0x800;
        const MAP_EXECUTABLE      = 0x1000;
        const MAP_LOCKED          = 0x2000;
        const MAP_NORESERVE       = 0x4000;
        const MAP_POPULATE        = 0x8000;
        const MAP_NONBLOCK        = 0x10000;
        const MAP_STACK           = 0x20000;
        const MAP_HUGETLB         = 0x40000;
        const MAP_SYNC            = 0x80000;
        const MAP_FIXED_NOREPLACE = 0x100000;
    }
}

impl TryFrom<u64> for MMapFlags {
    type Error = Error;

    fn try_from(value: u64) -> Result<Self> {
        MMapFlags::from_bits(value as u32)
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "unknown mmap flags"))
    }
}

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
