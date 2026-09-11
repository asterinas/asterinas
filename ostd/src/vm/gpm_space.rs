// SPDX-License-Identifier: MPL-2.0

//! Guest physical memory space.

use core::{mem::ManuallyDrop, ops::Range};

use super::Gpaddr;
use crate::{
    arch::vm::{
        ept::{EptItem, EptPtConfig},
        vmx::invept::EptGuard,
    },
    mm::{
        PageFlags, PageProperty, UFrame,
        page_table::{self, PageTable, PageTableFrag},
    },
    prelude::*,
    task::atomic_mode::AsAtomicModeGuard,
};

/// Manages the guest physical memory space of a VM.
///
/// This type owns the page table that maps guest physical addresses to
/// host physical frames. Its cursors manage 4-KiB mappings within a 48-bit
/// guest physical address space. Each mapping must permit reads; write and
/// execute permissions can be added independently.
///
/// The address space keeps VMX enabled until its translations are invalidated
/// and its page table is released. It must be created and dropped in task
/// context with IRQs and preemption enabled.
pub struct GuestPhysMemSpace {
    // Keep the page table allocated if invalidation fails, including during unwinding.
    pt: ManuallyDrop<PageTable<EptPtConfig>>,
    ept_guard: EptGuard,
}

impl GuestPhysMemSpace {
    /// Creates a new guest physical memory space.
    ///
    /// # Errors
    /// Returns an error if the CPU does not support second-stage address
    /// translation or the required invalidation operations, or VMX cannot be enabled.
    ///
    /// # Panics
    ///
    /// Panics if called with IRQs disabled. Creation and destruction require
    /// task context with preemption enabled to acquire and release the VMX guard.
    pub fn new() -> Result<Self> {
        let ept_guard = EptGuard::new()?;
        Ok(Self {
            pt: ManuallyDrop::new(PageTable::<EptPtConfig>::empty()),
            ept_guard,
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
    /// overlapping range is alive. Keep IRQs enabled when modifying or
    /// removing mappings, since invalidation waits for remote CPUs.
    pub fn cursor_mut<'a, G: AsAtomicModeGuard>(
        &'a self,
        guard: &'a G,
        gpa: &Range<Gpaddr>,
    ) -> Result<CursorMut<'a>> {
        Ok(CursorMut {
            pt_cursor: self.pt.cursor_mut(guard, gpa)?,
            ept_guard: &self.ept_guard,
        })
    }

    /// Returns the EPT pointer value for this guest memory space.
    ///
    /// Future guest execution must keep this address space borrowed while its
    /// EPTP is in use. Accessed/dirty tracking and supervisor shadow stacks
    /// are disabled in this EPTP.
    #[expect(dead_code)]
    pub(crate) fn eptp(&self) -> u64 {
        const EPT_MEM_TYPE_WB: u64 = 6;
        const EPT_PAGE_WALK_LENGTH_4_LEVELS: u64 = 3 << 3;

        self.pt.root_paddr() as u64 | EPT_MEM_TYPE_WB | EPT_PAGE_WALK_LENGTH_4_LEVELS
    }
}

impl Drop for GuestPhysMemSpace {
    fn drop(&mut self) {
        self.ept_guard
            .invalidate()
            .expect("failed to invalidate EPT translations");
        // SAFETY: No cursor or guest can retain a borrow of this address space
        // during Drop, and all CPUs have discarded its cached translations.
        // This is the only place that drops the manually managed page table.
        unsafe { ManuallyDrop::drop(&mut self.pt) };
    }
}

fn flush_and_drop(frags: Vec<ManuallyDrop<PageTableFrag<EptPtConfig>>>, guard: &EptGuard) {
    if frags.is_empty() {
        return;
    }

    // ManuallyDrop retains backing frames and page-table nodes if invalidation
    // fails or panics. Successful invalidation makes their reclamation safe.
    guard
        .invalidate()
        .expect("failed to invalidate EPT translations");
    for frag in frags {
        drop(ManuallyDrop::into_inner(frag));
    }
}

/// A queried mapping item.
///
/// The address is the host physical address backing the current guest physical
/// range, together with the page properties used for that mapping.
pub type QueriedItem = (Paddr, PageProperty);

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
    /// locked, it will return the guest physical address range and the mapped
    /// host physical address item.
    pub fn query(&mut self) -> Result<(Range<Gpaddr>, Option<QueriedItem>)> {
        let (range, item) = self.0.query()?;
        Ok((range, item.map(|(frame, prop)| (frame.paddr(), prop))))
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
    ept_guard: &'a EptGuard,
}

impl<'a> CursorMut<'a> {
    /// Queries the mapping at the current guest physical address.
    ///
    /// This is the same as [`Cursor::query`].
    ///
    /// If the cursor is pointing to a valid guest physical address that is
    /// locked, it will return the guest physical address range and the mapped
    /// host physical address item.
    pub fn query(&mut self) -> Result<(Range<Gpaddr>, Option<QueriedItem>)> {
        let (range, item) = self.pt_cursor.query()?;
        Ok((range, item.map(|(frame, prop)| (frame.paddr(), prop))))
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
    /// Panics if the slot is already mapped, is outside the cursor's range,
    /// or the permissions do not include read access.
    pub fn map(&mut self, frame: UFrame, prop: PageProperty) {
        assert!(
            prop.flags.contains(PageFlags::R),
            "EPT mappings must permit reads"
        );
        let item: EptItem = (frame, prop);

        // SAFETY: It is safe to map untyped memory into guest physical memory.
        unsafe { self.pt_cursor.map(item) };
    }

    /// Updates the permissions of the next mapped page within `len` bytes.
    ///
    /// Returns its guest physical range and advances past that page, or returns
    /// `None` if the range has no mapping. Cached translations are invalidated
    /// before this method returns.
    ///
    /// # Panics
    ///
    /// Panics if IRQs are disabled, `len` is unaligned or exceeds the remaining
    /// cursor range, or the new permissions do not include read access.
    pub fn protect_next(
        &mut self,
        len: usize,
        op: &mut impl FnMut(&mut PageFlags),
    ) -> Option<Range<Gpaddr>> {
        assert!(crate::arch::irq::is_local_enabled());
        // SAFETY: Only guest permissions are changed. The privileged flags
        // and backing-frame ownership remain intact, and INVEPT completes
        // before reporting success to the caller.
        let range = unsafe {
            self.pt_cursor.protect_next(len, &mut |prop| {
                op(&mut prop.flags);
                assert!(
                    prop.flags.contains(PageFlags::R),
                    "EPT mappings must permit reads"
                );
            })
        }?;
        self.ept_guard
            .invalidate()
            .expect("failed to invalidate EPT translations");
        Some(range)
    }

    /// Unmaps mappings from the current guest physical address.
    ///
    /// The method removes mapped pages or page-table subtrees up to `len`
    /// bytes from the current guest physical address, flushes TLB,
    /// and returns the number of unmapped frames or page-table frames.
    ///
    /// # Panics
    ///
    /// Panics if `len` is longer than the remaining range of the cursor or is
    /// not page-aligned, or if local IRQs are disabled.
    pub fn unmap(&mut self, len: usize) -> usize {
        assert!(crate::arch::irq::is_local_enabled());
        let end_gpa = self.gpa() + len;
        let mut num_unmapped: usize = 0;
        let mut frags = Vec::new();
        loop {
            // SAFETY: It is safe to un-map memory in the guest physical memory space.
            // And the un-mapped items are dropped after TLB flushes.
            let Some(frag) = (unsafe { self.pt_cursor.take_next(end_gpa - self.gpa()) }) else {
                break; // No more mappings in the range.
            };

            num_unmapped += match &frag {
                PageTableFrag::Mapped { .. } => 1,
                PageTableFrag::StrayPageTable { num_frames, .. } => *num_frames,
            };

            frags.push(ManuallyDrop::new(frag));
        }

        flush_and_drop(frags, self.ept_guard);
        num_unmapped
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

        {
            let guard = disable_preempt();
            let mut cursor = space.cursor_mut(&guard, &range).unwrap();
            cursor.map(first.into(), prop);
            cursor.map(second.into(), prop);
            assert_eq!(cursor.gpa(), 3 * PAGE_SIZE);

            cursor.jump(2 * PAGE_SIZE).unwrap();
            assert_eq!(cursor.query().unwrap().1, Some((second_paddr, prop)));
            cursor.jump(PAGE_SIZE).unwrap();
            assert_eq!(
                cursor.query().unwrap(),
                (PAGE_SIZE..2 * PAGE_SIZE, Some((first_paddr, prop)))
            );
            assert_eq!(
                cursor.protect_next(PAGE_SIZE, &mut |flags| *flags = PageFlags::RX),
                Some(PAGE_SIZE..2 * PAGE_SIZE)
            );

            cursor.jump(PAGE_SIZE).unwrap();
            assert_eq!(cursor.query().unwrap().1.unwrap().1.flags, PageFlags::RX);
            // This executes INVEPT in VMX root operation before releasing mappings.
            assert_eq!(cursor.unmap(2 * PAGE_SIZE), 2);
            cursor.jump(PAGE_SIZE).unwrap();
            assert!(cursor.find_next(3 * PAGE_SIZE).is_none());
        }

        // Reuse the same GPA after invalidation and observe the replacement frame.
        {
            let guard = disable_preempt();
            let frame = FrameAllocOptions::new().alloc_frame().unwrap();
            let paddr = frame.paddr();
            space
                .cursor_mut(&guard, &range)
                .unwrap()
                .map(frame.into(), prop);
            let mut cursor = space.cursor(&guard, &range).unwrap();
            assert_eq!(cursor.query().unwrap().1, Some((paddr, prop)));
        }

        // Dropping the address space invalidates its remaining mapping and keeps
        // VMX enabled until reclamation completes.
        drop(space);
    }
}
