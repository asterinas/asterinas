// SPDX-License-Identifier: MPL-2.0

//! Metadata management of frames.
//!
//! You can picture a globally shared, static, gigantic array of metadata
//! initialized for each frame.
//! Each entry in this array holds the metadata for a single frame.
//! There would be a dedicated small
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
    alloc::Layout,
    any::Any,
    cell::UnsafeCell,
    fmt::Debug,
    mem::{size_of, ManuallyDrop, MaybeUninit},
    result::Result,
    sync::atomic::{AtomicU64, Ordering},
};

use align_ext::AlignExt;
use log::info;

use crate::{
    arch::mm::PagingConsts,
    boot::memory_region::MemoryRegionType,
    const_assert,
    mm::{
        frame::allocator::{self, EarlyAllocatedFrameMeta},
        kspace::LINEAR_MAPPING_BASE_VADDR,
        paddr_to_vaddr, page_size,
        page_table::boot_pt,
        CachePolicy, Infallible, Paddr, PageFlags, PageProperty, PrivilegedPageFlags, Segment,
        Vaddr, VmReader, PAGE_SIZE,
    },
    panic::abort,
    util::ops::range_difference,
};

/// The maximum number of bytes of the metadata of a frame.
pub const FRAME_METADATA_MAX_SIZE: usize = META_SLOT_SIZE
    - size_of::<AtomicU64>()
    - size_of::<FrameMetaVtablePtr>()
    - size_of::<AtomicU64>();
/// The maximum alignment in bytes of the metadata of a frame.
pub const FRAME_METADATA_MAX_ALIGN: usize = META_SLOT_SIZE;

const META_SLOT_SIZE: usize = 96;

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
    /// Don't interpret this field as an array of bytes. It is a
    /// placeholder for the metadata of a frame.
    storage: UnsafeCell<[u8; FRAME_METADATA_MAX_SIZE]>,
    /// The reference count of the page.
    ///
    /// Specifically, the reference count has the following meaning:
    ///  - `REF_COUNT_UNUSED`: The page is not in use.
    ///  - `REF_COUNT_UNIQUE`: The page is owned by a [`UniqueFrame`].
    ///  - `0`: The page is being constructed ([`Frame::from_unused`])
    ///    or destructured ([`drop_last_in_place`]).
    ///  - `1..REF_COUNT_MAX`: The page is in use.
    ///  - `REF_COUNT_MAX..REF_COUNT_UNIQUE`: Illegal values to
    ///    prevent the reference count from overflowing. Otherwise,
    ///    overflowing the reference count will cause soundness issue.
    ///
    /// [`Frame::from_unused`]: super::Frame::from_unused
    /// [`UniqueFrame`]: super::unique::UniqueFrame
    /// [`drop_last_in_place`]: Self::drop_last_in_place
    //
    // Other than this field the fields should be `MaybeUninit`.
    // See initialization in `alloc_meta_frames`.
    pub(super) ref_count: AtomicU64,
    /// The virtual table that indicates the type of the metadata.
    pub(super) vtable_ptr: UnsafeCell<MaybeUninit<FrameMetaVtablePtr>>,
    /// This is only accessed by [`crate::mm::frame::linked_list`].
    /// It stores 0 if the frame is not in any list, otherwise it stores the
    /// ID of the list.
    ///
    /// It is ugly but allows us to tell if a frame is in a specific list by
    /// one relaxed read. Otherwise, if we store it conditionally in `storage`
    /// we would have to ensure that the type is correct before the read, which
    /// costs a synchronization.
    pub(super) in_list: AtomicU64,
}

pub(super) const REF_COUNT_UNUSED: u64 = u64::MAX;
pub(super) const REF_COUNT_UNIQUE: u64 = u64::MAX - 1;
pub(super) const REF_COUNT_MAX: u64 = i64::MAX as u64;

type FrameMetaVtablePtr = core::ptr::DynMetadata<dyn AnyFrameMeta>;

// const_assert!(PAGE_SIZE % META_SLOT_SIZE == 0);
const_assert!(size_of::<MetaSlot>() == META_SLOT_SIZE);

