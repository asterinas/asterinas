// SPDX-License-Identifier: MPL-2.0

//! Guest physical memory space.

use core::{mem::ManuallyDrop, ops::Range};

use super::Gpaddr;
use crate::{
    arch::vm::{
        ept::{EptItem, EptPtConfig},
        vmx::{VmxGuard, invept},
    },
    error::Error,
    mm::{
        AnyUFrameMeta, Frame, PageFlags, PageProperty, UFrame,
        frame::FrameRef,
        page_table::{self, PageTable, PageTableFrag},
    },
    prelude::*,
    smp::PendingIpis,
    sync::RcuDrop,
    task::atomic_mode::AsAtomicModeGuard,
};

/// Guest physical memory space of a VM.
///
/// This type owns the page table that maps guest physical addresses to
/// host physical frames. Its cursors manage 4-KiB mappings within a 48-bit
/// guest physical address space. Each mapping must permit reads; write and
/// execute permissions can be added independently.
pub struct GuestPhysMemSpace {
    pt: ManuallyDrop<PageTable<EptPtConfig>>,
    vmx_guard: VmxGuard,
}

impl GuestPhysMemSpace {
    /// Creates a new guest physical memory space.
    ///
    /// # Errors
    ///
    /// Returns an error if the CPU does not support second-stage address
    /// translation or the required invalidation operations.
    pub fn new() -> Result<Self> {
        if !invept::is_ept_supported() {
            return Err(Error::NotEnoughResources);
        }
        let vmx_guard = VmxGuard::acquire_vmx()?;
        Ok(Self {
            pt: ManuallyDrop::new(PageTable::<EptPtConfig>::empty()),
            vmx_guard,
        })
    }

    /// Gets an immutable cursor over a guest physical address range.
    ///
    /// The cursor behaves like a lock guard, exclusively owning a sub-tree of
    /// the page table, preventing others from creating a cursor in it. So be
    /// sure to drop the cursor as soon as possible.
    ///
    /// The creation of the cursor may block if another cursor having an
    /// overlapping range is alive.
    pub fn cursor<'a, G: AsAtomicModeGuard>(
        &'a self,
        guard: &'a G,
        gpa: &Range<Gpaddr>,
    ) -> Result<Cursor<'a>> {
        Ok(Cursor(self.pt.cursor(guard, gpa)?))
    }

    /// Gets a mutable cursor over a guest physical address range.
    ///
    /// The same as [`Self::cursor`], the cursor behaves like a lock guard,
    /// exclusively owning a sub-tree of the page table, preventing others
    /// from creating a cursor in it. So be sure to drop the cursor as soon as
    /// possible.
    ///
    /// The creation of the cursor may block if another cursor having an
    /// overlapping range is alive.
    pub fn cursor_mut<'a, G: AsAtomicModeGuard>(
        &'a self,
        guard: &'a G,
        gpa: &Range<Gpaddr>,
    ) -> Result<CursorMut<'a>> {
        Ok(CursorMut {
            pt_cursor: self.pt.cursor_mut(guard, gpa)?,
            vmx_guard: &self.vmx_guard,
            pending_ipis: PendingIpis::new_empty(),
        })
    }

    /// Returns the EPT pointer value for this guest memory space.
    ///
    /// Guest execution must keep this address space borrowed while its EPTP
    /// is in use. Accessed/dirty tracking and supervisor shadow stacks are
    /// disabled in this EPTP.
    pub(crate) fn eptp(&self) -> u64 {
        const EPT_MEM_TYPE_WB: u64 = 6;
        const EPT_PAGE_WALK_LENGTH_4_LEVELS: u64 = 3 << 3;

        self.pt.root_paddr() as u64 | EPT_MEM_TYPE_WB | EPT_PAGE_WALK_LENGTH_4_LEVELS
    }
}

impl Drop for GuestPhysMemSpace {
    fn drop(&mut self) {
        // SAFETY: `self.pt` is initialized in `new` and is taken only here.
        // It is never accessed again.
        let pt = unsafe { ManuallyDrop::take(&mut self.pt) };
        let root = ManuallyDrop::new(RcuDrop::new(pt.into_inner().into()));
        invept::invalidate(&self.vmx_guard, core::slice::from_ref(&root));
        drop(ManuallyDrop::into_inner(root));
    }
}

/// A borrowed backing frame and its page properties.
pub type QueriedItem<'a> = (FrameRef<'a, dyn AnyUFrameMeta>, PageProperty);

/// The cursor for querying over the guest physical memory space without modifying it.
///
/// It exclusively owns a sub-tree of the page table, preventing others from
/// reading or modifying the same sub-tree. Two read-only cursors can not be
/// created from the same guest physical address range either.
pub struct Cursor<'a>(page_table::Cursor<'a, EptPtConfig>);

