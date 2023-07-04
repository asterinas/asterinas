use alloc::{collections::BTreeMap, fmt, vec::Vec};
use limine::{LimineMemmapEntry, LimineMemmapRequest, LimineMemoryMapEntryType};
use log::debug;
use pod::Pod;
use spin::{Mutex, Once};
use x86_64::structures::paging::PhysFrame;

use crate::{
    config::{ENTRY_COUNT, PAGE_SIZE},
    vm::{
        page_table::{table_of, PageTableEntryTrait, PageTableFlagsTrait},
        MemoryRegions, MemoryRegionsType, Paddr,
    },
};

bitflags::bitflags! {
    #[derive(Pod)]
    #[repr(C)]
    /// Possible flags for a page table entry.
    pub struct PageTableFlags: usize {
        /// Specifies whether the mapped frame or page table is loaded in memory.
        const PRESENT =         1 << 0;
        /// Controls whether writes to the mapped frames are allowed.
        const WRITABLE =        1 << 1;
        /// Controls whether accesses from userspace (i.e. ring 3) are permitted.
        const USER =            1 << 2;
        /// If this bit is set, a “write-through” policy is used for the cache, else a “write-back”
        /// policy is used.
        const WRITE_THROUGH =   1 << 3;
        /// Disables caching for the pointed entry is cacheable.
        const NO_CACHE =        1 << 4;
        /// Whether this entry has been used for linear-address translation.
        const ACCESSED =        1 << 5;
        /// Indicates that the mapping is present in all address spaces, so it isn't flushed from
        /// the TLB on an address space switch.
        const GLOBAL =          1 << 8;
        /// Forbid execute codes on the page. The NXE bits in EFER msr must be set.
        const NO_EXECUTE =      1 << 63;
    }
}

#[derive(Clone, Copy, Pod)]
#[repr(C)]
pub struct PageTableEntry(usize);

pub unsafe fn activate_page_table(root_paddr: Paddr, flags: x86_64::registers::control::Cr3Flags) {
    x86_64::registers::control::Cr3::write(
        PhysFrame::from_start_address(x86_64::PhysAddr::new(root_paddr as u64)).unwrap(),
        flags,
    );
}

pub static ALL_MAPPED_PTE: Mutex<BTreeMap<usize, PageTableEntry>> = Mutex::new(BTreeMap::new());

/// Get memory regions, this function should call after the heap was initialized
pub fn get_memory_regions() -> Vec<MemoryRegions> {
    let mut memory_regions = Vec::new();
    let response = MEMMAP_REQUEST
        .get_response()
        .get()
        .expect("Not found memory region information");
    for i in response.memmap() {
        debug!("Found memory region:{:x?}", **i);
        memory_regions.push(MemoryRegions::from(&**i));
    }
    memory_regions
}

pub fn init() {
    let (page_directory_base, _) = x86_64::registers::control::Cr3::read();
    let page_directory_base = page_directory_base.start_address().as_u64() as usize;

    // Safety: page_directory_base is read from Cr3, the address is valid.
    let p4 = unsafe { table_of::<PageTableEntry>(page_directory_base).unwrap() };
    // Cancel mapping in lowest addresses.
    p4[0].clear();
    let mut map_pte = ALL_MAPPED_PTE.lock();
    for i in 0..512 {
        if p4[i].flags().contains(PageTableFlags::PRESENT) {
            map_pte.insert(i, p4[i]);
        }
    }
}

impl PageTableFlagsTrait for PageTableFlags {
    fn new() -> Self {
        Self::empty()
    }

    fn set_present(mut self, present: bool) -> Self {
        self.set(Self::PRESENT, present);
        self
    }

    fn set_writable(mut self, writable: bool) -> Self {
        self.set(Self::WRITABLE, writable);
        self
    }

    fn set_readable(self, readable: bool) -> Self {
        // do nothing
        self
    }

    fn set_accessible_by_user(mut self, accessible: bool) -> Self {
        self.set(Self::USER, accessible);
        self
    }

    fn is_present(&self) -> bool {
        self.contains(Self::PRESENT)
    }

    fn writable(&self) -> bool {
        self.contains(Self::WRITABLE)
    }

    fn readable(&self) -> bool {
        // always true
        true
    }

    fn accessible_by_user(&self) -> bool {
        self.contains(Self::USER)
    }

    fn set_executable(mut self, executable: bool) -> Self {
        self.set(Self::NO_EXECUTE, !executable);
        self
    }

    fn executable(&self) -> bool {
        !self.contains(Self::NO_EXECUTE)
    }

    fn has_accessed(&self) -> bool {
        self.contains(Self::ACCESSED)
    }

    fn union(&self, flags: &Self) -> Self {
        (*self).union(*flags)
    }

    fn remove(&mut self, flags: &Self) {
        self.remove(*flags)
    }

    fn insert(&mut self, flags: &Self) {
        self.insert(*flags)
    }
}

impl PageTableEntry {
    /// 51:12
    const PHYS_ADDR_MASK: usize = 0xF_FFFF_FFFF_F000;
}

impl PageTableEntryTrait for PageTableEntry {
    type F = PageTableFlags;
    fn new(paddr: Paddr, flags: PageTableFlags) -> Self {
        Self((paddr & Self::PHYS_ADDR_MASK) | flags.bits)
    }
    fn paddr(&self) -> Paddr {
        self.0 as usize & Self::PHYS_ADDR_MASK
    }
    fn flags(&self) -> PageTableFlags {
        PageTableFlags::from_bits_truncate(self.0)
    }
    fn is_unused(&self) -> bool {
        self.0 == 0
    }

    fn update(&mut self, paddr: Paddr, flags: Self::F) {
        self.0 = (paddr & Self::PHYS_ADDR_MASK) | flags.bits;
    }

    fn clear(&mut self) {
        self.0 = 0;
    }

    fn page_index(va: crate::vm::Vaddr, level: usize) -> usize {
        debug_assert!(level >= 1 && level <= 5);
        va >> (12 + 9 * (level - 1)) & (ENTRY_COUNT - 1)
    }
}

impl fmt::Debug for PageTableEntry {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut f = f.debug_struct("PageTableEntry");
        f.field("raw", &self.0)
            .field("paddr", &self.paddr())
            .field("flags", &self.flags())
            .finish()
    }
}

static MEMMAP_REQUEST: LimineMemmapRequest = LimineMemmapRequest::new(0);

impl From<&LimineMemmapEntry> for MemoryRegions {
    fn from(value: &LimineMemmapEntry) -> Self {
        Self {
            base: value.base,
            len: value.len,
            typ: MemoryRegionsType::from(value.typ),
        }
    }
}

impl From<LimineMemoryMapEntryType> for MemoryRegionsType {
    fn from(value: LimineMemoryMapEntryType) -> Self {
        match value {
            LimineMemoryMapEntryType::Usable => Self::Usable,
            LimineMemoryMapEntryType::Reserved => Self::Reserved,
            LimineMemoryMapEntryType::AcpiReclaimable => Self::AcpiReclaimable,
            LimineMemoryMapEntryType::AcpiNvs => Self::AcpiNvs,
            LimineMemoryMapEntryType::BadMemory => Self::BadMemory,
            LimineMemoryMapEntryType::BootloaderReclaimable => Self::BootloaderReclaimable,
            LimineMemoryMapEntryType::KernelAndModules => Self::KernelAndModules,
            LimineMemoryMapEntryType::Framebuffer => Self::Framebuffer,
        }
    }
}