/// All frame metadata types must implement this trait.
///
/// If a frame type needs specific drop behavior, it should specify
/// when implementing this trait. When we drop the last handle to
/// this frame, the `on_drop` method will be called. The `on_drop`
/// method is called with the physical address of the frame.
///
/// The implemented structure should have a size less than or equal to
/// [`FRAME_METADATA_MAX_SIZE`] and an alignment less than or equal to
/// [`FRAME_METADATA_MAX_ALIGN`]. Otherwise, the metadata type cannot
/// be used because storing it will fail compile-time assertions.
///
/// # Safety
///
/// If `on_drop` reads the page using the provided `VmReader`, the
/// implementer must ensure that the frame is safe to read.
pub unsafe trait AnyFrameMeta: Any + Send + Sync {
    /// Called when the last handle to the frame is dropped.
    fn on_drop(&mut self, _reader: &mut VmReader<Infallible>) {}

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
#[macro_export]
macro_rules! impl_frame_meta_for {
    // Implement without specifying the drop behavior.
    ($t:ty) => {
        // SAFETY: `on_drop` won't read the page.
        unsafe impl $crate::mm::frame::meta::AnyFrameMeta for $t {}

        $crate::const_assert!(
            core::mem::size_of::<$t>() <= $crate::mm::frame::meta::FRAME_METADATA_MAX_SIZE
        );
        $crate::const_assert!(
            $crate::mm::frame::meta::FRAME_METADATA_MAX_ALIGN % core::mem::align_of::<$t>() == 0
        );
    };
}

pub use impl_frame_meta_for;

/// The error type for getting the frame from a physical address.
#[derive(Debug)]
pub enum GetFrameError {
    /// The frame is in use.
    InUse,
    /// The frame is not in use.
    Unused,
    /// The frame is being initialized or destructed.
    Busy,
    /// The frame is private to an owner of [`UniqueFrame`].
    ///
    /// [`UniqueFrame`]: super::unique::UniqueFrame
    Unique,
    /// The provided physical address is out of bound.
    OutOfBound,
    /// The provided physical address is not aligned.
    NotAligned,
}

/// Gets the reference to a metadata slot.
pub(super) fn get_slot(paddr: Paddr) -> Result<&'static MetaSlot, GetFrameError> {
    if paddr % PAGE_SIZE != 0 {
        return Err(GetFrameError::NotAligned);
    }
    if paddr >= super::max_paddr() {
        return Err(GetFrameError::OutOfBound);
    }

    let vaddr = mapping::frame_to_meta::<PagingConsts>(paddr);
    let ptr = vaddr as *mut MetaSlot;

    // SAFETY: `ptr` points to a valid `MetaSlot` that will never be
    // mutably borrowed, so taking an immutable reference to it is safe.
    Ok(unsafe { &*ptr })
}

impl MetaSlot {
    /// Initializes the metadata slot of a frame assuming it is unused.
    ///
    /// If successful, the function returns a pointer to the metadata slot.
    /// And the slot is initialized with the given metadata.
    ///
    /// The resulting reference count held by the returned pointer is
    /// [`REF_COUNT_UNIQUE`] if `as_unique_ptr` is `true`, otherwise `1`.
    pub(super) fn get_from_unused<M: AnyFrameMeta>(
        paddr: Paddr,
        metadata: M,
        as_unique_ptr: bool,
    ) -> Result<*const Self, GetFrameError> {
        let slot = get_slot(paddr)?;

        // `Acquire` pairs with the `Release` in `drop_last_in_place` and ensures the metadata
        // initialization won't be reordered before this memory compare-and-exchange.
        slot.ref_count
            .compare_exchange(REF_COUNT_UNUSED, 0, Ordering::Acquire, Ordering::Relaxed)
            .map_err(|val| match val {
                REF_COUNT_UNIQUE => GetFrameError::Unique,
                0 => GetFrameError::Busy,
                _ => GetFrameError::InUse,
            })?;

        // SAFETY: The slot now has a reference count of `0`, other threads will
        // not access the metadata slot so it is safe to have a mutable reference.
        unsafe { slot.write_meta(metadata) };

        if as_unique_ptr {
            // No one can create a `Frame` instance directly from the page
            // address, so `Relaxed` is fine here.
            slot.ref_count.store(REF_COUNT_UNIQUE, Ordering::Relaxed);
        } else {
            // `Release` is used to ensure that the metadata initialization
            // won't be reordered after this memory store.
            slot.ref_count.store(1, Ordering::Release);
        }

        Ok(slot as *const MetaSlot)
    }

