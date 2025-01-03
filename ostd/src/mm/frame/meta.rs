// SPDX-License-Identifier: MPL-2.0

//! Metadata management of frames.
//!
//! You can picture a globally shared, static, gigantic array of metadata
//! initialized for each frame. An entry in the array is called a [`MetaSlot`],
//! which contains the metadata of a frame. There would be a dedicated small
//! "heap" space in each slot for dynamic metadata. You can store anything as
//! the metadata of a frame as long as it's [`Sync`].
//!
//! # Implementation
//!
//! The slots are placed in the metadata pages mapped to a certain virtual
//! address in the kernel space. So finding the metadata of a frame often
//! comes with no costs since the translation is a simple arithmetic operation.

pub(crate) mod mapping {
    //! The metadata of each physical page is linear mapped to fixed virtual addresses
    //! in [`FRAME_METADATA_RANGE`].

    use core::mem::size_of;

    use super::MetaSlot;
    use crate::mm::{kspace::FRAME_METADATA_RANGE, Paddr, PagingConstsTrait, Vaddr, PAGE_SIZE};

    /// Converts a physical address of a base frame to the virtual address of the metadata slot.
    pub(crate) const fn frame_to_meta<C: PagingConstsTrait>(paddr: Paddr) -> Vaddr {
        let base = FRAME_METADATA_RANGE.start;
        let offset = paddr / PAGE_SIZE;
        base + offset * size_of::<MetaSlot>()
    }

    /// Converts a virtual address of the metadata slot to the physical address of the frame.
    pub(crate) const fn meta_to_frame<C: PagingConstsTrait>(vaddr: Vaddr) -> Paddr {
        let base = FRAME_METADATA_RANGE.start;
        let offset = (vaddr - base) / size_of::<MetaSlot>();
        offset * PAGE_SIZE
    }
}

use core::{
    any::Any,
    cell::UnsafeCell,
    fmt::Debug,
    mem::{size_of, MaybeUninit},
    sync::atomic::{AtomicU32, Ordering},
};

use align_ext::AlignExt;
use log::info;
use static_assertions::const_assert_eq;

use super::{allocator, Segment};
use crate::{
    arch::mm::PagingConsts,
    mm::{
        kspace::LINEAR_MAPPING_BASE_VADDR, paddr_to_vaddr, page_size, page_table::boot_pt,
        CachePolicy, Infallible, Paddr, PageFlags, PageProperty, PrivilegedPageFlags, Vaddr,
        VmReader, PAGE_SIZE,
    },
    panic::abort,
};

/// The maximum number of bytes of the metadata of a frame.
pub const FRAME_METADATA_MAX_SIZE: usize =
    META_SLOT_SIZE - size_of::<bool>() - size_of::<AtomicU32>() - size_of::<FrameMetaVtablePtr>();
/// The maximum alignment in bytes of the metadata of a frame.
pub const FRAME_METADATA_MAX_ALIGN: usize = align_of::<MetaSlot>();

const META_SLOT_SIZE: usize = 64;

#[repr(C)]
pub(in crate::mm) struct MetaSlot {
    /// The metadata of a frame.
    ///
    /// It is placed at the beginning of a slot because:
    ///  - the implementation can simply cast a `*const MetaSlot`
    ///    to a `*const AnyFrameMeta` for manipulation;
    ///  - if the metadata need special alignment, we can provide
    ///    at most `PAGE_METADATA_ALIGN` bytes of alignment;
    ///  - the subsequent fields can utilize the padding of the
    ///    reference count to save space.
    ///
    /// Don't access this field with a reference to the slot.
    _storage: UnsafeCell<[u8; FRAME_METADATA_MAX_SIZE]>,
    /// The reference count of the page.
    ///
    /// Specifically, the reference count has the following meaning:
    ///  - `REF_COUNT_UNUSED`: The page is not in use.
    ///  - `0`: The page is being constructed ([`Frame::from_unused`])
    ///    or destructured ([`drop_last_in_place`]).
    ///  - `1..REF_COUNT_MAX`: The page is in use.
    ///  - `REF_COUNT_MAX..REF_COUNT_UNUSED`: Illegal values to
    ///    prevent the reference count from overflowing. Otherwise,
    ///    overflowing the reference count will cause soundness issue.
    ///
    /// [`Frame::from_unused`]: super::Frame::from_unused
    //
    // Other than this field the fields should be `MaybeUninit`.
    // See initialization in `alloc_meta_frames`.
    pub(super) ref_count: AtomicU32,
    /// The virtual table that indicates the type of the metadata.
    pub(super) vtable_ptr: UnsafeCell<MaybeUninit<FrameMetaVtablePtr>>,
}

pub(super) const REF_COUNT_UNUSED: u32 = u32::MAX;
const REF_COUNT_MAX: u32 = i32::MAX as u32;

