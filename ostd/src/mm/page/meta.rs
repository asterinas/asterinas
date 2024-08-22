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
    pub(crate) const fn page_to_meta<C: PagingConstsTrait>(paddr: Paddr) -> Vaddr {
        let base = FRAME_METADATA_RANGE.start;
        let offset = paddr / PAGE_SIZE;
        base + offset * size_of::<MetaSlot>()
    }

    /// Converts a virtual address of the metadata slot to the physical address of the page.
    pub(crate) const fn meta_to_page<C: PagingConstsTrait>(vaddr: Vaddr) -> Paddr {
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

use allocator::{PageAlloc, BOOTSTRAP_PAGE_ALLOCATOR};
use log::info;
use num_derive::FromPrimitive;
use static_assertions::const_assert_eq;

use super::{allocator, Page};
use crate::{
    arch::mm::{PageTableEntry, PagingConsts},
    mm::{
        paddr_to_vaddr,
        page::MAX_PADDR,
        page_size,
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
    /// The page is free, i.e. the page can be used for any purpose.
    Free = 0,
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
    /// The page is used by the boot page table.
    BootPageTable = 68,
    /// The page is used as a heap page.
    Heap = 69,
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

    /// The Reference Count of the page.
    ///
    /// Note that the meaning of the reference count is arraged as follows:
    /// - 0: The page is free and not referenced by anyone.
    /// - 1: The page is referenced by [`FreePage`] or [`Page`] and is not in
    ///   use, meaning that the page is to-be-allotted or to-be-released. You
    ///   can refer to `FreePage::Drop` and `meta::drop_as_last` for more
    ///   details.
    /// - others: The page is referenced by `ref_count - 1` handle(s) and is in
    ///   use.
    pub(super) ref_count: AtomicU32,
}

pub(super) union MetaSlotInner {
    _linklist: ManuallyDrop<FreeMeta>,
    _frame: ManuallyDrop<FrameMeta>,
    _pt: ManuallyDrop<PageTablePageMeta>,
}

// Currently the sizes of the `MetaSlotInner` union variants are no larger
// than 24 bytes and aligned to 24 bytes. So the size of `MetaSlot` is 32 bytes.
//
// Note that the size of `MetaSlot` should be a multiple of 8 bytes to prevent
// cross-page accesses.
const_assert_eq!(size_of::<MetaSlot>(), 32);

/// All page metadata types must implemented this sealed trait,
/// which ensures that each fields of `PageUsage` has one and only
/// one metadata type corresponding to the usage purpose.
///
/// Any user outside this module won't be able to add more metadata types
/// and break assumptions made by this module.
///
/// If a page type needs specific drop behavior, it should specify
/// when implementing this trait. When we drop the last handle to
/// this page, the `on_drop` method will be called.
pub trait PageMeta: Sync + private::Sealed + Sized {
    /// The usage of the page.
    const USAGE: PageUsage;

    /// The custom drop behavior of the page.
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
    debug_assert_eq!((*ptr).ref_count.load(Ordering::Relaxed), 1);
    // Let the custom dropper handle the drop.
    let mut page = Page::<M> {
        ptr,
        _marker: PhantomData,
    };
    // (*ptr).ref_count.fetch_add(1, Ordering::Relaxed);
    M::on_drop(&mut page);
    let _ = ManuallyDrop::new(page);
    // Drop the metadata.
    core::ptr::drop_in_place(ptr as *mut M);

    // Reset the metadata slot.
    // No handles means no usage. This also releases the page as free for further
    // calls to `Page::from_free`.
    // Set the reference count to 0.
    (*ptr)
        .ref_count
        .compare_exchange(1, 0, Ordering::SeqCst, Ordering::Relaxed)
        .expect("The reference count should be 1 while dropping as the last handle");
    // Set the usage to free.
    (*ptr).usage.store(0, Ordering::Release);

    // Deallocate the page.
    // It would return the page to the allocator for further use. This would be done
    // after the release of the metadata to avoid re-allocation before the metadata
    // is reset.
    allocator::PAGE_ALLOCATOR
        .get()
        .unwrap()
        .disable_irq()
        .lock()
        .dealloc(mapping::meta_to_page::<PagingConsts>(ptr as Vaddr), 1);
}

mod private {
    pub trait Sealed {}
}

// ======= Start of all the specific metadata structures definitions ==========

use private::Sealed;

/// FreeMeta: Bidirectional unordered circular linked list of free pages.
///
/// Furnish memory modules out of `OSTD` with safe, efficient linked list
/// to manage free pages. The linked list is arranged as follows:
///
/// `(paddr → freemeta) — next/prev —> (paddr → freemeta) — next/prev —>…`
#[derive(Debug, Default)]
#[repr(C)]
pub struct FreeMeta {
    value: usize,
    // The `prev` and `next` pointers store the **physical addresses**
    // of the designated memory pages.
    prev: Paddr,
    next: Paddr,
}

impl Sealed for FreeMeta {}

/// Free Pages: The only referencer to a specific free page.
#[derive(Debug)]
#[repr(C)]
pub struct FreePage {
    pub(in crate::mm::page) ptr: *const MetaSlot,
}

impl FreePage {
    /// Get a Free Page handle if the paddr's corresponding metadata is free
    /// and is not referenced by anyone.
    ///
    /// By leveraging this function, we can ensure that the page is not
    /// referenced by anyone else, which is critical to improve concurrency
    /// safety while transferring the page's usage.
    ///
    /// # Panic:
    ///
    /// The function panics if:
    ///  - the physical address is out of bound or not aligned;
    ///  - the page is already in use.
    pub fn try_lock(paddr: Paddr) -> Option<Self> {
        assert!(paddr % PAGE_SIZE == 0);
        assert!(paddr < MAX_PADDR.load(Ordering::Relaxed) as Paddr);
        let vaddr = mapping::page_to_meta::<PagingConsts>(paddr);
        let ptr = vaddr as *const MetaSlot;

        // SAFETY: The aligned pointer points to a initialized `MetaSlot`.
        let usage = unsafe { &(*ptr).usage };
        // SAFETY: The aligned pointer points to a initialized `MetaSlot`.
        let ref_count = unsafe { &(*ptr).ref_count };

        if ref_count
            .compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            debug_assert_eq!(usage.load(Ordering::Relaxed), PageUsage::Free as u8);
            Some(Self { ptr })
        } else {
            None
        }
    }

    /// Get the mutable metadata of the free page
    /// Non-const reference to the metadata is safe because the page is locked.
    /// Concurrency safety is guaranteed by the method [`try_lock`].
    pub fn meta_mut(&mut self) -> &mut FreeMeta {
        let raw_ptr = self as *const Self as *mut Self;
        unsafe {
            let cell_ptr = raw_ptr as *mut UnsafeCell<FreeMeta>;
            &mut *cell_ptr.as_mut().unwrap().get()
        }
    }

    /// Get the metadata of the free page.
    pub fn meta(&self) -> &FreeMeta {
        let raw_ptr = self as *const Self as *mut Self;
        unsafe {
            let cell_ptr = raw_ptr as *mut UnsafeCell<FreeMeta>;
            &*cell_ptr.as_mut().unwrap().get()
        }
    }
}

impl Drop for FreePage {
    /// Drop the FreePage, decrease the reference count of the corresponding
    /// page.
    fn drop(&mut self) {
        let ptr = self.ptr;

        // SAFETY: The aligned pointer points to a initialized `MetaSlot`.
        let ref_count = unsafe { &(*ptr).ref_count };
        let last_ref_cnt = ref_count.fetch_sub(1, Ordering::Release);

        debug_assert!(last_ref_cnt > 0);
    }
}

/// Implementations linklist operations for FreePage.
impl FreePage {
    /// Initialize the circular linked list node.
    pub fn init(&mut self) {
        let paddr = self.paddr();
        let meta = self.meta_mut();
        meta.prev = paddr;
        meta.next = paddr;
    }

    /// Get the physical address.
    pub fn paddr(&self) -> Paddr {
        mapping::meta_to_page::<PagingConsts>(self.ptr as Vaddr)
    }

    /// Get the value of the free page linked list.
    pub fn value(&self) -> usize {
        self.meta().value
    }

    /// Get the previous free page.
    pub fn prev(&self) -> Option<Paddr> {
        let current = self.paddr();
        let meta = self.meta();
        if meta.prev == current {
            // The previous page is the current page itself.
            None
        } else {
            Some(meta.prev)
        }
    }

    /// Get the next free page.
    pub fn next(&self) -> Option<Paddr> {
        let current = self.paddr();
        let meta = self.meta();
        if meta.next == current {
            // The next page is the current page itself.
            None
        } else {
            Some(meta.next)
        }
    }

    /// Set the value of the free page linked list.
    pub fn set_value(&mut self, value: usize) {
        self.meta_mut().value = value;
    }

    /// Insert a free page after the current free page.
    pub fn insert_after(&mut self, new_addr: Paddr) {
        let cur = self.paddr();
        let mut new_page = FreePage::try_lock(new_addr).expect("The new page is not free");
        let new_meta = new_page.meta_mut();

        if let Some(next) = self.next() {
            // The next page is not the current page itself.
            // Avoid deadlock when the next page is the current page.
            let mut next_page = FreePage::try_lock(next).expect("The next page is not free");
            let next_meta = next_page.meta_mut();

            new_meta.next = next;
            next_meta.prev = new_addr;
        } else {
            new_meta.next = cur;
            self.meta_mut().prev = new_addr;
        }

        self.meta_mut().next = new_addr;
        new_meta.prev = cur;
    }

    /// Insert a free page before the current free page.
    pub fn insert_before(&mut self, new_addr: Paddr) {
        let cur = self.paddr();
        let mut new_page = FreePage::try_lock(new_addr).expect("The new page is not free");
        let new_meta = new_page.meta_mut();

        if let Some(prev) = self.prev() {
            // The previous page is not the current page itself.
            // Avoid deadlock when the previous page is the current page.
            let mut prev_page = FreePage::try_lock(prev).expect("The previous page is not free");
            let prev_meta = prev_page.meta_mut();

            new_meta.prev = prev;
            prev_meta.next = new_addr;
        } else {
            new_meta.prev = cur;
            self.meta_mut().next = new_addr;
        }

        self.meta_mut().prev = new_addr;
        new_meta.next = cur;
    }

    /// Remove the current free page from the list.
    /// Return true if the linked list is not empty after the removal.
    pub fn remove(&mut self) -> bool {
        let cur = self.paddr();
        if let Some(next) = self.next() {
            // Current node is not the only node in the list.
            let mut next_page = FreePage::try_lock(next).expect("The next page is not free");
            let next_meta = next_page.meta_mut();

            let prev = self.prev().expect("Logical Error: The previous page is current page itself while the next page is not!");
            if let Some(mut prev_page) = FreePage::try_lock(prev) {
                let prev_meta = prev_page.meta_mut();
                prev_meta.next = next;
                next_meta.prev = prev;
            } else {
                // Special handle when linked list's size is 2.
                // When the size is 2, the previous page is also the next page.
                // After the removal, the list has only one node, which is the
                // `next_page`. Without this special handle, this removal would
                // cause a deadlock.
                debug_assert_eq!(next, prev);
                next_meta.next = next;
                next_meta.prev = next;
            }

            let cur_meta = self.meta_mut();
            cur_meta.next = cur;
            cur_meta.prev = cur;
            cur_meta.value = 0;

            true
        } else {
            // Special handle when the linked list's size is 1.
            // Current node is the only node in the list.
            // After the removal, the list is empty.
            let cur_meta = self.meta_mut();
            cur_meta.next = cur;
            cur_meta.prev = cur;
            cur_meta.value = 0;

            false
        }
    }
}

/// The metadata of the page used as a frame.
/// i.e., The corresponding page is a page of untyped memory.
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
pub(crate) struct MetaPageMeta {}

impl Sealed for MetaPageMeta {}
impl PageMeta for MetaPageMeta {
    const USAGE: PageUsage = PageUsage::Meta;
    fn on_drop(_page: &mut Page<Self>) {
        panic!("Meta pages are currently not allowed to be dropped");
    }
}

#[derive(Debug, Default)]
#[repr(C)]
pub(crate) struct KernelMeta {}

impl Sealed for KernelMeta {}
impl PageMeta for KernelMeta {
    const USAGE: PageUsage = PageUsage::Kernel;
    fn on_drop(_page: &mut Page<Self>) {
        panic!("Kernel pages are not allowed to be dropped");
    }
}

/// Metadata of a kernel stack page.
#[derive(Debug, Default)]
#[repr(C)]
pub struct KernelStackMeta {}

impl Sealed for KernelStackMeta {}
impl PageMeta for KernelStackMeta {
    const USAGE: PageUsage = PageUsage::KernelStack;
    fn on_drop(_page: &mut Page<Self>) {}
}

#[derive(Debug, Default)]
#[repr(C)]
pub(crate) struct BootPageTableMeta {}

impl Sealed for BootPageTableMeta {}
impl PageMeta for BootPageTableMeta {
    const USAGE: PageUsage = PageUsage::BootPageTable;
    fn on_drop(_page: &mut Page<Self>) {
        // Do noting.
    }
}

/// The metadata of a heap page.
#[derive(Debug, Default)]
#[repr(C)]
pub struct HeapMeta {}
impl Sealed for HeapMeta {}
impl PageMeta for HeapMeta {
    const USAGE: PageUsage = PageUsage::Heap;
    fn on_drop(_page: &mut Page<Self>) {
        // Nothing should be done so far since dropping the page would
        // have all taken care of.
    }
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
    boot_pt::with_borrow(|boot_pt| {
        boot_pt.manage_frames_with_meta();
    })
    .expect("Failed to manage frames with metadata");
    meta_pages
        .into_iter()
        .map(|paddr| Page::<MetaPageMeta>::from_free(paddr, MetaPageMeta::default()))
        .collect()
}

fn alloc_meta_pages(nframes: usize) -> Vec<Paddr> {
    let mut meta_pages = Vec::new();
    let mut allocator = BOOTSTRAP_PAGE_ALLOCATOR.get().unwrap().disable_irq().lock();
    for _ in 0..nframes {
        let frame_paddr = allocator
            .alloc_page(PAGE_SIZE)
            .unwrap_or_else(|| panic!("Failed to allocate metadata pages"));
        // Zero them out as initialization.
        let vaddr = paddr_to_vaddr(frame_paddr) as *mut u8;
        unsafe { core::ptr::write_bytes(vaddr, 0, PAGE_SIZE) };
        meta_pages.push(frame_paddr);
    }
    meta_pages
}
