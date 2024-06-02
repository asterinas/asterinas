// SPDX-License-Identifier: MPL-2.0

//! Metadata management of pages.
//!
//! You can picture a globally shared, static, gigantic arrary of metadata initialized for each page.
//! An entry in the array is called a `MetaSlot`, which contains the metadata of a page. There would
//! be a dedicated small "heap" space in each slot for dynamic metadata. You can store anything as the
//! metadata of a page as long as it's [`Sync`].
//!
//! In the implemetation level, the slots are placed in the metadata pages mapped to a certain virtual
//! address. It is faster, simpler, safer and more versatile compared with an actual static array
//! implementation.

pub mod mapping {
    //! The metadata of each physical page is linear mapped to fixed virtual addresses
    //! in [`FRAME_METADATA_RANGE`].

    use core::mem::size_of;

    use super::MetaSlot;
    use crate::mm::{kspace::FRAME_METADATA_RANGE, Paddr, PagingConstsTrait, Vaddr, PAGE_SIZE};

    /// Convert a physical address of a base page to the virtual address of the metadata slot.
    pub const fn page_to_meta<C: PagingConstsTrait>(paddr: Paddr) -> Vaddr {
        let base = FRAME_METADATA_RANGE.start;
        let offset = paddr / PAGE_SIZE;
        base + offset * size_of::<MetaSlot>()
    }

    /// Convert a virtual address of the metadata slot to the physical address of the page.
    pub const fn meta_to_page<C: PagingConstsTrait>(vaddr: Vaddr) -> Paddr {
        let base = FRAME_METADATA_RANGE.start;
        let offset = (vaddr - base) / size_of::<MetaSlot>();
        offset * PAGE_SIZE
    }
}

use alloc::vec::Vec;
use core::{
    mem::{size_of, ManuallyDrop},
    ops::Range,
    panic,
    sync::atomic::{AtomicU32, AtomicU8, Ordering},
};

use log::info;
use static_assertions::const_assert_eq;

use super::Page;
use crate::{
    arch::mm::{PageTableEntry, PagingConsts},
    mm::{
        paddr_to_vaddr,
        page::allocator::FRAME_ALLOCATOR,
        page_size,
        page_table::{boot_pt::BootPageTable, PageTableEntryTrait},
        CachePolicy, Paddr, PageFlags, PageProperty, PagingConstsTrait, PagingLevel,
        PrivilegedPageFlags, PAGE_SIZE,
    },
};

/// Represents the usage of a page.
#[repr(u8)]
pub enum PageUsage {
    // The zero variant is reserved for the unused type. Only an unused page
    // can be designated for one of the other purposes.
    Unused = 0,
    /// The page is reserved or unusable. The kernel should not touch it.
    Reserved = 1,

    /// The page is used as a frame, i.e., a page of untyped memory.
    Frame = 32,
    /// The page is used as the head frame in a segment.
    SegmentHead = 33,

    /// The page is used by a page table.
    PageTable = 64,
    /// The page stores metadata of other pages.
    Meta = 65,
    /// The page stores the kernel such as kernel code, data, etc.
    Kernel = 66,
}

#[repr(C)]
pub(in crate::mm) struct MetaSlot {
    /// The metadata of the page.
    ///
    /// The implementation may cast a `*const MetaSlot` to a `*const PageMeta`.
    _inner: MetaSlotInner,
    /// To store [`PageUsage`].
    pub(super) usage: AtomicU8,
    pub(super) ref_count: AtomicU32,
}

pub(super) union MetaSlotInner {
    frame: ManuallyDrop<FrameMeta>,
    seg_head: ManuallyDrop<SegmentHeadMeta>,
    // Make sure the the generic parameters don't effect the memory layout.
    pt: ManuallyDrop<PageTablePageMeta<PageTableEntry, PagingConsts>>,
}

// Currently the sizes of the `MetaSlotInner` union variants are no larger
// than 8 bytes and aligned to 8 bytes. So the size of `MetaSlot` is 16 bytes.
//
// Note that the size of `MetaSlot` should be a multiple of 8 bytes to prevent
// cross-page accesses.
const_assert_eq!(size_of::<MetaSlot>(), 16);

/// All page metadata types must implemented this sealed trait,
/// which ensures that each fields of `PageUsage` has one and only
/// one metadata type corresponding to the usage purpose. Any user
/// outside this module won't be able to add more metadata types
/// and break assumptions made by this module.
///
/// If a page type needs specific drop behavior, it should specify
/// when implementing this trait. When we drop the last handle to
/// this page, the `on_drop` method will be called.
pub trait PageMeta: Default + Sync + private::Sealed + Sized {
    const USAGE: PageUsage;

