// SPDX-License-Identifier: MPL-2.0

//! Kernel memory space management.
//!
//! The kernel memory space is currently managed as follows, if the
//! address width is 48 bits (with 47 bits kernel space).
//!
//! ```text
//! +-+ <- the highest used address (0xffff_ffff_ffff_0000)
//! | |         For the kernel code, 1 GiB.
//! +-+ <- 0xffff_ffff_8000_0000
//! | |
//! | |         Unused hole.
//! +-+ <- 0xffff_e200_0000_0000
//! | |         For frame metadata, 2 TiB. Mapped frames are tracked with handles.
//! +-+ <- 0xffff_e000_0000_0000
//! | |
//! | |         For vm alloc/io mappings, 32 TiB.
//! | |         Mapped frames are tracked with handles.
//! | |
//! +-+ <- the middle of the higher half (0xffff_c000_0000_0000)
//! | |
//! | |
//! | |
//! | |         For linear mappings, 64 TiB.
//! | |         Mapped physical addresses are untracked.
//! | |
//! | |
//! | |
//! +-+ <- the base of high canonical address (0xffff_8000_0000_0000)
//! ```
//!
//! If the address width is (according to [`crate::arch::mm::PagingConsts`])
//! 39 bits or 57 bits, the memory space just adjust porportionally.

use alloc::vec::Vec;
use core::{mem::size_of, ops::Range};

use align_ext::AlignExt;
use spin::Once;

use super::{
    frame::{
        allocator::FRAME_ALLOCATOR,
        meta,
        meta::{FrameMeta, FrameType},
    },
    nr_subpage_per_huge,
    page_prop::{CachePolicy, PageFlags, PageProperty, PrivilegedPageFlags},
    page_size,
    page_table::{boot_pt::BootPageTable, KernelMode, PageTable},
    MemoryRegionType, Paddr, PagingConstsTrait, Vaddr, VmFrame, PAGE_SIZE,
};
use crate::{
    arch::mm::{PageTableEntry, PagingConsts},
    sync::SpinLock,
};

/// The shortest supported address width is 39 bits. And the literal
/// values are written for 48 bits address width. Adjust the values
/// by arithmetic left shift.
const ADDR_WIDTH_SHIFT: isize = PagingConsts::ADDRESS_WIDTH as isize - 48;

/// Start of the kernel address space.
/// This is the _lowest_ address of the x86-64's _high_ canonical addresses.
pub const KERNEL_BASE_VADDR: Vaddr = 0xffff_8000_0000_0000 << ADDR_WIDTH_SHIFT;
/// End of the kernel address space (non inclusive).
pub const KERNEL_END_VADDR: Vaddr = 0xffff_ffff_ffff_0000 << ADDR_WIDTH_SHIFT;

/// The kernel code is linear mapped to this address.
///
/// FIXME: This offset should be randomly chosen by the loader or the
/// boot compatibility layer. But we disabled it because the framework
/// doesn't support relocatable kernel yet.
pub fn kernel_loaded_offset() -> usize {
    KERNEL_CODE_BASE_VADDR
}

const KERNEL_CODE_BASE_VADDR: usize = 0xffff_ffff_8000_0000 << ADDR_WIDTH_SHIFT;

pub(in crate::vm) const FRAME_METADATA_CAP_VADDR: Vaddr = 0xffff_e200_0000_0000 << ADDR_WIDTH_SHIFT;
pub(in crate::vm) const FRAME_METADATA_BASE_VADDR: Vaddr =
    0xffff_e000_0000_0000 << ADDR_WIDTH_SHIFT;

const VMALLOC_BASE_VADDR: Vaddr = 0xffff_c000_0000_0000 << ADDR_WIDTH_SHIFT;

