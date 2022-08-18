//! A Page in virtual address space
use core::ops::Range;

use alloc::vec;
use kxos_frame::vm::{
    Vaddr, VmAllocOptions, VmFrameVec, VmIo, VmMapOptions, VmPerm, VmSpace, PAGE_SIZE,
};

/// A set of **CONTINUOUS** virtual pages in VmSpace
pub struct VmPageRange<'a> {
    start_page: VmPage,
    end_page: VmPage,
    vm_space: Option<&'a VmSpace>,
}

impl<'a> VmPageRange<'a> {
    /// create a set of pages containing virtual address range [a, b)
    pub const fn new_range(vaddr_range: Range<Vaddr>) -> Self {
        let start_page = VmPage::containing_address(vaddr_range.start);
        let end_page = VmPage::containing_address(vaddr_range.end - 1);
        Self {
            start_page,
            end_page,
            vm_space: None,
        }
    }

    /// returns the page containing the specific vaddr
    pub const fn containing_address(vaddr: Vaddr) -> Self {
        let page = VmPage::containing_address(vaddr);
        Self {
            start_page: page,
            end_page: page,
            vm_space: None,
        }
    }

    pub const fn start_address(&self) -> Vaddr {
        self.start_page.start_address()
    }

    /// the address right after the end page
    pub const fn end_address(&self) -> Vaddr {
        self.end_page.start_address() + PAGE_SIZE
    }

    /// allocate a set of physical frames and map self to frames
    pub fn map(&mut self, vm_space: &'a VmSpace, vm_perm: VmPerm) {
        let options = VmAllocOptions::new(self.len());
        let frames = VmFrameVec::allocate(&options).expect("allocate frame error");
        self.map_to(vm_space, frames, vm_perm);
    }

    /// map self to a set of zeroed frames
    pub fn map_zeroed(&mut self, vm_space: &'a VmSpace, vm_perm: VmPerm) {
        let options = VmAllocOptions::new(self.len());
        let frames = VmFrameVec::allocate(&options).expect("allocate frame error");
        let buffer = vec![0u8; self.nbytes()];
        frames.write_bytes(0, &buffer).expect("write zero failed");
        self.map_to(vm_space, frames, vm_perm)
    }

    /// map self to a set of frames
    pub fn map_to(&mut self, vm_space: &'a VmSpace, frames: VmFrameVec, vm_perm: VmPerm) {
        assert_eq!(self.len(), frames.len());
        let mut vm_map_options = VmMapOptions::new();
        vm_map_options.addr(Some(self.start_address()));
        vm_map_options.perm(vm_perm);
        vm_space.map(frames, &vm_map_options).expect("map failed");
        self.vm_space = Some(vm_space)
    }

    pub fn unmap(&mut self) {
        if self.is_mapped() {
            let vm_space = self.vm_space.take().unwrap();
            vm_space
                .unmap(&(self.start_address()..self.end_address()))
                .expect("unmap failed");
        }
    }

    pub fn is_mapped(&self) -> bool {
        if let None = self.vm_space {
            false
        } else {
            true
        }
    }

    /// return the number of virtual pages
    pub const fn len(&self) -> usize {
        self.end_page.vpn - self.start_page.vpn + 1
    }

    pub const fn nbytes(&self) -> usize {
        self.len() * PAGE_SIZE
    }
}

/// A Virtual Page
#[derive(Debug, Clone, Copy)]
pub struct VmPage {
    /// Virtual Page Number
    vpn: usize,
}

impl VmPage {
    const fn containing_address(vaddr: Vaddr) -> Self {
        Self {
            vpn: vaddr / PAGE_SIZE,
        }
    }

    const fn start_address(&self) -> Vaddr {
        self.vpn * PAGE_SIZE
    }
}