    /// Gets another owning pointer to the metadata slot from the given page.
    pub(super) fn get_from_in_use(paddr: Paddr) -> Result<*const Self, GetFrameError> {
        let slot = get_slot(paddr)?;

        // Try to increase the reference count for an in-use frame. Otherwise fail.
        loop {
            match slot.ref_count.load(Ordering::Relaxed) {
                REF_COUNT_UNUSED => return Err(GetFrameError::Unused),
                REF_COUNT_UNIQUE => return Err(GetFrameError::Unique),
                0 => return Err(GetFrameError::Busy),
                last_ref_cnt => {
                    if last_ref_cnt >= REF_COUNT_MAX {
                        // See `Self::inc_ref_count` for the explanation.
                        abort();
                    }
                    // Using `Acquire` here to pair with `get_from_unused` or
                    // `<Frame<M> as From<UniqueFrame<M>>>::from` (who must be
                    // performed after writing the metadata).
                    //
                    // It ensures that the written metadata will be visible to us.
                    if slot
                        .ref_count
                        .compare_exchange_weak(
                            last_ref_cnt,
                            last_ref_cnt + 1,
                            Ordering::Acquire,
                            Ordering::Relaxed,
                        )
                        .is_ok()
                    {
                        return Ok(slot as *const MetaSlot);
                    }
                }
            }
            core::hint::spin_loop();
        }
    }

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

    /// Gets the corresponding frame's physical address.
    pub(super) fn frame_paddr(&self) -> Paddr {
        mapping::meta_to_frame::<PagingConsts>(self as *const MetaSlot as Vaddr)
    }

    /// Gets a dynamically typed pointer to the stored metadata.
    ///
    /// # Safety
    ///
    /// The caller should ensure that:
    ///  - the stored metadata is initialized (by [`Self::write_meta`]) and valid.
    ///
    /// The returned pointer should not be dereferenced as mutable unless having
    /// exclusive access to the metadata slot.
    pub(super) unsafe fn dyn_meta_ptr(&self) -> *mut dyn AnyFrameMeta {
        // SAFETY: The page metadata is valid to be borrowed immutably, since
        // it will never be borrowed mutably after initialization.
        let vtable_ptr = unsafe { *self.vtable_ptr.get() };

        // SAFETY: The page metadata is initialized and valid.
        let vtable_ptr = *unsafe { vtable_ptr.assume_init_ref() };

        let meta_ptr: *mut dyn AnyFrameMeta =
            core::ptr::from_raw_parts_mut(self as *const MetaSlot as *mut MetaSlot, vtable_ptr);

        meta_ptr
    }

    /// Gets the stored metadata as type `M`.
    ///
    /// Calling the method should be safe, but using the returned pointer would
    /// be unsafe. Specifically, the derefernecer should ensure that:
    ///  - the stored metadata is initialized (by [`Self::write_meta`]) and
    ///    valid;
    ///  - the initialized metadata is of type `M`;
    ///  - the returned pointer should not be dereferenced as mutable unless
    ///    having exclusive access to the metadata slot.
    pub(super) fn as_meta_ptr<M: AnyFrameMeta>(&self) -> *mut M {
        self.storage.get() as *mut M
    }

    /// Writes the metadata to the slot without reading or dropping the previous value.
    ///
    /// # Safety
    ///
    /// The caller should have exclusive access to the metadata slot's fields.
    pub(super) unsafe fn write_meta<M: AnyFrameMeta>(&self, metadata: M) {
        const { assert!(size_of::<M>() <= FRAME_METADATA_MAX_SIZE) };
        const { assert!(align_of::<M>() <= FRAME_METADATA_MAX_ALIGN) };

        // SAFETY: Caller ensures that the access to the fields are exclusive.
        let vtable_ptr = unsafe { &mut *self.vtable_ptr.get() };
        vtable_ptr.write(core::ptr::metadata(&metadata as &dyn AnyFrameMeta));

        let ptr = self.storage.get();
        // SAFETY:
        // 1. `ptr` points to the metadata storage.
        // 2. The size and the alignment of the metadata storage is large enough to hold `M`
        //    (guaranteed by the const assertions above).
        // 3. We have exclusive access to the metadata storage (guaranteed by the caller).
        unsafe { ptr.cast::<M>().write(metadata) };
    }

