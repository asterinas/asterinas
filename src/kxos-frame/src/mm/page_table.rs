use super::{memory_set::MapArea, *};
use crate::{
    config::{ENTRY_COUNT, KERNEL_OFFSET, PAGE_SIZE, PHYS_OFFSET},
    vm::VmFrame,
    *,
};
use alloc::{collections::BTreeMap, vec, vec::Vec};
use core::fmt;
use lazy_static::lazy_static;

lazy_static! {
    pub static ref ALL_MAPPED_PTE: UPSafeCell<BTreeMap<usize, PageTableEntry>> =
        unsafe { UPSafeCell::new(BTreeMap::new()) };
}

#[derive(Clone, Copy)]
#[repr(transparent)]
pub struct PageTableEntry(usize);

impl PageTableEntry {
    const PHYS_ADDR_MASK: usize = !(PAGE_SIZE - 1);

    pub const fn new_page(pa: PhysAddr, flags: PTFlags) -> Self {
        Self((pa.0 & Self::PHYS_ADDR_MASK) | flags.bits)
    }
    const fn pa(self) -> PhysAddr {
        PhysAddr(self.0 as usize & Self::PHYS_ADDR_MASK)
    }
    const fn flags(self) -> PTFlags {
        PTFlags::from_bits_truncate(self.0)
    }
    const fn is_unused(self) -> bool {
        self.0 == 0
    }
    const fn is_present(self) -> bool {
        (self.0 & PTFlags::PRESENT.bits) != 0
    }
}

impl fmt::Debug for PageTableEntry {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut f = f.debug_struct("PageTableEntry");
        f.field("raw", &self.0)
            .field("pa", &self.pa())
            .field("flags", &self.flags())
            .finish()
    }
}

pub struct PageTable {
    pub root_pa: PhysAddr,
    /// store all the physical frame that the page table need to map all the frame e.g. the frame of the root_pa
    tables: Vec<VmFrame>,
}

impl PageTable {
    pub fn new() -> Self {
        let root_frame = VmFrame::alloc_zero().unwrap();
        let p4 = table_of(root_frame.start_pa());
        let map_pte = ALL_MAPPED_PTE.exclusive_access();
        for (index, pte) in map_pte.iter() {
            p4[*index] = *pte;
        }
        Self {
            root_pa: root_frame.start_pa(),
            tables: vec![root_frame],
        }
    }

    pub fn print_kernel(&self) {
        let p4 = table_of(self.root_pa);
        for i in 0..(256) {
            let phys = PhysAddr(i << (12 + 27));
            let a = p4[p4_index(phys.kvaddr())];
            if a.is_present() {
                println!("index:{:?},PTE:{:?}", i, a);
            }
        }
    }

    pub fn map(&mut self, va: VirtAddr, pa: PhysAddr, flags: PTFlags) {
        let entry = self.get_entry_or_create(va).unwrap();
        if !entry.is_unused() {
            panic!("{:#x?} is mapped before mapping", va);
        }
        *entry = PageTableEntry::new_page(pa.align_down(), flags);
    }

    pub fn unmap(&mut self, va: VirtAddr) {
        let entry = get_entry(self.root_pa, va).unwrap();
        if entry.is_unused() {
            panic!("{:#x?} is invalid before unmapping", va);
        }
        entry.0 = 0;
    }

    pub fn map_area(&mut self, area: &MapArea) {
        for (va, pa) in area.mapper.iter() {
            assert!(pa.start_pa().0 < PHYS_OFFSET);
            self.map(*va, pa.start_pa(), area.flags);
        }
    }

    pub fn unmap_area(&mut self, area: &MapArea) {
        for (va, _) in area.mapper.iter() {
            self.unmap(*va);
        }
    }
}

impl PageTable {
    fn alloc_table(&mut self) -> PhysAddr {
        let frame = VmFrame::alloc_zero().unwrap();
        let pa = frame.start_pa();
        self.tables.push(frame);
        pa
    }

    fn get_entry_or_create(&mut self, va: VirtAddr) -> Option<&mut PageTableEntry> {
        let p4 = table_of(self.root_pa);
        let p4e = &mut p4[p4_index(va)];
        let p3 = next_table_or_create(p4e, || self.alloc_table())?;
        let p3e = &mut p3[p3_index(va)];
        let p2 = next_table_or_create(p3e, || self.alloc_table())?;
        let p2e = &mut p2[p2_index(va)];
        let p1 = next_table_or_create(p2e, || self.alloc_table())?;
        let p1e = &mut p1[p1_index(va)];
        Some(p1e)
    }
}

const fn p4_index(va: VirtAddr) -> usize {
    (va.0 >> (12 + 27)) & (ENTRY_COUNT - 1)
}

const fn p3_index(va: VirtAddr) -> usize {
    (va.0 >> (12 + 18)) & (ENTRY_COUNT - 1)
}

const fn p2_index(va: VirtAddr) -> usize {
    (va.0 >> (12 + 9)) & (ENTRY_COUNT - 1)
}

const fn p1_index(va: VirtAddr) -> usize {
    (va.0 >> 12) & (ENTRY_COUNT - 1)
}

pub fn query(root_pa: PhysAddr, va: VirtAddr) -> Option<(PhysAddr, PTFlags)> {
    let entry = get_entry(root_pa, va)?;
    if entry.is_unused() {
        return None;
    }
    let off = va.page_offset();
    Some((PhysAddr(entry.pa().0 + off), entry.flags()))
}

fn get_entry(root_pa: PhysAddr, va: VirtAddr) -> Option<&'static mut PageTableEntry> {
    let p4 = table_of(root_pa);
    let p4e = &mut p4[p4_index(va)];
    let p3 = next_table(p4e)?;
    let p3e = &mut p3[p3_index(va)];
    let p2 = next_table(p3e)?;
    let p2e = &mut p2[p2_index(va)];
    let p1 = next_table(p2e)?;
    let p1e = &mut p1[p1_index(va)];
    Some(p1e)
}

fn table_of<'a>(pa: PhysAddr) -> &'a mut [PageTableEntry] {
    let ptr = pa.kvaddr().as_ptr() as *mut _;
    unsafe { core::slice::from_raw_parts_mut(ptr, ENTRY_COUNT) }
}

fn next_table<'a>(entry: &PageTableEntry) -> Option<&'a mut [PageTableEntry]> {
    if entry.is_present() {
        Some(table_of(entry.pa()))
    } else {
        None
    }
}

fn next_table_or_create<'a>(
    entry: &mut PageTableEntry,
    mut alloc: impl FnMut() -> PhysAddr,
) -> Option<&'a mut [PageTableEntry]> {
    if entry.is_unused() {
        let pa = alloc();
        *entry = PageTableEntry::new_page(pa, PTFlags::PRESENT | PTFlags::WRITABLE | PTFlags::USER);
        Some(table_of(pa))
    } else {
        next_table(entry)
    }
}

pub(crate) fn init() {
    let cr3 = x86_64_util::get_cr3();

    let p4 = table_of(PhysAddr(cr3));
    // Cancel mapping in lowest addresses.
    p4[0].0 = 0;
    // there is mapping where index is 1,2,3, so user may not use these value
    let mut map_pte = ALL_MAPPED_PTE.exclusive_access();
    for i in 0..512 {
        if !p4[i].flags().is_empty() {
            map_pte.insert(i, p4[i]);
        }
    }
    // Cancel mapping in lowest addresses.
    // p4[0].0 = 0;
}