type FrameMetaVtablePtr = core::ptr::DynMetadata<dyn AnyFrameMeta>;

const_assert_eq!(PAGE_SIZE % META_SLOT_SIZE, 0);
const_assert_eq!(size_of::<MetaSlot>(), META_SLOT_SIZE);

/// All frame metadata types must implement this trait.
///
/// If a frame type needs specific drop behavior, it should specify
/// when implementing this trait. When we drop the last handle to
/// this frame, the `on_drop` method will be called. The `on_drop`
/// method is called with the physical address of the frame.
///
/// # Safety
///
/// The implemented structure must have a size less than or equal to
/// [`FRAME_METADATA_MAX_SIZE`] and an alignment less than or equal to
/// [`FRAME_METADATA_MAX_ALIGN`].
///
/// The implementer of the `on_drop` method should ensure that the frame is
/// safe to be read.
pub unsafe trait AnyFrameMeta: Any + Send + Sync + Debug + 'static {
    /// Called when the last handle to the frame is dropped.
    fn on_drop(&mut self, reader: &mut VmReader<Infallible>) {
        let _ = reader;
    }

    /// Whether the metadata's associated frame is untyped.
    ///
    /// If a type implements [`AnyUFrameMeta`], this should be `true`.
    /// Otherwise, it should be `false`.
    ///
    /// [`AnyUFrameMeta`]: super::untyped::AnyUFrameMeta
    fn is_untyped(&self) -> bool {
        false
    }
}

/// Makes a structure usable as a frame metadata.
///
/// Directly implementing [`AnyFrameMeta`] is not safe since the size and alignment
/// must be checked. This macro provides a safe way to implement the trait with
/// compile-time checks.
#[macro_export]
macro_rules! impl_frame_meta_for {
    // Implement without specifying the drop behavior.
    ($t:ty) => {
        static_assertions::const_assert!(
            size_of::<$t>() <= $crate::mm::frame::meta::FRAME_METADATA_MAX_SIZE
        );
        static_assertions::const_assert!(
            align_of::<$t>() <= $crate::mm::frame::meta::FRAME_METADATA_MAX_ALIGN
        );
        // SAFETY: The size and alignment of the structure are checked.
        unsafe impl $crate::mm::frame::meta::AnyFrameMeta for $t {}
    };
}

pub use impl_frame_meta_for;

impl MetaSlot {
    /// Increases the frame reference count by one.
    ///
    /// # Safety
    ///
    /// The caller must have already held a reference to the frame.
    pub(super) unsafe fn inc_ref_count(&self) {
        let last_ref_cnt = self.ref_count.fetch_add(1, Ordering::Relaxed);
        debug_assert!(last_ref_cnt != 0 && last_ref_cnt != REF_COUNT_UNUSED);

        if last_ref_cnt >= REF_COUNT_MAX {
            // This follows the same principle as the `Arc::clone` implementation to prevent the
            // reference count from overflowing. See also
            // <https://doc.rust-lang.org/std/sync/struct.Arc.html#method.clone>.
            abort();
        }
    }
}

/// An internal routine in dropping implementations.
///
/// # Safety
///
/// The caller should ensure that the pointer points to a frame's metadata slot. The
/// frame should have a last handle to the frame, and the frame is about to be dropped,
/// as the metadata slot after this operation becomes uninitialized.
pub(super) unsafe fn drop_last_in_place(ptr: *mut MetaSlot) {
    // SAFETY: `ptr` points to a valid `MetaSlot` that will never be mutably borrowed, so taking an
    // immutable reference to it is always safe.
    let slot = unsafe { &*ptr };

    // This should be guaranteed as a safety requirement.
    debug_assert_eq!(slot.ref_count.load(Ordering::Relaxed), 0);

    let paddr = mapping::meta_to_frame::<PagingConsts>(ptr as Vaddr);

    // SAFETY: We have exclusive access to the frame metadata.
    let vtable_ptr = unsafe { &mut *slot.vtable_ptr.get() };
    // SAFETY: The frame metadata is initialized and valid.
    let vtable_ptr = unsafe { vtable_ptr.assume_init_read() };

    let meta_ptr: *mut dyn AnyFrameMeta = core::ptr::from_raw_parts_mut(ptr, vtable_ptr);

    // SAFETY: The implementer of the frame metadata decides that if the frame
    // is safe to be read or not.
    let mut reader =
        unsafe { VmReader::from_kernel_space(paddr_to_vaddr(paddr) as *const u8, PAGE_SIZE) };

    // SAFETY: `ptr` points to the metadata storage which is valid to be mutably borrowed under
    // `vtable_ptr` because the metadata is valid, the vtable is correct, and we have the exclusive
    // access to the frame metadata.
    unsafe {
        // Invoke the custom `on_drop` handler.
        (*meta_ptr).on_drop(&mut reader);
        // Drop the frame metadata.
        core::ptr::drop_in_place(meta_ptr);
    }

    // `Release` pairs with the `Acquire` in `Frame::from_unused` and ensures `drop_in_place` won't
    // be reordered after this memory store.
    slot.ref_count.store(REF_COUNT_UNUSED, Ordering::Release);

    // Deallocate the frame.
    // It would return the frame to the allocator for further use. This would be done
    // after the release of the metadata to avoid re-allocation before the metadata
    // is reset.
    allocator::FRAME_ALLOCATOR
        .get()
        .unwrap()
        .lock()
        .dealloc(paddr / PAGE_SIZE, 1);
}

