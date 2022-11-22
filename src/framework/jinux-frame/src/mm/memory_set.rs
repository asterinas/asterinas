use super::{page_table::PageTable, *};
use crate::prelude::*;
use crate::{
    config::PAGE_SIZE,
    mm::address::is_aligned,
    vm::{VmFrame, VmFrameVec},
    *,
};
use alloc::collections::{btree_map::Entry, BTreeMap};
use core::fmt;

pub struct MapArea {
    /// flags
    pub flags: PTFlags,
    /// start virtual address
    pub start_va: VirtAddr,
    /// the size of these area
    pub size: usize,
    /// all the map information
    pub mapper: BTreeMap<VirtAddr, VmFrame>,
}

pub struct MemorySet {
    pub pt: PageTable,
    /// all the map area, sort by the start virtual address
    areas: BTreeMap<VirtAddr, MapArea>,
}

impl MapArea {
    pub fn mapped_size(&self) -> usize {
        self.size
    }

    pub fn clone(&self) -> Self {
        let mut mapper = BTreeMap::new();
        for (&va, old) in &self.mapper {
            let new = PhysFrame::alloc().unwrap();
            new.as_slice()
                .copy_from_slice(old.physical_frame.as_slice());
            mapper.insert(va, unsafe { VmFrame::new(new) });
        }
        Self {
            start_va: self.start_va,
            size: self.size,
            flags: self.flags,
            mapper,
        }
    }

    /// This function will map the vitural address to the given physical frames
    pub fn new(
        start_va: VirtAddr,
        size: usize,
        flags: PTFlags,
        physical_frames: VmFrameVec,
    ) -> Self {
        assert!(
            start_va.is_aligned()
                && is_aligned(size)
                && physical_frames.len() == (size / PAGE_SIZE)
        );

        let mut map_area = Self {
            flags,
            start_va,
            size,
            mapper: BTreeMap::new(),
        };
        let mut current_va = start_va.clone();
        let page_size = size / PAGE_SIZE;
        let mut phy_frame_iter = physical_frames.iter();

        for i in 0..page_size {
            let vm_frame = phy_frame_iter.next().unwrap();
            map_area.map_with_physical_address(current_va, vm_frame.clone());
            current_va += PAGE_SIZE;
        }

        map_area
    }

    pub fn map_with_physical_address(&mut self, va: VirtAddr, pa: VmFrame) -> PhysAddr {
        assert!(va.is_aligned());

        match self.mapper.entry(va) {
            Entry::Occupied(e) => panic!("already mapped a input physical address"),
            Entry::Vacant(e) => e.insert(pa).physical_frame.start_pa(),
        }
    }

    pub fn map(&mut self, va: VirtAddr) -> PhysAddr {
        assert!(va.is_aligned());
        match self.mapper.entry(va) {
            Entry::Occupied(e) => e.get().physical_frame.start_pa(),
            Entry::Vacant(e) => e
                .insert(VmFrame::alloc_zero().unwrap())
                .physical_frame
                .start_pa(),
        }
    }

    pub fn unmap(&mut self, va: VirtAddr) -> Option<VmFrame> {
        self.mapper.remove(&va)
    }

    pub fn write_data(&mut self, addr: usize, data: &[u8]) {
        let mut current_start_address = addr;
        let mut remain = data.len();
        let mut processed = 0;
        for (va, pa) in self.mapper.iter() {
            if current_start_address >= va.0 && current_start_address < va.0 + PAGE_SIZE {
                let offset = current_start_address - va.0;
                let copy_len = (va.0 + PAGE_SIZE - current_start_address).min(remain);
                let src = &data[processed..processed + copy_len];
                let dst =
                    &mut pa.start_pa().kvaddr().get_bytes_array()[offset..(offset + copy_len)];
                dst.copy_from_slice(src);
                processed += copy_len;
                remain -= copy_len;
                if remain == 0 {
                    return;
                }
                current_start_address = va.0 + PAGE_SIZE;
            }
        }
    }

    pub fn read_data(&self, addr: usize, data: &mut [u8]) {
        let mut start = addr;
        let mut remain = data.len();
        let mut processed = 0;
        for (va, pa) in self.mapper.iter() {
            if start >= va.0 && start < va.0 + PAGE_SIZE {
                let offset = start - va.0;
                let copy_len = (va.0 + PAGE_SIZE - start).min(remain);
                let src = &mut data[processed..processed + copy_len];
                let dst = &pa.start_pa().kvaddr().get_bytes_array()[offset..(offset + copy_len)];
                src.copy_from_slice(dst);
                processed += copy_len;
                remain -= copy_len;
                if remain == 0 {
                    return;
                }
                start = va.0 + PAGE_SIZE;
            }
        }
    }
}

