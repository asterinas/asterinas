// SPDX-License-Identifier: MPL-2.0

//! Metadata management of pages.
//!
//! You can picture a globally shared, static, gigantic array of metadata initialized for each page.
//! An entry in the array is called a `MetaSlot`, which contains the metadata of a page. There would
//! be a dedicated small "heap" space in each slot for dynamic metadata. You can store anything as the
//! metadata of a page as long as it's [`Sync`].
//!
//! In the implementation level, the slots are placed in the metadata pages mapped to a certain virtual
//! address. It is faster, simpler, safer and more versatile compared with an actual static array
//! implementation.

pub mod mapping {
    //! The metadata of each physical page is linear mapped to fixed virtual addresses
    //! in [`FRAME_METADATA_RANGE`].

    use core::mem::size_of;

    use super::MetaSlot;
    use crate::mm::{kspace::FRAME_METADATA_RANGE, Paddr, PagingConstsTrait, Vaddr, PAGE_SIZE};

    /// Converts a physical address of a base page to the virtual address of the metadata slot.
    pub const fn page_to_meta<C: PagingConstsTrait>(paddr: Paddr) -> Vaddr {
        let base = FRAME_METADATA_RANGE.start;
        let offset = paddr / PAGE_SIZE;
        base + offset * size_of::<MetaSlot>()
    }

    /// Converts a virtual address of the metadata slot to the physical address of the page.
    pub const fn meta_to_page<C: PagingConstsTrait>(vaddr: Vaddr) -> Paddr {
        let base = FRAME_METADATA_RANGE.start;
        let offset = (vaddr - base) / size_of::<MetaSlot>();
        offset * PAGE_SIZE
    }
}

use alloc::vec::Vec;
use core::{
    cell::UnsafeCell,
    marker::PhantomData,
    mem::{size_of, ManuallyDrop},
    panic,
    sync::atomic::{AtomicU32, AtomicU8, Ordering},
};

use log::info;
use num_derive::FromPrimitive;
use static_assertions::const_assert_eq;

use super::{allocator, Page};
use crate::{
    arch::mm::{PageTableEntry, PagingConsts},
    mm::{
        paddr_to_vaddr, page_size,
        page_table::{boot_pt, PageTableEntryTrait},
        CachePolicy, Paddr, PageFlags, PageProperty, PagingConstsTrait, PagingLevel,
        PrivilegedPageFlags, Vaddr, PAGE_SIZE,
    },
};

/// Represents the usage of a page.
#[repr(u8)]
#[derive(Debug, FromPrimitive, PartialEq)]
pub enum PageUsage {
    // The zero variant is reserved for the unused type. Only an unused page
    // can be designated for one of the other purposes.
    #[allow(dead_code)]
    Unused = 0,
    /// The page is reserved or unusable. The kernel should not touch it.
    #[allow(dead_code)]
    Reserved = 1,

    /// The page is used as a frame, i.e., a page of untyped memory.
    Frame = 32,

    /// The page is used by a page table.
    PageTable = 64,
    /// The page stores metadata of other pages.
    Meta = 65,
    /// The page stores the kernel such as kernel code, data, etc.
    Kernel = 66,

    /// The page stores data for kernel stack.
    KernelStack = 67,
}

#[repr(C)]
pub(in crate::mm) struct MetaSlot {
    /// The metadata of the page.
    ///
    /// It is placed at the beginning of a slot because:
    ///  - the implementation can simply cast a `*const MetaSlot`
    ///    to a `*const PageMeta` for manipulation;
    ///  - the subsequent fields can utilize the end padding of the
    ///    the inner union to save space.
    _inner: MetaSlotInner,
    /// To store [`PageUsage`].
    pub(super) usage: AtomicU8,
    pub(super) ref_count: AtomicU32,
}

pub(super) union MetaSlotInner {
    _frame: ManuallyDrop<FrameMeta>,
    _pt: ManuallyDrop<PageTablePageMeta>,
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
pub trait PageMeta: Sync + private::Sealed + Sized {
    const USAGE: PageUsage;