/// The base address of the linear mapping of all physical
/// memory in the kernel address space.
pub const LINEAR_MAPPING_BASE_VADDR: Vaddr = 0xffff_8000_0000_0000 << ADDR_WIDTH_SHIFT;
pub const LINEAR_MAPPING_VADDR_RANGE: Range<Vaddr> = LINEAR_MAPPING_BASE_VADDR..VMALLOC_BASE_VADDR;

/// Convert physical address to virtual address using offset, only available inside aster-frame
pub fn paddr_to_vaddr(pa: Paddr) -> usize {
    debug_assert!(pa < VMALLOC_BASE_VADDR - LINEAR_MAPPING_BASE_VADDR);
    pa + LINEAR_MAPPING_BASE_VADDR
}

/// This is for destructing the boot page table.
static BOOT_PAGE_TABLE: SpinLock<Option<BootPageTable<PageTableEntry, PagingConsts>>> =
    SpinLock::new(None);
pub static KERNEL_PAGE_TABLE: Once<PageTable<KernelMode, PageTableEntry, PagingConsts>> =
    Once::new();

/// Initialize the kernel page table.
///
/// This function should be called after:
///  - the page allocator and the heap allocator are initialized;
///  - the memory regions are initialized.
///
/// This function should be called before:
///  - any initializer that modifies the kernel page table.
pub fn init_kernel_page_table() {
    let regions = crate::boot::memory_regions();
    let phys_mem_cap = {
        let mut end = 0;
        for r in regions {
            end = end.max(r.base() + r.len());
        }
        end.align_up(PAGE_SIZE)
    };

    // The kernel page table should be built afther the metadata pages are initialized.
    let (boot_pt, meta_frames) = init_boot_page_table_and_page_meta(phys_mem_cap);
    // Move it to the global static to prolong it's life.
    // There's identical mapping in it so we can't drop it and activate the kernel page table
    // immediately in this function.
    *BOOT_PAGE_TABLE.lock() = Some(boot_pt);

    // Starting to initialize the kernel page table.
    let kpt = PageTable::<KernelMode>::empty();

    // Make shared the page tables mapped by the root table in the kernel space.
    {
        let pte_index_max = nr_subpage_per_huge::<PagingConsts>();
        kpt.make_shared_tables(pte_index_max / 2..pte_index_max);
    }

    // Do linear mappings for the kernel.
    {
        let from = LINEAR_MAPPING_BASE_VADDR..LINEAR_MAPPING_BASE_VADDR + phys_mem_cap;
        let to = 0..phys_mem_cap;
        let prop = PageProperty {
            flags: PageFlags::RW,
            cache: CachePolicy::Writeback,
            priv_flags: PrivilegedPageFlags::GLOBAL,
        };
        // SAFETY: we are doing the linear mapping for the kernel.
        unsafe {
            kpt.map(&from, &to, prop).unwrap();
        }
    }

    // Map the metadata pages.
    {
        let start_va = meta::mapping::page_to_meta::<PagingConsts>(0, 1);
        let from = start_va..start_va + meta_frames.len() * PAGE_SIZE;
        let prop = PageProperty {
            flags: PageFlags::RW,
            cache: CachePolicy::Writeback,
            priv_flags: PrivilegedPageFlags::GLOBAL,
        };
        let mut cursor = kpt.cursor_mut(&from).unwrap();
        for frame in meta_frames {
            // Safety: we are doing the metadata mappings for the kernel.
            unsafe {
                cursor.map(frame, prop);
            }
        }
    }

    // Map for the I/O area.
    // TODO: we need to have an allocator to allocate kernel space for
    // the I/O areas, rather than doing it using the linear mappings.
    {
        let to = 0x8_0000_0000..0x9_0000_0000;
        let from = LINEAR_MAPPING_BASE_VADDR + to.start..LINEAR_MAPPING_BASE_VADDR + to.end;
        let prop = PageProperty {
            flags: PageFlags::RW,
            cache: CachePolicy::Uncacheable,
            priv_flags: PrivilegedPageFlags::GLOBAL,
        };
        // SAFETY: we are doing I/O mappings for the kernel.
        unsafe {
            kpt.map(&from, &to, prop).unwrap();
        }
    }

    // Map for the kernel code itself.
    // TODO: set separated permissions for each segments in the kernel.
    {
        let region = regions
            .iter()
            .find(|r| r.typ() == MemoryRegionType::Kernel)
            .unwrap();
        let offset = kernel_loaded_offset();
        let to =
            region.base().align_down(PAGE_SIZE)..(region.base() + region.len()).align_up(PAGE_SIZE);
        let from = to.start + offset..to.end + offset;
        let prop = PageProperty {
            flags: PageFlags::RWX,
            cache: CachePolicy::Writeback,
            priv_flags: PrivilegedPageFlags::GLOBAL,
        };
        // SAFETY: we are doing mappings for the kernel.
        unsafe {
            kpt.map(&from, &to, prop).unwrap();
        }
    }

    KERNEL_PAGE_TABLE.call_once(|| kpt);
}

