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
    any::Any,
    cell::UnsafeCell,
    mem::size_of,
    sync::atomic::{AtomicU32, Ordering},
};

use log::info;
use static_assertions::const_assert_eq;

use super::{allocator, Page};
use crate::{
    arch::mm::PagingConsts,
    mm::{
        paddr_to_vaddr, page_size, page_table::boot_pt, CachePolicy, Paddr, PageFlags,
        PageProperty, PrivilegedPageFlags, Vaddr, PAGE_SIZE,
    },
};

/// The maximum number of bytes of the metadata of a page.
pub const PAGE_METADATA_MAX_SIZE: usize =
    META_SLOT_SIZE - size_of::<AtomicU32>() - size_of::<PageMetaVtablePtr>();
/// The maximum alignment in bytes of the metadata of a page.
pub const PAGE_METADATA_MAX_ALIGN: usize = align_of::<MetaSlot>();

const META_SLOT_SIZE: usize = 64;

#[repr(C)]
pub(in crate::mm) struct MetaSlot {
    /// The metadata of the page.
    ///
    /// It is placed at the beginning of a slot because:
    ///  - the implementation can simply cast a `*const MetaSlot`
    ///    to a `*const PageMeta` for manipulation;
    ///  - if the metadata need special alignment, we can provide
    ///    at most `PAGE_METADATA_ALIGN` bytes of alignment;
    ///  - the subsequent fields can utilize the padding of the
    ///    reference count to save space.
    storage: UnsafeCell<[u8; PAGE_METADATA_MAX_SIZE]>,
    /// The reference count of the page.
    pub(super) ref_count: AtomicU32,
    /// The virtual table that indicates the type of the metadata.
    pub(super) vtable_ptr: UnsafeCell<PageMetaVtablePtr>,
}

type PageMetaVtablePtr = core::ptr::DynMetadata<dyn PageMeta>;

const_assert_eq!(PAGE_SIZE % META_SLOT_SIZE, 0);
const_assert_eq!(size_of::<MetaSlot>(), META_SLOT_SIZE);

/// All page metadata types must implement this trait.
///
/// If a page type needs specific drop behavior, it should specify
/// when implementing this trait. When we drop the last handle to
/// this page, the `on_drop` method will be called. The `on_drop`
/// method is called with the physical address of the page.
///
/// # Safety
///
/// The implemented structure must have a size less than or equal to
/// [`PAGE_METADATA_MAX_SIZE`] and an alignment less than or equal to
/// [`PAGE_METADATA_MAX_ALIGN`].
pub unsafe trait PageMeta: Any + Send + Sync + 'static {
    fn on_drop(&mut self, _paddr: Paddr) {}
}

/// Makes a structure usable as a page metadata.
///
/// Directly implementing [`PageMeta`] is not safe since the size and alignment
/// must be checked. This macro provides a safe way to implement the trait with
/// compile-time checks.
#[macro_export]
macro_rules! impl_page_meta {
    ($($t:ty),*) => {
        $(
            use static_assertions::const_assert;
            const_assert!(size_of::<$t>() <= $crate::mm::page::meta::PAGE_METADATA_MAX_SIZE);
            const_assert!(align_of::<$t>() <= $crate::mm::page::meta::PAGE_METADATA_MAX_ALIGN);
            // SAFETY: The size and alignment of the structure are checked.
            unsafe impl $crate::mm::page::meta::PageMeta for $t {}
        )*
    };
}

pub use impl_page_meta;

/// An internal routine in dropping implementations.
///
/// # Safety
///
/// The caller should ensure that the pointer points to a page's metadata slot. The
/// page should have a last handle to the page, and the page is about to be dropped,
/// as the metadata slot after this operation becomes uninitialized.
pub(super) unsafe fn drop_last_in_place(ptr: *mut MetaSlot) {
    // This would be guaranteed as a safety requirement.
    debug_assert_eq!((*ptr).ref_count.load(Ordering::Relaxed), 0);

    let paddr = mapping::meta_to_page::<PagingConsts>(ptr as Vaddr);

    let meta_ptr: *mut dyn PageMeta = core::ptr::from_raw_parts_mut(ptr, *(*ptr).vtable_ptr.get());

    // Let the custom dropper handle the drop.
    (*meta_ptr).on_drop(paddr);

    // Drop the metadata.
    core::ptr::drop_in_place(meta_ptr);

    // Deallocate the page.
    // It would return the page to the allocator for further use. This would be done
    // after the release of the metadata to avoid re-allocation before the metadata
    // is reset.
    allocator::PAGE_ALLOCATOR
        .get()
        .unwrap()
        .lock()
        .dealloc(paddr / PAGE_SIZE, 1);
}
/// The metadata of pages that holds metadata of pages.
#[derive(Debug, Default)]
pub struct MetaPageMeta {}

impl_page_meta!(MetaPageMeta);

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
