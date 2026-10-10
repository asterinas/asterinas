// SPDX-License-Identifier: MPL-2.0

//! Guest physical memory space.

use core::{mem::ManuallyDrop, ops::Range};

use super::Gpaddr;
use crate::{
    arch::vm::{
        ept::{EptItem, EptPtConfig},
        vmx::{VmxGuard, invept},
    },
    mm::{
        AnyUFrameMeta, Frame, PageFlags, PageProperty, UFrame,
        frame::FrameRef,
        page_table::{self, PageTable, PageTableFrag, PteStateRef},
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
///
/// This address space must be dropped outside [atomic mode](crate::task::atomic_mode).
pub struct GuestPhysMemSpace {
    pt: PageTable<EptPtConfig>,
    vmx_guard: VmxGuard,
}

impl GuestPhysMemSpace {
    /// Creates a new guest physical memory space.
    ///
    /// # Errors
    ///
    /// Returns an error if the CPU does not support second-stage address
    /// translation or the required invalidation operations.
    ///
    /// # Panics
    ///
    /// Panics if called in [atomic mode](crate::task::atomic_mode).
    pub fn new() -> Result<Self> {
        let vmx_guard = VmxGuard::acquire_vmx()?;
        let pt = PageTable::<EptPtConfig>::empty();

        // A reused EPT root-table address may still have cached translations.
        invept::invalidate(&vmx_guard, &[]).wait();

        Ok(Self { pt, vmx_guard })
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
    /// The caller must keep this address space borrowed while its EPTP is in use.
    #[expect(dead_code)]
    pub(crate) fn eptp(&self) -> u64 {
        const EPT_MEM_TYPE_WB: u64 = 6;
        const EPT_PAGE_WALK_LENGTH_4_LEVELS: u64 = 3 << 3;

        self.pt.root_paddr() as u64 | EPT_MEM_TYPE_WB | EPT_PAGE_WALK_LENGTH_4_LEVELS
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
    /// locked, it will return the borrowed backing frame and its page properties.
    pub fn query(&mut self) -> Option<QueriedItem<'_>> {
        while self.0.push_level_if_exists().is_some() {}
        match self.0.query() {
            PteStateRef::Mapped(item) => Some(item),
            PteStateRef::Absent => None,
            PteStateRef::PageTable(_) => unreachable!(),
        }
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
    /// Panics if:
    ///  - the length is longer than the remaining range of the cursor;
    ///  - the length is not page-aligned.
    pub fn find_next(&mut self, len: usize) -> Option<Gpaddr> {
        self.0.find_next(self.gpa().checked_add(len).unwrap())
    }

    /// Jumps to the guest physical address.
    ///
    /// If the target address is out of the range, this method will return `Err`.
    ///
    /// # Panics
    ///
    /// This method panics if the address has bad alignment.
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
    /// locked, it will return the borrowed backing frame and its page properties.
    pub fn query(&mut self) -> Option<QueriedItem<'_>> {
        while self.pt_cursor.push_level_if_exists().is_some() {}
        match self.pt_cursor.query() {
            PteStateRef::Mapped(item) => Some(item),
            PteStateRef::Absent => None,
            PteStateRef::PageTable(_) => unreachable!(),
        }
    }

    /// Moves the cursor forward to the next mapped guest physical address.
    ///
    /// This is the same as [`Cursor::find_next`].
    pub fn find_next(&mut self, len: usize) -> Option<Gpaddr> {
        let end = self.gpa().checked_add(len).unwrap();
        self.pt_cursor.find_next(end)
    }

    /// Jumps to the guest physical address.
    ///
    /// This is the same as [`Cursor::jump`].
    ///
    /// # Panics
    ///
    /// This method panics if the address has bad alignment.
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
    /// The cursor remains at the mapped address.
    ///
    /// # Panics
    ///
    /// Panics if:
    ///  - the current guest physical address is already mapped;
    ///  - the current guest physical address is outside the cursor's range.
    pub fn map(&mut self, frame: UFrame, prop: PageProperty) {
        self.pt_cursor.adjust_level(1);
        let item: EptItem = (frame, prop);

        // SAFETY: It is safe to map untyped memory into guest physical memory.
        unsafe { self.pt_cursor.map(item) };
    }

    /// Applies the operation to the mapping at the current address.
    ///
    /// The cursor remains at the protected address. If the address is not
    /// mapped, this method does nothing.
    ///
    /// Cached translations are invalidated on the current CPU before returning,
    /// and asynchronously on remote CPUs. Use [`Self::sync_tlb_flush`] to
    /// wait for remote invalidations to complete.
    pub fn protect(&mut self, op: &mut impl FnMut(&mut PageFlags)) {
        if self.query().is_none() {
            return;
        }
        // SAFETY: It is safe to set `PageFlags` of guest physical memory.
        unsafe { self.pt_cursor.protect(&mut |prop| op(&mut prop.flags)) };
        self.pending_ipis
            .extend(&invept::invalidate(self.vmx_guard, &[]));
    }

    /// Clears the mapping starting from the current slot,
    /// and returns the number of unmapped pages.
    ///
    /// This method brings the cursor forward by at least `len` bytes in the
    /// guest physical address space, but not past the end of the cursor's range.
    ///
    /// Already-absent mappings encountered by the cursor will be skipped.
    /// It is valid to unmap a range that is not mapped.
    ///
    /// This method issues and dispatches EPT invalidation for removed mappings.
    /// Cached translations are invalidated on the current CPU before returning,
    /// and asynchronously on remote CPUs. Removed frames are retained until
    /// invalidation completes on all CPUs. Use [`Self::sync_tlb_flush`] to
    /// wait for remote invalidations to complete. Using a large `len` avoids
    /// the overhead of multiple invalidations.
    ///
    /// # Panics
    ///
    /// Panics if:
    ///  - the length is longer than the remaining range of the cursor;
    ///  - the length is not page-aligned.
    pub fn unmap(&mut self, len: usize) -> usize {
        let end_gpa = self.gpa().checked_add(len).unwrap();
        let mut num_unmapped: usize = 0;
        // Retain removed frames even if unwinding happens before dispatch.
        let mut frames = ManuallyDrop::new(Vec::new());
        loop {
            if self
                .pt_cursor
                .find_next_unmappable_subtree(end_gpa)
                .is_none()
            {
                break;
            }
            frames.reserve(1);
            // SAFETY:
            // 1. It is safe to unmap guest physical memory.
            // 2. Removed frames are retained below, then cloned into each CPU's
            //    invalidation queue before their references here are released.
            let frag = unsafe { self.pt_cursor.unmap() }.unwrap();

            match frag {
                PageTableFrag::Mapped { item, .. } => {
                    // SAFETY: The frame will be dropped after the RCU grace
                    // period (see the following `RcuDrop::new`).
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
    /// [`Self::protect`]. Dropping the cursor does not wait for completion;
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
        Error,
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
        assert_eq!(cursor.gpa(), PAGE_SIZE);
        cursor.jump(2 * PAGE_SIZE).unwrap();
        cursor.map(second.into(), prop);
        assert_eq!(cursor.gpa(), 2 * PAGE_SIZE);

        // Test `query`.
        cursor.jump(2 * PAGE_SIZE).unwrap();
        let item = cursor.query();
        assert_eq!(
            item.map(|(frame, prop)| (frame.paddr(), prop)),
            Some((second_paddr, prop))
        );
        cursor.jump(PAGE_SIZE).unwrap();
        let item = cursor.query();
        assert_eq!(
            item.map(|(frame, prop)| (frame.paddr(), prop)),
            Some((first_paddr, prop))
        );

        // Test `protect`.
        cursor.protect(&mut |flags| *flags = PageFlags::RX);
        assert_eq!(cursor.gpa(), PAGE_SIZE);
        cursor.jump(PAGE_SIZE).unwrap();
        assert_eq!(cursor.query().unwrap().1.flags, PageFlags::RX);

        // Test `unmap`.
        assert_eq!(cursor.unmap(2 * PAGE_SIZE), 2);
        cursor.jump(PAGE_SIZE).unwrap();
        assert!(cursor.find_next(3 * PAGE_SIZE).is_none());
    }
}