    fn on_drop(page: &mut Page<Self>);
}

/// An internal routine in dropping implementations.
///
/// # Safety
///
/// The caller should ensure that the pointer points to a page's metadata slot. The
/// page should have a last handle to the page, and the page is about to be dropped,
/// as the metadata slot after this operation becomes uninitialized.
pub(super) unsafe fn drop_as_last<M: PageMeta>(ptr: *const MetaSlot) {
    // This would be guaranteed as a safety requirement.
    debug_assert_eq!((*ptr).ref_count.load(Ordering::Relaxed), 0);
    // Let the custom dropper handle the drop.
    let mut page = Page::<M> {
        ptr,
        _marker: PhantomData,
    };
    M::on_drop(&mut page);
    let _ = ManuallyDrop::new(page);
    // Drop the metadata.
    core::ptr::drop_in_place(ptr as *mut M);
    // No handles means no usage. This also releases the page as unused for further
    // calls to `Page::from_unused`.
    (*ptr).usage.store(0, Ordering::Release);
    // Deallocate the page.
    // It would return the page to the allocator for further use. This would be done
    // after the release of the metadata to avoid re-allocation before the metadata
    // is reset.
    allocator::PAGE_ALLOCATOR.get().unwrap().lock().dealloc(
        mapping::meta_to_page::<PagingConsts>(ptr as Vaddr) / PAGE_SIZE,
        1,
    );
}

mod private {
    pub trait Sealed {}
}

// ======= Start of all the specific metadata structures definitions ==========

use private::Sealed;

#[derive(Debug, Default)]
#[repr(C)]
pub struct FrameMeta {
    // If not doing so, the page table metadata would fit
    // in the front padding of meta slot and make it 12 bytes.
    // We make it 16 bytes. Further usage of frame metadata
    // is welcome to exploit this space.
    _unused_for_layout_padding: [u8; 8],
}

impl Sealed for FrameMeta {}

/// The metadata of any kinds of page table pages.
/// Make sure the the generic parameters don't effect the memory layout.
#[derive(Debug)]
#[repr(C)]
pub(in crate::mm) struct PageTablePageMeta<
    E: PageTableEntryTrait = PageTableEntry,
    C: PagingConstsTrait = PagingConsts,
> where
    [(); C::NR_LEVELS as usize]:,
{
    /// The number of valid PTEs. It is mutable if the lock is held.
    pub nr_children: UnsafeCell<u16>,
    /// The level of the page table page. A page table page cannot be
    /// referenced by page tables of different levels.
    pub level: PagingLevel,
    /// Whether the pages mapped by the node is tracked.
    pub is_tracked: MapTrackingStatus,
    /// The lock for the page table page.
    pub lock: AtomicU8,
    _phantom: core::marker::PhantomData<(E, C)>,
}

/// Describe if the physical address recorded in this page table refers to a
/// page tracked by metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(in crate::mm) enum MapTrackingStatus {
    /// The page table node cannot contain references to any pages. It can only
    /// contain references to child page table nodes.
    NotApplicable,
    /// The mapped pages are not tracked by metadata. If any child page table
    /// nodes exist, they should also be tracked.
    Untracked,
    /// The mapped pages are tracked by metadata. If any child page table nodes
    /// exist, they should also be tracked.
    Tracked,
}

impl<E: PageTableEntryTrait, C: PagingConstsTrait> PageTablePageMeta<E, C>
where
    [(); C::NR_LEVELS as usize]:,
{
    pub fn new_locked(level: PagingLevel, is_tracked: MapTrackingStatus) -> Self {
        Self {
            nr_children: UnsafeCell::new(0),
            level,
            is_tracked,
            lock: AtomicU8::new(1),
            _phantom: PhantomData,
        }
    }
}

impl<E: PageTableEntryTrait, C: PagingConstsTrait> Sealed for PageTablePageMeta<E, C> where
    [(); C::NR_LEVELS as usize]:
{
}

unsafe impl<E: PageTableEntryTrait, C: PagingConstsTrait> Send for PageTablePageMeta<E, C> where
    [(); C::NR_LEVELS as usize]:
{
}

unsafe impl<E: PageTableEntryTrait, C: PagingConstsTrait> Sync for PageTablePageMeta<E, C> where
    [(); C::NR_LEVELS as usize]:
{
}

#[derive(Debug, Default)]
#[repr(C)]
pub struct MetaPageMeta {}

impl Sealed for MetaPageMeta {}
impl PageMeta for MetaPageMeta {
    const USAGE: PageUsage = PageUsage::Meta;
    fn on_drop(_page: &mut Page<Self>) {
        panic!("Meta pages are currently not allowed to be dropped");
    }
}

#[derive(Debug, Default)]
#[repr(C)]
pub struct KernelMeta {}

impl Sealed for KernelMeta {}
impl PageMeta for KernelMeta {
    const USAGE: PageUsage = PageUsage::Kernel;
    fn on_drop(_page: &mut Page<Self>) {
        panic!("Kernel pages are not allowed to be dropped");
    }
}

#[derive(Debug, Default)]
#[repr(C)]
pub struct KernelStackMeta {}

impl Sealed for KernelStackMeta {}
impl PageMeta for KernelStackMeta {
    const USAGE: PageUsage = PageUsage::KernelStack;
    fn on_drop(_page: &mut Page<Self>) {}
}

// ======== End of all the specific metadata structures definitions ===========

/// Initializes the metadata of all physical pages.
///
/// The function returns a list of `Page`s containing the metadata.
pub(crate) fn init() -> Vec<Page<MetaPageMeta>> {
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
    boot_pt::with_borrow(|boot_pt| {
        for (i, frame_paddr) in meta_pages.iter().enumerate() {
            let vaddr = mapping::page_to_meta::<PagingConsts>(0) + i * PAGE_SIZE;
            let prop = PageProperty {
                flags: PageFlags::RW,
                cache: CachePolicy::Writeback,
                priv_flags: PrivilegedPageFlags::GLOBAL,
            };
            // SAFETY: we are doing the metadata mappings for the kernel.
            unsafe { boot_pt.map_base_page(vaddr, frame_paddr / PAGE_SIZE, prop) };
        }
    })
    .unwrap();
    // Now the metadata pages are mapped, we can initialize the metadata.
    meta_pages
        .into_iter()
        .map(|paddr| Page::<MetaPageMeta>::from_unused(paddr, MetaPageMeta::default()))
        .collect()
}

fn alloc_meta_pages(nframes: usize) -> Vec<Paddr> {
    let mut meta_pages = Vec::new();
    let start_frame = allocator::PAGE_ALLOCATOR
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