impl Cursor<'_> {
    /// Queries the mapping at the current guest physical address.
    ///
    /// If the cursor is pointing to a valid guest physical address that is
    /// locked, it will return the guest physical address range, the borrowed
    /// backing frame, and its page properties.
    pub fn query(&mut self) -> Result<(Range<Gpaddr>, Option<QueriedItem<'_>>)> {
        Ok(self.0.query()?)
    }

    /// Moves the cursor forward to the next mapped guest physical address.
    ///
    /// If there is a mapped guest physical address following the current
    /// address within next `len` bytes, it will return that mapped address. In
    /// this case, the cursor will stop at the mapped address.
    ///
    /// Otherwise, it will return `None`. And the cursor may stop at any
    /// address after `len` bytes.
    ///
    /// # Panics
    ///
    /// Panics if the length is longer than the remaining range of the cursor.
    pub fn find_next(&mut self, len: usize) -> Option<Gpaddr> {
        self.0.find_next(len)
    }

    /// Jumps to the guest physical address.
    pub fn jump(&mut self, gpa: Gpaddr) -> Result<()> {
        self.0.jump(gpa)?;
        Ok(())
    }

    /// Gets the guest physical address of the current slot.
    pub fn gpa(&self) -> Gpaddr {
        self.0.virt_addr()
    }
}

/// The cursor for modifying the mappings in guest physical memory space.
///
/// It exclusively owns a sub-tree of the page table, preventing others from
/// reading or modifying the same sub-tree.
pub struct CursorMut<'a> {
    pt_cursor: page_table::CursorMut<'a, EptPtConfig>,
    vmx_guard: &'a VmxGuard,
    pending_ipis: PendingIpis,
}

impl<'a> CursorMut<'a> {
    /// Queries the mapping at the current guest physical address.
    ///
    /// This is the same as [`Cursor::query`].
    ///
    /// If the cursor is pointing to a valid guest physical address that is
    /// locked, it will return the guest physical address range, the borrowed
    /// backing frame, and its page properties.
    pub fn query(&mut self) -> Result<(Range<Gpaddr>, Option<QueriedItem<'_>>)> {
        Ok(self.pt_cursor.query()?)
    }

    /// Moves the cursor forward to the next mapped guest physical address.
    ///
    /// This is the same as [`Cursor::find_next`].
    pub fn find_next(&mut self, len: usize) -> Option<Gpaddr> {
        self.pt_cursor.find_next(len)
    }

    /// Jumps to the guest physical address.
    ///
    /// This is the same as [`Cursor::jump`].
    pub fn jump(&mut self, gpa: Gpaddr) -> Result<()> {
        self.pt_cursor.jump(gpa)?;
        Ok(())
    }

    /// Gets the guest physical address of the current slot.
    pub fn gpa(&self) -> Gpaddr {
        self.pt_cursor.virt_addr()
    }

    /// Maps a frame into the current slot.
    ///
    /// This method will bring the cursor to the next slot after the modification.
    ///
    /// # Panics
    ///
    /// Panics if the slot is already mapped, is outside the cursor's range.
    pub fn map(&mut self, frame: UFrame, prop: PageProperty) {
        let item: EptItem = (frame, prop);

        // SAFETY: It is safe to map untyped memory into guest physical memory.
        unsafe { self.pt_cursor.map(item) };
    }

    /// Updates the permissions of the next mapped page within `len` bytes.
    ///
    /// Returns its guest physical range and advances past that page, or returns
    /// `None` if the range has no mapping. Cached translations are invalidated
    /// on the current CPU before returning, and asynchronously on remote CPUs.
    ///
    /// # Panics
    ///
    /// Panics if `len` is unaligned or exceeds the remaining cursor range.
    pub fn protect_next(
        &mut self,
        len: usize,
        op: &mut impl FnMut(&mut PageFlags),
    ) -> Option<Range<Gpaddr>> {
        // SAFETY: Only guest permissions are changed. The privileged flags
        // and backing-frame ownership remain intact.
        let range = unsafe {
            self.pt_cursor
                .protect_next(len, &mut |prop| op(&mut prop.flags))
        }?;
        self.pending_ipis
            .extend(&invept::invalidate(self.vmx_guard, &[]));
        Some(range)
    }