    /// Drops the metadata and deallocates the frame.
    ///
    /// # Safety
    ///
    /// The caller should ensure that:
    ///  - the reference count is `0` (so we are the sole owner of the frame);
    ///  - the metadata is initialized;
    pub(super) unsafe fn drop_last_in_place(&self) {
        // This should be guaranteed as a safety requirement.
        debug_assert_eq!(self.ref_count.load(Ordering::Relaxed), 0);

        // SAFETY: The caller ensures safety.
        unsafe { self.drop_meta_in_place() };

        // `Release` pairs with the `Acquire` in `Frame::from_unused` and ensures
        // `drop_meta_in_place` won't be reordered after this memory store.
        self.ref_count.store(REF_COUNT_UNUSED, Ordering::Release);
    }

    /// Drops the metadata of a slot in place.
    ///
    /// After this operation, the metadata becomes uninitialized. Any access to the
    /// metadata is undefined behavior unless it is re-initialized by [`Self::write_meta`].
    ///
    /// # Safety
    ///
    /// The caller should ensure that:
    ///  - the reference count is `0` (so we are the sole owner of the frame);
    ///  - the metadata is initialized;
    pub(super) unsafe fn drop_meta_in_place(&self) {
        let paddr = self.frame_paddr();

        // SAFETY: We have exclusive access to the frame metadata.
        let vtable_ptr = unsafe { &mut *self.vtable_ptr.get() };
        // SAFETY: The frame metadata is initialized and valid.
        let vtable_ptr = unsafe { vtable_ptr.assume_init_read() };

        let meta_ptr: *mut dyn AnyFrameMeta =
            core::ptr::from_raw_parts_mut(self.storage.get(), vtable_ptr);

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
    }
}

/// The metadata of frames that holds metadata of frames.
#[derive(Debug, Default)]
pub struct MetaPageMeta {}

impl_frame_meta_for!(MetaPageMeta);

