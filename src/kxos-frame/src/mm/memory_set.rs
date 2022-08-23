use super::{page_table::PageTable, *};
use crate::prelude::*;
use crate::{
    config::PAGE_SIZE,
    mm::address::{is_aligned},
    vm::{VmFrame, VmFrameVec},
    *,
};
use alloc::{
    collections::{btree_map::Entry, BTreeMap}};
use core::fmt;
use x86_64::registers::control::Cr3Flags;
// use xmas_elf::{program::{SegmentData, Type}, {header, ElfFile}};

pub const USTACK_SIZE: usize = 4096 * 4;
pub const USTACK_TOP: usize = 0x8000_0000_0000;

pub struct MapArea {
    /// flags
    pub flags: PTFlags,
    /// all the map information
    pub mapper: BTreeMap<VirtAddr, VmFrame>,
}

pub struct MemorySet {
    pub pt: PageTable,
    /// all the map area
    area: Option<MapArea>,
}

impl MapArea {
    pub fn mapped_size(&self) -> usize {
        self.mapper.len()
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
            mapper: BTreeMap::new(),
        };
        let mut current_va = start_va.clone();
        let page_size = size / PAGE_SIZE;
        let mut phy_frame_iter = physical_frames.iter();

        for i in 0..page_size {
            let vm_frame = phy_frame_iter.next().unwrap();
            map_area.map_with_physical_address(current_va, vm_frame.clone());
            current_va+=PAGE_SIZE;
        }

        map_area
    }

    pub fn map_with_physical_address(&mut self, va: VirtAddr, pa: VmFrame) -> PhysAddr {
        assert!(va.is_aligned());

        match self.mapper.entry(va) {
            Entry::Occupied(e) => panic!("already mapped a input physical address"),
            Entry::Vacant(e) => e.insert(pa).physical_frame.exclusive_access().start_pa(),
        }
    }

    pub fn map(&mut self, va: VirtAddr) -> PhysAddr {
        assert!(va.is_aligned());
        match self.mapper.entry(va) {
            Entry::Occupied(e) => e.get().physical_frame.exclusive_access().start_pa(),
            Entry::Vacant(e) => e
                .insert(VmFrame::alloc_zero().unwrap())
                .physical_frame
                .exclusive_access()
                .start_pa(),
        }
    }

    pub fn unmap(&mut self, va: VirtAddr) -> Option<VmFrame> {
        self.mapper.remove(&va)
    }

    pub fn write_data(&mut self, offset: usize, data: &[u8]) {
        let mut start = offset;
        let mut remain = data.len();
        let mut processed = 0;
        for (va, pa) in self.mapper.iter_mut() {
            if start >= PAGE_SIZE {
                start -= PAGE_SIZE;
            } else {
                let copy_len = (PAGE_SIZE - start).min(remain);
                let src = &data[processed..processed + copy_len];
                let dst = &mut pa.start_pa().kvaddr().get_bytes_array()[start..src.len() + start];
                dst.copy_from_slice(src);
                processed += copy_len;
                remain -= copy_len;
                start = 0;
                if remain == 0 {
                    return;
                }
            }
        }
    }

    pub fn read_data(&self, offset: usize, data: &mut [u8]) {
        let mut start = offset;
        let mut remain = data.len();
        let mut processed = 0;
        for (va, pa) in self.mapper.iter() {
            if start >= PAGE_SIZE {
                start -= PAGE_SIZE;
            } else {
                let copy_len = (PAGE_SIZE - start).min(remain);
                let src = &mut data[processed..processed + copy_len];
                let dst = &mut pa.start_pa().kvaddr().get_bytes_array()[start..src.len() + start];
                src.copy_from_slice(dst);
                processed += copy_len;
                remain -= copy_len;
                start = 0;
                if remain == 0 {
                    return;
                }
            }
        }
    }
}

impl Clone for MapArea {
    fn clone(&self) -> Self {
        let mut mapper = BTreeMap::new();
        for (&va, old) in &self.mapper {
            let new = VmFrame::alloc().unwrap();
            new.physical_frame
                .exclusive_access()
                .as_slice()
                .copy_from_slice(old.physical_frame.exclusive_access().as_slice());
            mapper.insert(va, new);
        }
        Self {
            flags: self.flags,
            mapper,
        }
    }
}

impl MemorySet {
    pub fn new(area: MapArea) -> Self {
        let mut pt = PageTable::new();
        pt.map_area(&area);

        Self {
            pt: PageTable::new(),
            area: Some(area),
        }
    }

    pub fn zero() -> Self {
        Self {
            pt: PageTable::new(),
            area: None,
        }
    }

    pub fn unmap(&mut self, va: VirtAddr) -> Result<()> {
        if self.area.is_none() {
            Err(Error::InvalidArgs)
        } else {
            self.area.take().unwrap().unmap(va);
            Ok(())
        }
    }

    pub fn clear(&mut self) {
        self.pt.unmap_area(&self.area.take().unwrap());
        self.area = None;
    }

    pub fn activate(&self) {
        unsafe {
            x86_64::registers::control::Cr3::write(
                x86_64::structures::paging::PhysFrame::from_start_address(x86_64::PhysAddr::new(
                    self.pt.root_pa.0 as u64,
                ))
                .unwrap(),
                Cr3Flags::empty(),
            );
        }
    }

    pub fn write_bytes(&mut self, offset: usize, data: &[u8]) -> Result<()> {
        if self.area.is_none() {
            Err(Error::InvalidArgs)
        } else {
            self.area.take().unwrap().write_data(offset, data);
            Ok(())
        }
    }

    pub fn read_bytes(&self, offset: usize, data: &mut [u8]) -> Result<()> {
        if self.area.is_none() {
            Err(Error::InvalidArgs)
        } else {
            self.area.as_ref().unwrap().read_data(offset, data);
            Ok(())
        }
    }
}

impl Clone for MemorySet {
    fn clone(&self) -> Self {
        if self.area.is_none() {
            Self::zero()
        } else {
            Self::new(self.area.clone().unwrap())
        }
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
            .field("areas", &self.area)
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
