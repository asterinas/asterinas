// SPDX-License-Identifier: MPL-2.0

use super::page_table::{PageTable, PageTableConfig, UserMode};
use crate::{
    arch::mm::{PageTableEntry, PageTableFlags},
    config::{PAGE_SIZE, PHYS_OFFSET},
    vm::is_page_aligned,
    vm::{VmAllocOptions, VmFrame, VmFrameVec, VmReader, VmWriter},
};
use crate::{prelude::*, Error};
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
    /// all the map area, sort by the start virtual address
    areas: BTreeMap<Vaddr, MapArea>,
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

    pub fn write_data(&mut self, addr: usize, data: &[u8]) {
        let mut current_start_address = addr;
        let mut buf_reader: VmReader = data.into();
        for (va, pa) in self.mapper.iter() {
            if current_start_address >= *va && current_start_address < va + PAGE_SIZE {
                let offset = current_start_address - va;
                let _ = pa.writer().skip(offset).write(&mut buf_reader);
                if !buf_reader.has_remain() {
                    return;
                }
                current_start_address = va + PAGE_SIZE;
            }
        }
    }

    pub fn read_data(&self, addr: usize, data: &mut [u8]) {
        let mut start = addr;
        let mut buf_writer: VmWriter = data.into();
        for (va, pa) in self.mapper.iter() {
            if start >= *va && start < va + PAGE_SIZE {
                let offset = start - va;
                let _ = pa.reader().skip(offset).read(&mut buf_writer);
                if !buf_writer.has_avail() {
                    return;
                }
                start = va + PAGE_SIZE;
            }
        }
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
            if let Entry::Vacant(e) = self.areas.entry(area.start_va) {
                let area = e.insert(area);
                for (va, frame) in area.mapper.iter() {
                    debug_assert!(frame.start_paddr() < PHYS_OFFSET);
                    self.pt.map(*va, frame, area.flags).unwrap();
                }
            } else {
                panic!(
                    "MemorySet::map: MapArea starts from {:#x?} is existed!",
                    area.start_va
                );
            }
        }
    }

    /// determine whether a Vaddr is in a mapped area
    pub fn is_mapped(&self, vaddr: Vaddr) -> bool {
        for (start_address, map_area) in self.areas.iter() {
            if *start_address > vaddr {
                break;
            }
            if *start_address <= vaddr && vaddr < *start_address + map_area.mapped_size() {
                return true;
            }
        }
        false
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
        Self {
            pt: page_table,
            areas: BTreeMap::new(),
        }
    }

    pub fn unmap(&mut self, va: Vaddr) -> Result<()> {
        if let Some(area) = self.areas.remove(&va) {
            for (va, _) in area.mapper.iter() {
                self.pt.unmap(*va).unwrap();
            }
            Ok(())
        } else {
            Err(Error::PageFault)
        }
    }

    pub fn clear(&mut self) {
        for area in self.areas.values_mut() {
            for (va, _) in area.mapper.iter() {
                self.pt.unmap(*va).unwrap();
            }
        }
        self.areas.clear();
    }

    pub fn write_bytes(&mut self, addr: usize, data: &[u8]) -> Result<()> {
        let mut current_addr = addr;
        let mut remain = data.len();
        let start_write = false;
        let mut offset = 0usize;
        for (va, area) in self.areas.iter_mut() {
            if current_addr >= *va && current_addr < area.size + va {
                if !area.flags.contains(PageTableFlags::WRITABLE) {
                    return Err(Error::PageFault);
                }
                let write_len = remain.min(area.size + va - current_addr);
                area.write_data(current_addr, &data[offset..(offset + write_len)]);
                offset += write_len;
                remain -= write_len;
                // remain -= (va.0 + area.size - current_addr).min(remain);
                if remain == 0 {
                    return Ok(());
                }
                current_addr = va + area.size;
            } else if start_write {
                return Err(Error::PageFault);
            }
        }
        Err(Error::PageFault)
    }

    pub fn read_bytes(&self, addr: usize, data: &mut [u8]) -> Result<()> {
        let mut current_addr = addr;
        let mut remain = data.len();
        let mut offset = 0usize;
        let start_read = false;
        for (va, area) in self.areas.iter() {
            if current_addr >= *va && current_addr < area.size + va {
                let read_len = remain.min(area.size + va - current_addr);
                area.read_data(current_addr, &mut data[offset..(offset + read_len)]);
                remain -= read_len;
                offset += read_len;
                // remain -= (va.0 + area.size - current_addr).min(remain);
                if remain == 0 {
                    return Ok(());
                }
                current_addr = va + area.size;
            } else if start_read {
                return Err(Error::PageFault);
            }
        }
        Err(Error::PageFault)
    }

    pub fn protect(&mut self, addr: Vaddr, flags: PageTableFlags) {
        let va = addr;
        self.pt.protect(va, flags).unwrap();
    }
}

impl Clone for MemorySet {
    fn clone(&self) -> Self {
        let mut ms = Self::new();
        for area in self.areas.values() {
            ms.map(area.clone());
        }
        ms
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
            .field("areas", &self.areas)
            .field("page_table_root", &self.pt.root_paddr())
            .finish()
    }
}