/// Initializes the metadata of all physical frames.
///
/// The function returns a list of `Frame`s containing the metadata.
///
/// # Safety
///
/// This function should be called only once and only on the BSP,
/// before any APs are started.
pub(crate) unsafe fn init() -> Segment<MetaPageMeta> {
    let max_paddr = {
        let regions = &crate::boot::EARLY_INFO.get().unwrap().memory_regions;
        regions
            .iter()
            .filter(|r| r.typ() == MemoryRegionType::Usable)
            .map(|r| r.base() + r.len())
            .max()
            .unwrap()
    };

    info!(
        "Initializing frame metadata for physical memory up to {:x}",
        max_paddr
    );

    // In RISC-V, the boot page table has mapped the 512GB memory,
    // so we don't need to add temporary linear mapping.
    // In LoongArch, the DWM0 has mapped the whole memory,
    // so we don't need to add temporary linear mapping.
    #[cfg(target_arch = "x86_64")]
    add_temp_linear_mapping(max_paddr);

    let tot_nr_frames = max_paddr / page_size::<PagingConsts>(1);
    let (nr_meta_pages, meta_pages) = alloc_meta_frames(tot_nr_frames);

    // Map the metadata frames.
    boot_pt::with_borrow(|boot_pt| {
        for i in 0..nr_meta_pages {
            let frame_paddr = meta_pages + i * PAGE_SIZE;
            let vaddr = mapping::frame_to_meta::<PagingConsts>(0) + i * PAGE_SIZE;
            let prop = PageProperty {
                has_map: true,
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
    super::MAX_PADDR.store(max_paddr, Ordering::Relaxed);

    let meta_page_range = meta_pages..meta_pages + nr_meta_pages * PAGE_SIZE;

    let (range_1, range_2) = allocator::EARLY_ALLOCATOR
        .lock()
        .as_ref()
        .unwrap()
        .allocated_regions();
    for r in range_difference(&range_1, &meta_page_range) {
        let early_seg = Segment::from_unused(r, |_| EarlyAllocatedFrameMeta).unwrap();
        let _ = ManuallyDrop::new(early_seg);
    }
    for r in range_difference(&range_2, &meta_page_range) {
        let early_seg = Segment::from_unused(r, |_| EarlyAllocatedFrameMeta).unwrap();
        let _ = ManuallyDrop::new(early_seg);
    }

    mark_unusable_ranges();

    Segment::from_unused(meta_page_range, |_| MetaPageMeta {}).unwrap()
}

/// Returns whether the global frame allocator is initialized.
pub(in crate::mm) fn is_initialized() -> bool {
    // `init` sets it with relaxed ordering somewhere in the middle. But due
    // to the safety requirement of the `init` function, we can assume that
    // there is no race conditions.
    super::MAX_PADDR.load(Ordering::Relaxed) != 0
}

fn alloc_meta_frames(tot_nr_frames: usize) -> (usize, Paddr) {
    let nr_meta_pages = tot_nr_frames
        .checked_mul(size_of::<MetaSlot>())
        .unwrap()
        .div_ceil(PAGE_SIZE);
    let start_paddr = allocator::early_alloc(
        Layout::from_size_align(nr_meta_pages * PAGE_SIZE, PAGE_SIZE).unwrap(),
    )
    .unwrap();

    let slots = paddr_to_vaddr(start_paddr) as *mut MetaSlot;

    // Initialize the metadata slots.
    for i in 0..tot_nr_frames {
        // SAFETY: The memory is successfully allocated with `tot_nr_frames`
        // slots so the index must be within the range.
        let slot = unsafe { slots.add(i) };
        // SAFETY: The memory is just allocated so we have exclusive access and
        // it's valid for writing.
        unsafe {
            slot.write(MetaSlot {
                storage: UnsafeCell::new([0; FRAME_METADATA_MAX_SIZE]),
                ref_count: AtomicU64::new(REF_COUNT_UNUSED),
                vtable_ptr: UnsafeCell::new(MaybeUninit::uninit()),
                in_list: AtomicU64::new(0),
            })
        };
    }

    (nr_meta_pages, start_paddr)
}

/// Unusable memory metadata. Cannot be used for any purposes.
#[derive(Debug)]
pub struct UnusableMemoryMeta;
impl_frame_meta_for!(UnusableMemoryMeta);

/// Reserved memory metadata. Maybe later used as I/O memory.
#[derive(Debug)]
pub struct ReservedMemoryMeta;
impl_frame_meta_for!(ReservedMemoryMeta);

/// The metadata of physical pages that contains the kernel itself.
#[derive(Debug, Default)]
pub struct KernelMeta;
impl_frame_meta_for!(KernelMeta);

macro_rules! mark_ranges {
    ($region: expr, $typ: expr) => {{
        debug_assert!($region.base() % PAGE_SIZE == 0);
        debug_assert!($region.len() % PAGE_SIZE == 0);

        let seg = Segment::from_unused($region.base()..$region.end(), |_| $typ).unwrap();
        let _ = ManuallyDrop::new(seg);
    }};
}

fn mark_unusable_ranges() {
    let regions = &crate::boot::EARLY_INFO.get().unwrap().memory_regions;

    for region in regions
        .iter()
        .rev()
        .skip_while(|r| r.typ() != MemoryRegionType::Usable)
    {
        match region.typ() {
            MemoryRegionType::BadMemory => mark_ranges!(region, UnusableMemoryMeta),
            MemoryRegionType::Unknown => mark_ranges!(region, ReservedMemoryMeta),
            MemoryRegionType::NonVolatileSleep => mark_ranges!(region, UnusableMemoryMeta),
            MemoryRegionType::Reserved => mark_ranges!(region, ReservedMemoryMeta),
            MemoryRegionType::Kernel => mark_ranges!(region, KernelMeta),
            MemoryRegionType::Module => mark_ranges!(region, UnusableMemoryMeta),
            MemoryRegionType::Framebuffer => mark_ranges!(region, ReservedMemoryMeta),
            MemoryRegionType::Reclaimable => mark_ranges!(region, UnusableMemoryMeta),
            MemoryRegionType::Usable => {} // By default it is initialized as usable.
        }
    }
}

/// Adds a temporary linear mapping for the metadata frames.
///
/// We only assume boot page table to contain 4G linear mapping. Thus if the
/// physical memory is huge we end up depleted of linear virtual memory for
/// initializing metadata.
#[cfg(target_arch = "x86_64")]
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
        has_map: true,
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
