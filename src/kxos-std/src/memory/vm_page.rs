//! A Page in virtual address space
use crate::prelude::*;
use core::ops::Range;
use kxos_frame::{
    vm::{VmAllocOptions, VmFrameVec, VmIo, VmMapOptions, VmPerm, VmSpace},
    Error,
};

/// A set of **CONTINUOUS** virtual pages in VmSpace
pub struct VmPageRange {
    start_page: VmPage,
    end_page: VmPage,
}

impl VmPageRange {
    /// create a set of pages containing virtual address range [a, b)
    pub const fn new_range(vaddr_range: Range<Vaddr>) -> Self {
        let start_page = VmPage::containing_address(vaddr_range.start);
        let end_page = VmPage::containing_address(vaddr_range.end - 1);
        Self {
            start_page,
            end_page,
        }
    }

    pub const fn new_page_range(start_page: VmPage, end_page: VmPage) -> Self {
        Self {
            start_page,
            end_page,
        }
    }

    /// returns the page containing the specific vaddr
    pub const fn containing_address(vaddr: Vaddr) -> Self {
        let page = VmPage::containing_address(vaddr);
        Self {
            start_page: page,
            end_page: page,
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
    pub fn map(&mut self, vm_space: &VmSpace, vm_perm: VmPerm) {
        let options = VmAllocOptions::new(self.len());
        let frames = VmFrameVec::allocate(&options).expect("allocate frame error");
        self.map_to(vm_space, frames, vm_perm);
    }

    /// map self to a set of zeroed frames
    pub fn map_zeroed(&self, vm_space: &VmSpace, vm_perm: VmPerm) {
        let options = VmAllocOptions::new(self.len());
        let frames = VmFrameVec::allocate(&options).expect("allocate frame error");
        let buffer = vec![0u8; self.nbytes()];
        self.map_to(vm_space, frames, vm_perm);
        vm_space
            .write_bytes(self.start_address(), &buffer)
            .expect("write zero failed");
        // frames.write_bytes(0, &buffer).expect("write zero failed");
    }

    /// map self to a set of frames
    pub fn map_to(&self, vm_space: &VmSpace, frames: VmFrameVec, vm_perm: VmPerm) {
        assert_eq!(self.len(), frames.len());
        let mut vm_map_options = VmMapOptions::new();
        vm_map_options.addr(Some(self.start_address()));
        vm_map_options.perm(vm_perm);
        vm_space.map(frames, &vm_map_options).expect("map failed");
    }

    pub fn unmap(&mut self, vm_space: &VmSpace) {
        vm_space
            .unmap(&(self.start_address()..self.end_address()))
            .expect("unmap failed");
    }

    pub fn is_mapped(&self, vm_space: &VmSpace) -> bool {
        todo!()
    }

    pub fn iter(&self) -> VmPageIter<'_> {
        VmPageIter {
            current: self.start_page,
            page_range: self,
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

pub struct VmPageIter<'a> {
    current: VmPage,
    page_range: &'a VmPageRange,
}

impl<'a> Iterator for VmPageIter<'a> {
    type Item = VmPage;

    fn next(&mut self) -> Option<Self::Item> {
        let next_page = if self.current <= self.page_range.end_page {
            Some(self.current)
        } else {
            None
        };
        self.current = self.current.next_page();
        next_page
    }
}

/// A Virtual Page
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct VmPage {
    /// Virtual Page Number
    vpn: usize,
}

impl VmPage {
    pub const fn containing_address(vaddr: Vaddr) -> Self {
        Self {
            vpn: vaddr / PAGE_SIZE,
        }
    }

    pub const fn start_address(&self) -> Vaddr {
        self.vpn * PAGE_SIZE
    }

    pub const fn next_page(&self) -> VmPage {
        VmPage { vpn: self.vpn + 1 }
    }

    /// Check whether current page is mapped
    pub fn is_mapped(&self, vm_space: &VmSpace) -> bool {
        vm_space.is_mapped(self.start_address())
    }

    pub fn map_page(&self, vm_space: &VmSpace, vm_perm: VmPerm) -> Result<(), Error> {
        let vm_alloc_option = VmAllocOptions::new(1);
        let vm_frame = VmFrameVec::allocate(&vm_alloc_option)?;

        let mut vm_map_options = VmMapOptions::new();
        vm_map_options.addr(Some(self.start_address()));
        vm_map_options.perm(vm_perm);
        vm_space.map(vm_frame, &vm_map_options)?;

        Ok(())
    }
}