pub fn activate_kernel_page_table() {
    // Safety: the kernel page table is initialized properly.
    unsafe {
        KERNEL_PAGE_TABLE.get().unwrap().activate_unchecked();
        crate::arch::mm::tlb_flush_all_including_global();
    }
    // Drop the boot page table.
    *BOOT_PAGE_TABLE.lock() = None;
}

/// Initialize the boot page table and the page metadata for all physical memories.
/// The boot page table created should be dropped after the kernel page table is initialized.
///
/// It returns the metadata frames for each level of the page table.
fn init_boot_page_table_and_page_meta(
    phys_mem_cap: usize,
) -> (BootPageTable<PageTableEntry, PagingConsts>, Vec<VmFrame>) {
    let mut boot_pt = {
        let cur_pt_paddr = crate::arch::mm::current_page_table_paddr();
        BootPageTable::from_anonymous_boot_pt(cur_pt_paddr)
    };

    let num_pages = phys_mem_cap / page_size::<PagingConsts>(1);
    let num_meta_pages = (num_pages * size_of::<FrameMeta>()).div_ceil(PAGE_SIZE);
    let meta_frames = alloc_meta_frames(num_meta_pages);

    // Map the metadata pages.
    for (i, frame_paddr) in meta_frames.iter().enumerate() {
        let vaddr = meta::mapping::page_to_meta::<PagingConsts>(0, 1) + i * PAGE_SIZE;
        let prop = PageProperty {
            flags: PageFlags::RW,
            cache: CachePolicy::Writeback,
            priv_flags: PrivilegedPageFlags::GLOBAL,
        };
        boot_pt.map_base_page(vaddr, frame_paddr / PAGE_SIZE, prop);
    }

    // Now the metadata pages are mapped, we can initialize the metadata and
    // turn meta frame addresses into `VmFrame`s.
    let meta_frames = meta_frames
        .into_iter()
        .map(|paddr| {
            // Safety: the frame is allocated but not initialized thus not referenced.
            let mut frame = unsafe { VmFrame::from_free_raw(paddr, 1) };
            // Safety: this is the only reference to the frame so it's exclusive.
            unsafe { frame.meta.deref_mut().frame_type = FrameType::Meta };
            frame
        })
        .collect();

    (boot_pt, meta_frames)
}

fn alloc_meta_frames(nframes: usize) -> Vec<Paddr> {
    let mut meta_pages = Vec::new();
    let start_frame = FRAME_ALLOCATOR
        .get()
        .unwrap()
        .lock()
        .alloc(nframes)
        .unwrap()
        * PAGE_SIZE;
    // Zero them out as initialization.
    let vaddr = paddr_to_vaddr(start_frame) as *mut u8;
    unsafe { core::ptr::write_bytes(vaddr, 0, PAGE_SIZE * nframes) };
    for i in 0..nframes {
        let paddr = start_frame + i * PAGE_SIZE;
        meta_pages.push(paddr);
    }
    meta_pages
}