    fn on_drop(page: &mut Page<Self>);
}

mod private {
    pub trait Sealed {}
}

// ======= Start of all the specific metadata structures definitions ==========

use private::Sealed;

#[derive(Debug, Default)]
#[repr(C)]
pub struct FrameMeta {}

impl Sealed for FrameMeta {}

#[derive(Debug, Default)]
#[repr(C)]
pub struct SegmentHeadMeta {
    /// Length of the segment in bytes.
    pub(in crate::mm) seg_len: u64,
}

impl Sealed for SegmentHeadMeta {}

impl From<Page<FrameMeta>> for Page<SegmentHeadMeta> {
    fn from(page: Page<FrameMeta>) -> Self {
        // FIXME: I intended to prevent a page simultaneously managed by a segment handle
        // and a frame handle. However, `Vmo` holds a frame handle while block IO needs a
        // segment handle from the same page.
        // A segment cannot be mapped. So we have to introduce this enforcement soon:
        // assert_eq!(page.count(), 1);
        unsafe {
            let mut head = Page::<SegmentHeadMeta>::from_raw(page.into_raw());
            (*head.ptr)
                .usage
                .store(PageUsage::SegmentHead as u8, Ordering::Relaxed);
            head.meta_mut().seg_len = PAGE_SIZE as u64;
            head
        }
    }
}

#[derive(Debug, Default)]
#[repr(C)]
pub struct PageTablePageMeta<E: PageTableEntryTrait, C: PagingConstsTrait>
where
    [(); C::NR_LEVELS as usize]:,
{
    pub lock: AtomicU8,
    pub level: PagingLevel,
    pub nr_children: u16,
    _phantom: core::marker::PhantomData<(E, C)>,
}

impl<E: PageTableEntryTrait, C: PagingConstsTrait> Sealed for PageTablePageMeta<E, C> where
    [(); C::NR_LEVELS as usize]:
{
}

#[derive(Debug, Default)]
#[repr(C)]
pub struct MetaPageMeta {}

impl Sealed for MetaPageMeta {}
impl PageMeta for MetaPageMeta {
    const USAGE: PageUsage = PageUsage::Meta;
    fn on_drop(page: &mut Page<Self>) {
        panic!("Meta pages are currently not allowed to be dropped");
    }
}

#[derive(Debug, Default)]
#[repr(C)]
pub struct KernelMeta {}

impl Sealed for KernelMeta {}
impl PageMeta for KernelMeta {
    const USAGE: PageUsage = PageUsage::Kernel;
    fn on_drop(page: &mut Page<Self>) {
        panic!("Kernel pages are not allowed to be dropped");
    }
}

// ======== End of all the specific metadata structures definitions ===========

/// Initialize the metadata of all physical pages.
///
/// The function returns a list of `Page`s containing the metadata.
pub(crate) fn init(boot_pt: &mut BootPageTable) -> Vec<Range<Paddr>> {
    let max_paddr = {
        let regions = crate::boot::memory_regions();
        regions.iter().map(|r| r.base() + r.len()).max().unwrap()
    };

    info!(
        "Initializing page metadata for physical memory up to {:x}",
        max_paddr
    );

    super::MAX_PADDR.store(max_paddr, Ordering::Relaxed);

    let num_pages = max_paddr / page_size::<PagingConsts>(1);
    let num_meta_pages = (num_pages * size_of::<MetaSlot>()).div_ceil(PAGE_SIZE);
    let meta_pages = alloc_meta_pages(num_meta_pages);

    // Map the metadata pages.
    for (i, frame_paddr) in meta_pages.iter().enumerate() {
        let vaddr = mapping::page_to_meta::<PagingConsts>(0) + i * PAGE_SIZE;
        let prop = PageProperty {
            flags: PageFlags::RW,
            cache: CachePolicy::Writeback,
            priv_flags: PrivilegedPageFlags::GLOBAL,
        };
        boot_pt.map_base_page(vaddr, frame_paddr / PAGE_SIZE, prop);
    }

    // Now the metadata pages are mapped, we can initialize the metadata.
    meta_pages
        .into_iter()
        .map(|paddr| {
            let pa = Page::<MetaPageMeta>::from_unused(paddr).into_raw();
            pa..pa + PAGE_SIZE
        })
        .collect()
}

fn alloc_meta_pages(nframes: usize) -> Vec<Paddr> {
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
