use super::page_table::{PageTable, PageTableConfig, UserMode};
use crate::prelude::*;
use crate::{
    arch::mm::{PageTableEntry, PageTableFlags},
    config::{PAGE_SIZE, PHYS_OFFSET},
    vm::is_page_aligned,
    vm::{VmAllocOptions, VmFrame, VmFrameVec},
};
use alloc::collections::{btree_map::Entry, BTreeMap};
use core::fmt;

#[derive(Debug)]
pub struct MapArea {
    pub flags: PageTableFlags,
    pub start_va: Vaddr,
    pub size: usize,
    pub mapper: BTreeMap<Vaddr, VmFrame>,
}

pub struct MemorySet {
    pub pt: PageTable<PageTableEntry>,
}

impl Clone for MapArea {
    fn clone(&self) -> Self {
        let mut mapper = BTreeMap::new();
        for (&va, old) in &self.mapper {
            let new = VmAllocOptions::new(1).uninit(true).alloc_single().unwrap();
            new.copy_from_frame(old);
            mapper.insert(va, new.clone());
        }
        Self {
            start_va: self.start_va,
            size: self.size,
            flags: self.flags,
            mapper,
        }
    }
}

impl MapArea {
    pub fn mapped_size(&self) -> usize {
        self.size
    }

    /// This function will map the vitural address to the given physical frames
    pub fn new(
        start_va: Vaddr,
        size: usize,
        flags: PageTableFlags,
        physical_frames: VmFrameVec,
    ) -> Self {
        assert!(
            is_page_aligned(start_va)
                && is_page_aligned(size)
                && physical_frames.len() == (size / PAGE_SIZE)
        );

        let mut map_area = Self {
            flags,
            start_va,
            size,
            mapper: BTreeMap::new(),
        };
        let mut current_va = start_va;
        let page_size = size / PAGE_SIZE;
        let mut phy_frame_iter = physical_frames.iter();

        for i in 0..page_size {
            let vm_frame = phy_frame_iter.next().unwrap();
            map_area.map_with_physical_address(current_va, vm_frame.clone());
            current_va += PAGE_SIZE;
        }

        map_area
    }

    pub fn map_with_physical_address(&mut self, va: Vaddr, pa: VmFrame) -> Paddr {
        assert!(is_page_aligned(va));

        match self.mapper.entry(va) {
            Entry::Occupied(e) => panic!("already mapped a input physical address"),
            Entry::Vacant(e) => e.insert(pa).start_paddr(),
        }
    }

    pub fn map(&mut self, va: Vaddr) -> Paddr {
        assert!(is_page_aligned(va));

        match self.mapper.entry(va) {
            Entry::Occupied(e) => e.get().start_paddr(),
            Entry::Vacant(e) => e
                .insert(VmAllocOptions::new(1).alloc_single().unwrap())
                .start_paddr(),
        }
    }

    pub fn unmap(&mut self, va: Vaddr) -> Option<VmFrame> {
        self.mapper.remove(&va)
    }
}

impl Default for MemorySet {
    fn default() -> Self {
        Self::new()
    }
}

impl MemorySet {
    pub fn map(&mut self, area: MapArea) {
        if area.size > 0 {
            // TODO: check overlap
            for (va, frame) in area.mapper.iter() {
                debug_assert!(frame.start_paddr() < PHYS_OFFSET);
                self.pt.map(*va, frame, area.flags).unwrap();
            }
        }
    }

    pub fn new() -> Self {
        let mut page_table = PageTable::<PageTableEntry, UserMode>::new(PageTableConfig {
            address_width: super::page_table::AddressWidth::Level4,
        });
        let mapped_pte = crate::arch::mm::ALL_MAPPED_PTE.lock();
        for (index, pte) in mapped_pte.iter() {
            // Safety: These PTEs are all valid PTEs fetched from the initial page table during memory initialization.
            unsafe {
                page_table.add_root_mapping(*index, pte);
            }
        }
        Self { pt: page_table }
    }

    pub fn unmap_one_page(&mut self, va: Vaddr) -> Result<()> {
        self.pt.unmap(va).unwrap();
        Ok(())
    }

    pub fn clear(&mut self) {}

    pub fn protect(&mut self, addr: Vaddr, flags: PageTableFlags) {
        let va = addr;
        self.pt.protect(va, flags).unwrap();
    }
}

impl Clone for MemorySet {
    fn clone(&self) -> Self {
        Self::new()
    }
}
impl Drop for MemorySet {
    fn drop(&mut self) {
        self.clear();
    }
}

impl fmt::Debug for MemorySet {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("MemorySet")
            .field("page_table_root", &self.pt.root_paddr())
            .finish()
    }
}