/// The metadata of frames that holds metadata of frames.
#[derive(Debug, Default)]
pub struct MetaPageMeta {}

impl_frame_meta_for!(MetaPageMeta);

/// Initializes the metadata of all physical frames.
///
/// The function returns a list of `Frame`s containing the metadata.
pub(crate) fn init() -> Segment<MetaPageMeta> {
    let max_paddr = {
        let regions = &crate::boot::EARLY_INFO.get().unwrap().memory_regions;
        regions.iter().map(|r| r.base() + r.len()).max().unwrap()
    };

    info!(
        "Initializing frame metadata for physical memory up to {:x}",
        max_paddr
    );

    add_temp_linear_mapping(max_paddr);

    super::MAX_PADDR.store(max_paddr, Ordering::Relaxed);

    let tot_nr_frames = max_paddr / page_size::<PagingConsts>(1);
    let (nr_meta_pages, meta_pages) = alloc_meta_frames(tot_nr_frames);

    // Map the metadata frames.
    boot_pt::with_borrow(|boot_pt| {
        for i in 0..nr_meta_pages {
            let frame_paddr = meta_pages + i * PAGE_SIZE;
            let vaddr = mapping::frame_to_meta::<PagingConsts>(0) + i * PAGE_SIZE;
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

    // Now the metadata frames are mapped, we can initialize the metadata.
    Segment::from_unused(meta_pages..meta_pages + nr_meta_pages * PAGE_SIZE, |_| {
        MetaPageMeta {}
    })
}

fn alloc_meta_frames(tot_nr_frames: usize) -> (usize, Paddr) {
    let nr_meta_pages = tot_nr_frames
        .checked_mul(size_of::<MetaSlot>())
        .unwrap()
        .div_ceil(PAGE_SIZE);
    let start_paddr = allocator::FRAME_ALLOCATOR
        .get()
        .unwrap()
        .lock()
        .alloc(nr_meta_pages)
        .unwrap()
        * PAGE_SIZE;

    let slots = paddr_to_vaddr(start_paddr) as *mut MetaSlot;

    // Fill the metadata frames with a byte pattern of `REF_COUNT_UNUSED`.
    debug_assert_eq!(REF_COUNT_UNUSED.to_ne_bytes(), [0xff, 0xff, 0xff, 0xff]);
    // SAFETY: `slots` and the length is a valid region for the metadata frames
    // that are going to be treated as metadata slots. The byte pattern is
    // valid as the initial value of the reference count (other fields are
    // either not accessed or `MaybeUninit`).
    unsafe {
        core::ptr::write_bytes(
            slots as *mut u8,
            0xff,
            tot_nr_frames * size_of::<MetaSlot>(),
        );
    }

    (nr_meta_pages, start_paddr)
}

/// Adds a temporary linear mapping for the metadata frames.
///
/// We only assume boot page table to contain 4G linear mapping. Thus if the
/// physical memory is huge we end up depleted of linear virtual memory for
/// initializing metadata.
fn add_temp_linear_mapping(max_paddr: Paddr) {
    const PADDR4G: Paddr = 0x1_0000_0000;

    if max_paddr <= PADDR4G {
        return;
    }

    // TODO: We don't know if the allocator would allocate from low to high or
    // not. So we prepare all linear mappings in the boot page table. Hope it
    // won't drag the boot performance much.
    let end_paddr = max_paddr.align_up(PAGE_SIZE);
    let prange = PADDR4G..end_paddr;
    let prop = PageProperty {
        flags: PageFlags::RW,
        cache: CachePolicy::Writeback,
        priv_flags: PrivilegedPageFlags::GLOBAL,
    };

    // SAFETY: we are doing the linear mapping for the kernel.
    unsafe {
        boot_pt::with_borrow(|boot_pt| {
            for paddr in prange.step_by(PAGE_SIZE) {
                let vaddr = LINEAR_MAPPING_BASE_VADDR + paddr;
                boot_pt.map_base_page(vaddr, paddr / PAGE_SIZE, prop);
            }
        })
        .unwrap();
    }
}
