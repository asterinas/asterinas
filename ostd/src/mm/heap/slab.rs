// SPDX-License-Identifier: MPL-2.0

//! Slabs for implementing the slab allocator.

use core::{alloc::AllocError, marker::PhantomPinned, pin::Pin, ptr::NonNull};

use super::{slot::HeapSlot, slot_list::SlabSlotList};
use crate::mm::{
    FrameAllocOptions, HasPaddr, HasSize, PAGE_SIZE, UniqueFrame,
    frame::{linked_list::Link, meta::AnyFrameMeta},
    paddr_to_vaddr,
};

/// A slab.
///
/// The slot size is the maximum size of objects that can be allocated from the
/// slab. The slab is densely divided into slots of this size.
///
/// The `SLOT_SIZE` is the size of the slots in bytes. The size of the slots
/// cannot be smaller than the size of [`usize`]. It must be smaller than or
/// equal to [`PAGE_SIZE`].
///
/// A slab should have the size of one basic page. This restriction may be
/// lifted in the future.
pub type Slab<const SLOT_SIZE: usize> = UniqueFrame<Link<SlabMeta<SLOT_SIZE>>>;

/// Frame metadata of a slab.
///
/// Each slab is backed by a [`UniqueFrame`].
///
/// # Address sensitivity and pinning
///
/// A slab's metadata contains two internal parts:
///
/// 1. A free list that contains pointers into the slab's physical page;
/// 2. A counter that determines whether that page may be released.
///
/// Moving this metadata to another frame would separate
/// those pointers and the counter from the page they describe,
/// potentially causing use-after-free.
/// The metadata must therefore remain in its frame's metadata slot
/// once the free list is initialized.
/// In short, `SlabMeta<SLOT_SIZE>` is **address-sensitive**.
///
/// To express the address-sensitive nature,
/// we make this type `!Unpin`.
/// Mutation operations on `SlabMeta` require using pinned references.
/// For example, allocating slab slots is done through `slab.pin_meta().project().alloc()`.
///
/// Pinning alone does not prevent replacement:
/// [`Pin::set`] drops the old metadata in place
/// without calling `on_drop`, bypassing its allocation check.
/// To prevent this, safe callers must not be able to obtain owned replacement values of `SlabMeta`.
/// So we keep the constructor private and
/// do not provide `Default`, `Clone`, or
/// other APIs that would allow callers to construct or extract such values.
#[derive(Debug)]
pub struct SlabMeta<const SLOT_SIZE: usize> {
    /// The list of free slots inside the slab.
    ///
    /// Slots not inside the slab should not be in the list.
    free_list: SlabSlotList<SLOT_SIZE>,

    /// The number of allocated slots in the slab.
    ///
    /// Even if a slot is free, as long as it does not stay in the
    /// [`Self::free_list`], it is considered allocated.
    nr_allocated: u16,

    _pin: PhantomPinned,
}

unsafe impl<const SLOT_SIZE: usize> Send for SlabMeta<SLOT_SIZE> {}
unsafe impl<const SLOT_SIZE: usize> Sync for SlabMeta<SLOT_SIZE> {}

// SAFETY: The callback does not read the frame. The construction and access
// restrictions documented on `SlabMeta` preserve its address dependencies.
unsafe impl<const SLOT_SIZE: usize> AnyFrameMeta for SlabMeta<SLOT_SIZE> {
    unsafe fn on_drop(
        self: Pin<&mut Self>,
        _reader: &mut crate::mm::VmReader<crate::mm::Infallible>,
    ) {
        if self.nr_allocated != 0 {
            panic!("{} slots allocated when dropping a slab", self.nr_allocated);
        }
    }

    fn is_untyped(&self) -> bool {
        false
    }
}

impl<const SLOT_SIZE: usize> SlabMeta<SLOT_SIZE> {
    /// Gets the capacity of the slab (regardless of the number of allocated slots).
    pub const fn capacity(&self) -> u16 {
        (PAGE_SIZE / SLOT_SIZE) as u16
    }

    /// Gets the number of allocated slots.
    pub fn nr_allocated(&self) -> u16 {
        self.nr_allocated
    }

    /// Allocates a slot from the slab.
    pub fn alloc(self: Pin<&mut Self>) -> Result<HeapSlot, AllocError> {
        // SAFETY: Only the free list and count change; the metadata stays pinned
        // in the slot associated with the page that owns these slots.
        let this = unsafe { self.get_unchecked_mut() };
        let Some(allocated) = this.free_list.pop() else {
            crate::error!("Allocating a slot from a full slab");
            return Err(AllocError);
        };
        this.nr_allocated += 1;
        Ok(allocated)
    }

    /// Creates empty metadata for [`Slab::new`].
    const fn new() -> Self {
        Self {
            free_list: SlabSlotList::new(),
            nr_allocated: 0,
            _pin: PhantomPinned,
        }
    }
}