// impl Clone for MapArea {
//     fn clone(&self) -> Self {
//         let mut mapper = BTreeMap::new();
//         for (&va, old) in &self.mapper {
//             let new = VmFrame::alloc().unwrap();
//             new.physical_frame
//                 .exclusive_access()
//                 .as_slice()
//                 .copy_from_slice(old.physical_frame.exclusive_access().as_slice());
//             mapper.insert(va, new);
//         }
//         Self {
//             flags: self.flags,
//             mapper,
//         }
//     }
// }

impl MemorySet {
    pub fn map(&mut self, area: MapArea) {
        if area.size > 0 {
            // TODO: check overlap
            if let Entry::Vacant(e) = self.areas.entry(area.start_va) {
                self.pt.map_area(e.insert(area));
            } else {
                panic!(
                    "MemorySet::map: MapArea starts from {:#x?} is existed!",
                    area.start_va
                );
            }
        }
    }

    /// determine whether a virtaddr is in a mapped area
    pub fn is_mapped(&self, vaddr: VirtAddr) -> bool {
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
        Self {
            pt: PageTable::new(),
            areas: BTreeMap::new(),
        }
    }

    pub fn unmap(&mut self, va: VirtAddr) -> Result<()> {
        if let Some(area) = self.areas.remove(&va) {
            self.pt.unmap_area(&area);
            Ok(())
        } else {
            Err(Error::PageFault)
        }
    }

    pub fn clear(&mut self) {
        for area in self.areas.values_mut() {
            self.pt.unmap_area(area);
        }
        self.areas.clear();
    }

    pub fn write_bytes(&mut self, addr: usize, data: &[u8]) -> Result<()> {
        let mut current_addr = addr;
        let mut remain = data.len();
        let start_write = false;
        let mut offset = 0usize;
        for (va, area) in self.areas.iter_mut() {
            if current_addr >= va.0 && current_addr < area.size + va.0 {
                if !area.flags.contains(PTFlags::WRITABLE) {
                    return Err(Error::PageFault);
                }
                let write_len = remain.min(area.size + va.0 - current_addr);
                area.write_data(current_addr, &data[offset..(offset + write_len)]);
                offset += write_len;
                remain -= write_len;
                // remain -= (va.0 + area.size - current_addr).min(remain);
                if remain == 0 {
                    return Ok(());
                }
                current_addr = va.0 + area.size;
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
            if current_addr >= va.0 && current_addr < area.size + va.0 {
                let read_len = remain.min(area.size + va.0 - current_addr);
                area.read_data(current_addr, &mut data[offset..(offset + read_len)]);
                remain -= read_len;
                offset += read_len;
                // remain -= (va.0 + area.size - current_addr).min(remain);
                if remain == 0 {
                    return Ok(());
                }
                current_addr = va.0 + area.size;
            } else if start_read {
                return Err(Error::PageFault);
            }
        }
        Err(Error::PageFault)
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

impl fmt::Debug for MapArea {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("MapArea")
            .field("flags", &self.flags)
            .field("mapped area", &self.mapper)
            .finish()
    }
}

impl fmt::Debug for MemorySet {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("MemorySet")
            .field("areas", &self.areas)
            .field("page_table_root", &self.pt.root_pa)
            .finish()
    }
}

// pub fn load_app(elf_data: &[u8]) -> (usize, MemorySet) {
//   let elf = ElfFile::new(elf_data).expect("invalid ELF file");
//   assert_eq!(elf.header.pt1.class(), header::Class::SixtyFour, "64-bit ELF required");
//   assert_eq!(elf.header.pt2.type_().as_type(), header::Type::Executable, "ELF is not an executable object");
//   assert_eq!(elf.header.pt2.machine().as_machine(), header::Machine::X86_64, "invalid ELF arch");
//   let mut ms = MemorySet::new();
//   for ph in elf.program_iter() {
//     if ph.get_type() != Ok(Type::Load) {
//       continue;
//     }
//     let va = VirtAddr(ph.virtual_addr() as _);
//     let offset = va.page_offset();
//     let area_start = va.align_down();
//     let area_end = VirtAddr((ph.virtual_addr() + ph.mem_size()) as _).align_up();
//     let data = match ph.get_data(&elf).unwrap() {
//       SegmentData::Undefined(data) => data,
//       _ => panic!("failed to get ELF segment data"),
//     };

//     let mut flags = PTFlags::PRESENT | PTFlags::USER;
//     if ph.flags().is_write() {
//       flags |= PTFlags::WRITABLE;
//     }
//     let mut area = MapArea::new(area_start, area_end.0 - area_start.0, flags);
//     area.write_data(offset, data);
//     ms.insert(area);
//   }
//   ms.insert(MapArea::new(VirtAddr(USTACK_TOP - USTACK_SIZE), USTACK_SIZE,
//     PTFlags::PRESENT | PTFlags::WRITABLE | PTFlags::USER));
//   (elf.header.pt2.entry_point() as usize, ms)
// }