    /// Unmaps mappings from the current guest physical address.
    ///
    /// The method removes mapped pages or page-table subtrees up to `len`
    /// bytes from the current guest physical address, dispatches EPT invalidation,
    /// and returns the number of unmapped frames or page-table frames.
    ///
    /// # Panics
    ///
    /// Panics if `len` is longer than the remaining range of the cursor or is
    /// not page-aligned.
    pub fn unmap(&mut self, len: usize) -> usize {
        let end_gpa = self.gpa() + len;
        let mut num_unmapped: usize = 0;
        // Retain removed frames even if unwinding happens before dispatch.
        let mut frames = ManuallyDrop::new(Vec::new());
        loop {
            frames.reserve(1);
            // SAFETY:
            // 1. The EPT maps only physical frames of type `UFrame`, so unmapping them
            //    does not compromise kernel memory safety.
            // 2. Removed frames are retained below, then cloned into each CPU's
            //    invalidation queue before their references here are released.
            let Some(frag) = (unsafe { self.pt_cursor.take_next(end_gpa - self.gpa()) }) else {
                break; // No more mappings in the range.
            };

            match frag {
                PageTableFrag::Mapped { item, .. } => {
                    // SAFETY: The pre-reserved slot retains the frame until
                    // invalidation, and `RcuDrop` preserves its RCU lifetime.
                    let ((frame, _), panic_guard) = unsafe { RcuDrop::into_inner(item) };
                    frames.push(Frame::rcu_from_unsized(RcuDrop::new(frame)));
                    panic_guard.forget();
                    num_unmapped += 1;
                }
                PageTableFrag::StrayPageTable { pt, num_frames, .. } => {
                    frames.push(Frame::rcu_from_unsized(pt));
                    num_unmapped += num_frames;
                }
            }
        }

        if !frames.is_empty() {
            self.pending_ipis
                .extend(&invept::invalidate(self.vmx_guard, &frames));
        }
        drop(ManuallyDrop::into_inner(frames));
        num_unmapped
    }

    /// Waits for this cursor's previous EPT invalidations to complete on all CPUs.
    ///
    /// This synchronizes invalidations issued by [`Self::unmap`] and
    /// [`Self::protect_next`]. Dropping the cursor does not wait for completion;
    /// removed frames are retained until invalidation completes regardless.
    ///
    /// # Panics
    ///
    /// Panics if local IRQs are disabled.
    pub fn sync_tlb_flush(&mut self) {
        self.pending_ipis.wait();
        self.pending_ipis = PendingIpis::new_empty();
    }
}

#[cfg(ktest)]
mod test {
    use super::*;
    use crate::{
        mm::{CachePolicy, FrameAllocOptions, PAGE_SIZE},
        task::disable_preempt,
    };

    #[ktest]
    fn guest_mapping_lifecycle() {
        let space = match GuestPhysMemSpace::new() {
            Ok(space) => space,
            Err(Error::NotEnoughResources | Error::AccessDenied) => {
                crate::early_print!(" [skipped: VMX/EPT invalidation unavailable]");
                return;
            }
            Err(err) => panic!("failed to create guest physical memory: {:?}", err),
        };

        let first = FrameAllocOptions::new().alloc_frame().unwrap();
        let second = FrameAllocOptions::new().alloc_frame().unwrap();
        let first_paddr = first.paddr();
        let second_paddr = second.paddr();
        let prop = PageProperty::new_user(PageFlags::RWX, CachePolicy::Writeback);
        let range = PAGE_SIZE..4 * PAGE_SIZE;

        let guard = disable_preempt();
        let mut cursor = space.cursor_mut(&guard, &range).unwrap();

        // Test `map`.
        cursor.map(first.into(), prop);
        cursor.map(second.into(), prop);
        assert_eq!(cursor.gpa(), 3 * PAGE_SIZE);

        // Test `query`.
        cursor.jump(2 * PAGE_SIZE).unwrap();
        let (queried_range, item) = cursor.query().unwrap();
        assert_eq!(queried_range, 2 * PAGE_SIZE..3 * PAGE_SIZE);
        assert_eq!(
            item.map(|(frame, prop)| (frame.paddr(), prop)),
            Some((second_paddr, prop))
        );
        cursor.jump(PAGE_SIZE).unwrap();
        let (queried_range, item) = cursor.query().unwrap();
        assert_eq!(queried_range, PAGE_SIZE..2 * PAGE_SIZE);
        assert_eq!(
            item.map(|(frame, prop)| (frame.paddr(), prop)),
            Some((first_paddr, prop))
        );

        // Test `protect`.
        assert_eq!(
            cursor.protect_next(PAGE_SIZE, &mut |flags| *flags = PageFlags::RX),
            Some(PAGE_SIZE..2 * PAGE_SIZE)
        );
        cursor.jump(PAGE_SIZE).unwrap();
        assert_eq!(cursor.query().unwrap().1.unwrap().1.flags, PageFlags::RX);

        // Test `unmap`.
        assert_eq!(cursor.unmap(2 * PAGE_SIZE), 2);
        cursor.jump(PAGE_SIZE).unwrap();
        assert!(cursor.find_next(3 * PAGE_SIZE).is_none());
    }
}