impl<const SLOT_SIZE: usize> Slab<SLOT_SIZE> {
    /// Allocates a new slab of the given size.
    ///
    /// If the size is less than `SLOT_SIZE` or [`PAGE_SIZE`], the size will be
    /// the maximum of the two.
    pub fn new() -> crate::prelude::Result<Self> {
        const { assert!(SLOT_SIZE <= PAGE_SIZE) };
        // To ensure we can store a pointer in each slot.
        const { assert!(SLOT_SIZE >= size_of::<usize>()) };
        // To ensure `nr_allocated` can be stored in a `u16`.
        const { assert!(PAGE_SIZE / SLOT_SIZE <= u16::MAX as usize) };

        let mut slab: Slab<SLOT_SIZE> = FrameAllocOptions::new()
            .zeroed(false)
            .alloc_frame_with(Link::new(SlabMeta::<SLOT_SIZE>::new()))?
            .try_into()
            .unwrap();

        let head_paddr = slab.paddr();
        let head_vaddr = paddr_to_vaddr(head_paddr);

        // SAFETY: The metadata is installed in its final slot. Only its free
        // list is initialized, with slots belonging to this frame; it never moves.
        let meta = unsafe { slab.pin_meta().project().get_unchecked_mut() };
        // Push each slot to the free list.
        for slot_offset in (0..PAGE_SIZE).step_by(SLOT_SIZE) {
            // SAFETY: The slot is within the slab so it can't be NULL.
            let slot_ptr = unsafe { NonNull::new_unchecked((head_vaddr + slot_offset) as *mut u8) };
            // SAFETY: The slot is newly allocated in the slab.
            meta.free_list
                .push(unsafe { HeapSlot::new(slot_ptr, super::SlotInfo::SlabSlot(SLOT_SIZE)) });
        }

        Ok(slab)
    }

    /// Deallocates a slot to the slab.
    ///
    /// If the slot does not belong to the slab it returns [`AllocError`].
    pub fn dealloc(&mut self, slot: HeapSlot) -> Result<(), AllocError> {
        if !(self.paddr()..self.paddr() + self.size()).contains(&slot.paddr()) {
            crate::error!("Deallocating a slot to a slab that does not own the slot");
            return Err(AllocError);
        }
        debug_assert_eq!(slot.size(), SLOT_SIZE);
        // SAFETY: Only the free list and count change; the metadata stays pinned.
        // The ownership check above preserves its association with this page.
        let meta = unsafe { self.pin_meta().project().get_unchecked_mut() };
        meta.free_list.push(slot);
        meta.nr_allocated -= 1;

        Ok(())
    }
}

#[cfg(ktest)]
mod test {
    use super::*;
    use crate::{
        mm::{Frame, frame::linked_list::LinkedList},
        panic,
        prelude::*,
    };

    #[ktest]
    fn slab_exhaustion_and_refill() {
        let mut slab = Slab::<64>::new().unwrap();
        let slab_range = slab.paddr()..slab.paddr() + PAGE_SIZE;
        let capacity = slab.meta().capacity();

        // Fill and empty the slab twice to exercise reuse.
        for _ in 0..2 {
            let mut slots: Vec<HeapSlot> = Vec::new();
            for _ in 0..capacity {
                let slot = slab.pin_meta().project().alloc().unwrap();
                assert!(slab_range.contains(&slot.paddr()));
                assert_eq!(slot.size(), 64);
                assert!(slot.paddr().is_multiple_of(64));
                assert!(slots.iter().all(|other| other.paddr() != slot.paddr()));
                slots.push(slot);
            }
            assert_eq!(slab.meta().nr_allocated(), capacity);
            assert!(slab.pin_meta().project().alloc().is_err());

            for slot in slots {
                slab.dealloc(slot).unwrap();
            }
            assert_eq!(slab.meta().nr_allocated(), 0);
        }
    }

    #[ktest]
    fn slab_slots_stay_with_their_frames() {
        // Regression for https://github.com/asterinas/asterinas/issues/4252.
        let first = Slab::<64>::new().unwrap();
        let first_range = first.paddr()..first.paddr() + PAGE_SIZE;
        let mut list = LinkedList::new();
        list.push_back(first);
        list.push_back(Slab::<64>::new().unwrap());

        let mut cursor = list.cursor_front_mut();
        let first_slot = cursor.pin_current_meta().unwrap().alloc().unwrap();
        assert!(first_range.contains(&first_slot.paddr()));
        let mut first = cursor.take_current().unwrap();
        drop(list);

        // Releasing the other page must not invalidate this slab's free list.
        let another_slot = first.pin_meta().project().alloc().unwrap();
        assert!(first_range.contains(&another_slot.paddr()));
        assert_ne!(another_slot.paddr(), first_slot.paddr());
        first.dealloc(first_slot).unwrap();
        first.dealloc(another_slot).unwrap();
    }

    #[ktest]
    fn slab_rejects_foreign_slots() {
        let mut owner = Slab::<64>::new().unwrap();
        let mut other = Slab::<64>::new().unwrap();
        let slot = owner.pin_meta().project().alloc().unwrap();
        let ptr = NonNull::new(slot.as_ptr()).unwrap();
        let info = slot.info();

        assert!(other.dealloc(slot).is_err());
        assert_eq!(other.meta().nr_allocated(), 0);
        assert_eq!(owner.meta().nr_allocated(), 1);
        // SAFETY: The failed deallocation consumed the handle but left this free
        // slot allocated in `owner`. Restore its sole handle to clean up.
        owner.dealloc(unsafe { HeapSlot::new(ptr, info) }).unwrap();
    }

    #[ktest]
    fn outstanding_slots_prevent_slab_teardown() {
        fn assert_retained(teardown_fn: impl FnOnce(Slab<64>)) {
            let mut slab = Slab::new().unwrap();
            let paddr = slab.paddr();
            let _slot = slab.pin_meta().project().alloc().unwrap();
            assert!(panic::catch_unwind(|| teardown_fn(slab)).is_err());
            assert!(UniqueFrame::from_unused(paddr, ()).is_err());
            assert!(Frame::<dyn AnyFrameMeta>::from_in_use(paddr).is_err());
            // These pages and slots must leak: teardown has panicked, so they
            // cannot safely be accessed, released, or reused.
        }

        assert_retained(drop);
        assert_retained(|slab| drop(Frame::from(slab)));
        assert_retained(|slab| drop(slab.repurpose(())));
        assert_retained(Slab::reset_as_unused);
        assert_retained(|slab| {
            let mut list = LinkedList::new();
            list.push_back(slab);
            drop(list);
        });
    }
}
