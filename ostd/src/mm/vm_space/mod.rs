// SPDX-License-Identifier: MPL-2.0

//! Virtual memory space management.
//!
//! The [`VmSpace`] struct is provided to manage the virtual memory space of a
//! user. Cursors are used to traverse and modify over the virtual memory space
//! concurrently. The VM space cursor [`self::Cursor`] is just a wrapper over
//! the page table cursor, providing efficient, powerful concurrent accesses
//! to the page table.

#[cfg(ktest)]
mod test;

use core::{ops::Range, sync::atomic::Ordering};

use super::{AnyUFrameMeta, PagingLevel, page_table::PageTableConfig};
use crate::{
    Error,
    arch::mm::{PageTableEntry, PagingConsts, current_page_table_paddr},
    cpu::{AtomicCpuSet, CpuSet, PinCurrentCpu},
    cpu_local_cell,
    io::IoMem,
    mm::{
        Frame, HasPaddr, MAX_USERSPACE_VADDR, PAGE_SIZE, PageProperty, PrivilegedPageFlags, UFrame,
        VmReader, VmWriter,
        frame::FrameRef,
        io::Fallible,
        kspace::KERNEL_PAGE_TABLE,
        page_prop::{CachePolicy, PageFlags},
        page_table::{
            self, AuxPageTableMeta, PageTable, PageTableFrag, PteStateRef, largest_pages,
        },
        tlb::{TlbFlushOp, TlbFlusher},
    },
    prelude::*,
    sync::{RcuDrop, SpinLock},
    task::{DisabledPreemptGuard, atomic_mode::AsAtomicModeGuard, disable_preempt},
};

/// A virtual address space for user-mode tasks, enabling safe manipulation of user-space memory.
///
/// The `VmSpace` type provides memory isolation guarantees between user-space and
/// kernel-space. For example, given an arbitrary user-space pointer, one can read and
/// write the memory location referred to by the user-space pointer without the risk of
/// breaking the memory safety of the kernel space.
///
/// # Task Association Semantics
///
/// As far as OSTD is concerned, a `VmSpace` is not necessarily associated with a task. Once a
/// `VmSpace` is activated (see [`VmSpace::activate`]), it remains activated until another
/// `VmSpace` is activated **possibly by another task running on the same CPU**.
///
/// This means that it's up to the kernel to ensure that a task's `VmSpace` is always activated
/// while the task is running. This can be done by using the injected post schedule handler
/// (see [`inject_post_schedule_handler`]) to always activate the correct `VmSpace` after each
/// context switch.
///
/// If the kernel otherwise decides not to ensure that the running task's `VmSpace` is always
/// activated, the kernel must deal with race conditions when calling methods that require the
/// `VmSpace` to be activated, e.g., [`UserMode::execute`], [`VmSpace::reader`],
/// [`VmSpace::writer`]. Otherwise, the behavior is unspecified, though it's guaranteed _not_ to
/// compromise the kernel's memory safety.
///
/// # Memory Backing
///
/// A newly-created `VmSpace` is not backed by any physical memory pages. To
/// provide memory pages for a `VmSpace`, one can allocate and map physical
/// memory ([`UFrame`]s) to the `VmSpace` using the cursor.
///
/// A `VmSpace` can also attach a page fault handler, which will be invoked to
/// handle page faults generated from user space.
///
/// [`inject_post_schedule_handler`]: crate::task::inject_post_schedule_handler
/// [`UserMode::execute`]: crate::user::UserMode::execute
#[derive(Debug)]
pub struct VmSpace<A: AuxPageTableMeta = ()> {
    pt: PageTable<UserPtConfig<A>>,
    cpus: Arc<AtomicCpuSet>,
    iomems: SpinLock<Vec<IoMem>>,
}

impl<A: AuxPageTableMeta> VmSpace<A> {
    /// Creates a new VM address space.
    pub fn new() -> Self {
        Self {
            pt: KERNEL_PAGE_TABLE
                .get()
                .unwrap()
                .create_user_page_table::<A>(),
            cpus: Arc::new(AtomicCpuSet::new(CpuSet::new_empty())),
            iomems: SpinLock::new(Vec::new()),
        }
    }

    /// Gets an immutable cursor in the virtual address range.
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
        va: &Range<Vaddr>,
    ) -> Result<Cursor<'a, A>> {
        Ok(Cursor(self.pt.cursor(guard, va)?))
    }

    /// Gets an mutable cursor in the virtual address range.
    ///
    /// The same as [`Self::cursor`], the cursor behaves like a lock guard,
    /// exclusively owning a sub-tree of the page table, preventing others
    /// from creating a cursor in it. So be sure to drop the cursor as soon as
    /// possible.
    ///
    /// The creation of the cursor may block if another cursor having an
    /// overlapping range is alive. The modification to the mapping by the
    /// cursor may also block or be overridden the mapping of another cursor.
    pub fn cursor_mut<'a, G: AsAtomicModeGuard>(
        &'a self,
        guard: &'a G,
        va: &Range<Vaddr>,
    ) -> Result<CursorMut<'a, A>> {
        Ok(CursorMut {
            pt_cursor: self.pt.cursor_mut(guard, va)?,
            flusher: TlbFlusher::new(&self.cpus, disable_preempt()),
            vmspace: self,
        })
    }

    /// Activates the page table on the current CPU.
    pub fn activate(&self) {
        let preempt_guard = disable_preempt();
        let cpu = preempt_guard.current_cpu();

        let last_cpus_ptr = ACTIVATED_VM_SPACE_CPUSET.load();

        if last_cpus_ptr == Arc::as_ptr(&self.cpus) {
            return;
        }

        // Record ourselves in the CPU set and the activated VM space pointer.
        // `Acquire` to ensure the modification to the PT is visible by this CPU.
        self.cpus.add(cpu, Ordering::Acquire);

        let self_cpus_ptr = Arc::into_raw(Arc::clone(&self.cpus)) as *mut AtomicCpuSet;
        ACTIVATED_VM_SPACE_CPUSET.store(self_cpus_ptr);

        if !last_cpus_ptr.is_null() {
            // SAFETY: The pointer is cast from an `Arc` when it's activated
            // the last time, so it can be restored and only restored once.
            let last = unsafe { Arc::from_raw(last_cpus_ptr) };
            last.remove(cpu, Ordering::Relaxed);
        }

        self.pt.activate();
    }

    /// Creates a reader to read data from the user space of the current task.
    ///
    /// Returns `Err` if this `VmSpace` doesn't belong to the user space of the current task
    /// or the `vaddr` and `len` do not represent a user space memory range.
    ///
    /// Users must ensure that no other page table is activated in the current task during the
    /// lifetime of the created `VmReader`. This guarantees that the `VmReader` can operate correctly.
    pub fn reader(&self, vaddr: Vaddr, len: usize) -> Result<VmReader<'_, Fallible>> {
        if current_page_table_paddr() != self.pt.root_paddr() {
            return Err(Error::AccessDenied);
        }

        if vaddr.saturating_add(len) > MAX_USERSPACE_VADDR {
            return Err(Error::AccessDenied);
        }

        // SAFETY: The memory range is in user space, as checked above.
        Ok(unsafe { VmReader::<Fallible>::from_user_space(vaddr as *const u8, len) })
    }

    /// Creates a writer to write data into the user space.
    ///
    /// Returns `Err` if this `VmSpace` doesn't belong to the user space of the current task
    /// or the `vaddr` and `len` do not represent a user space memory range.
    ///
    /// Users must ensure that no other page table is activated in the current task during the
    /// lifetime of the created `VmWriter`. This guarantees that the `VmWriter` can operate correctly.
    pub fn writer(&self, vaddr: Vaddr, len: usize) -> Result<VmWriter<'_, Fallible>> {
        if current_page_table_paddr() != self.pt.root_paddr() {
            return Err(Error::AccessDenied);
        }

        if vaddr.saturating_add(len) > MAX_USERSPACE_VADDR {
            return Err(Error::AccessDenied);
        }

        // `VmWriter` is neither `Sync` nor `Send`, so it will not live longer than the current
        // task. This ensures that the correct page table is activated during the usage period of
        // the `VmWriter`.
        //
        // SAFETY: The memory range is in user space, as checked above.
        Ok(unsafe { VmWriter::<Fallible>::from_user_space(vaddr as *mut u8, len) })
    }

    /// Creates a reader/writer pair to read data from and write data into the user space.
    ///
    /// Returns `Err` if this `VmSpace` doesn't belong to the user space of the current task
    /// or the `vaddr` and `len` do not represent a user space memory range.
    ///
    /// Users must ensure that no other page table is activated in the current task during the
    /// lifetime of the created `VmReader` and `VmWriter`. This guarantees that the `VmReader`
    /// and the `VmWriter` can operate correctly.
    ///
    /// This method is semantically equivalent to calling [`Self::reader`] and [`Self::writer`]
    /// separately, but it avoids double checking the validity of the memory region.
    pub fn reader_writer(
        &self,
        vaddr: Vaddr,
        len: usize,
    ) -> Result<(VmReader<'_, Fallible>, VmWriter<'_, Fallible>)> {
        if current_page_table_paddr() != self.pt.root_paddr() {
            return Err(Error::AccessDenied);
        }

        if vaddr.saturating_add(len) > MAX_USERSPACE_VADDR {
            return Err(Error::AccessDenied);
        }

        // SAFETY: The memory range is in user space, as checked above.
        let reader = unsafe { VmReader::<Fallible>::from_user_space(vaddr as *const u8, len) };

        // `VmWriter` is neither `Sync` nor `Send`, so it will not live longer than the current
        // task. This ensures that the correct page table is activated during the usage period of
        // the `VmWriter`.
        //
        // SAFETY: The memory range is in user space, as checked above.
        let writer = unsafe { VmWriter::<Fallible>::from_user_space(vaddr as *mut u8, len) };

        Ok((reader, writer))
    }
}

impl<A: AuxPageTableMeta> Default for VmSpace<A> {
    fn default() -> Self {
        Self::new()
    }
}

impl<A: AuxPageTableMeta> VmSpace<A> {
    /// Finds the [`IoMem`] that contains the given physical address.
    ///
    /// It is a private method for internal use only. Please refer to
    /// [`CursorMut::find_iomem_by_paddr`] for more details.
    fn find_iomem_by_paddr(&self, paddr: Paddr) -> Option<(IoMem, usize)> {
        let iomems = self.iomems.lock();
        for iomem in iomems.iter() {
            let start = iomem.paddr();
            let end = start + iomem.size();
            if paddr >= start && paddr < end {
                let offset = paddr - start;
                return Some((iomem.clone(), offset));
            }
        }
        None
    }
}

/// The cursor for querying over the VM space without modifying it.
///
/// It exclusively owns a sub-tree of the page table, preventing others from
/// reading or modifying the same sub-tree. Two read-only cursors can not be
/// created from the same virtual address range either.
pub struct Cursor<'a, A: AuxPageTableMeta = ()>(page_table::Cursor<'a, UserPtConfig<A>>);

impl<A: AuxPageTableMeta> Cursor<'_, A> {
    /// Queries the mapping at the current virtual address.
    pub fn query(&mut self) -> VmQueriedItem<'_> {
        self.0.query().into()
    }

    /// Moves the cursor forward to the next mapped virtual address.
    ///
    /// If there is mapped virtual address following the current address within
    /// next `len` bytes, it will return that mapped address. In this case,
    /// the cursor will stop at the mapped address.
    ///
    /// Otherwise, it will return `None`. And the cursor may stop at any
    /// address within `len` bytes.
    ///
    /// # Panics
    ///
    /// Panics if the length is longer than the remaining range of the cursor.
    pub fn find_next(&mut self, len: usize) -> Option<Vaddr> {
        self.0.find_next(len)
    }

    /// Jump to the virtual address.
    pub fn jump(&mut self, va: Vaddr) -> Result<()> {
        self.0.jump(va)?;
        Ok(())
    }

    /// Get the virtual address of the current slot.
    pub fn virt_addr(&self) -> Vaddr {
        self.0.virt_addr()
    }

    /// Get the current level of the cursor.
    pub fn level(&self) -> PagingLevel {
        self.0.level()
    }

    /// Get the current virtual address range of the cursor.
    pub fn cur_va_range(&self) -> Range<Vaddr> {
        self.0.cur_va_range()
    }

    /// Moves the cursor down to the next level if the next level page table exists.
    ///
    /// Returns the new level if the next level page table exists, or `None` otherwise.
    pub fn push_level_if_exists(&mut self) -> Option<PagingLevel> {
        self.0.push_level_if_exists()
    }
}

/// The cursor for modifying the mappings in VM space.
///
/// It exclusively owns a sub-tree of the page table, preventing others from
/// reading or modifying the same sub-tree.
pub struct CursorMut<'a, A: AuxPageTableMeta = ()> {
    pt_cursor: page_table::CursorMut<'a, UserPtConfig<A>>,
    // We have a read lock so the CPU set in the flusher is always a superset
    // of actual activated CPUs.
    flusher: TlbFlusher<'a, DisabledPreemptGuard>,
    // References to the `VmSpace`
    vmspace: &'a VmSpace<A>,
}

impl<'a, A: AuxPageTableMeta> CursorMut<'a, A> {
    /// Queries the mapping at the current virtual address.
    ///
    /// This is the same as [`Cursor::query`].
    ///
    /// If the cursor is pointing to a valid virtual address that is locked,
    /// it will return the virtual address range and the mapped item.
    pub fn query(&self) -> VmQueriedItem<'_> {
        self.pt_cursor.query().into()
    }

    /// Moves the cursor forward to the next mapped virtual address.
    ///
    /// This is the same as [`Cursor::find_next`].
    pub fn find_next(&mut self, len: usize) -> Option<Vaddr> {
        self.pt_cursor.find_next(len)
    }

    /// Moves the cursor forward to the largest possible subtree that contains
    /// mapped pages.
    ///
    /// This is similar to [`Self::find_next`], except that the cursor will
    /// stop at the highest possible level, that the subtree's virtual address
    /// range is fully covered by `len`. This is useful for
    /// [`CursorMut::unmap`].
    pub fn find_next_unmappable_subtree(&mut self, len: usize) -> Option<Vaddr> {
        self.pt_cursor.find_next_unmappable_subtree(len)
    }

    /// Jump to the virtual address.
    ///
    /// This is the same as [`Cursor::jump`].
    pub fn jump(&mut self, va: Vaddr) -> Result<()> {
        self.pt_cursor.jump(va)?;
        Ok(())
    }

    /// Adjusts the level of the cursor to the given level.
    ///
    /// When the specified level page table is not allocated, it will allocate
    /// and go to that page table. If the current virtual address contains a
    /// huge mapping, and the specified level is lower than the mapping, it
    /// will split the huge mapping into smaller mappings.
    ///
    /// # Panics
    ///
    /// Panics if the specified level is invalid.
    pub fn adjust_level(&mut self, level: PagingLevel) {
        self.pt_cursor.adjust_level(level);
    }

    /// Get the virtual address of the current slot.
    pub fn virt_addr(&self) -> Vaddr {
        self.pt_cursor.virt_addr()
    }

    /// Get the current level of the cursor.
    pub fn level(&self) -> PagingLevel {
        self.pt_cursor.level()
    }

    /// Moves the cursor down to the next level if the next level page table exists.
    ///
    /// Returns the new level if the next level page table exists, or `None` otherwise.
    pub fn push_level_if_exists(&mut self) -> Option<PagingLevel> {
        self.pt_cursor.push_level_if_exists()
    }

    /// Get the current virtual address range of the cursor.
    pub fn cur_va_range(&self) -> Range<Vaddr> {
        self.pt_cursor.cur_va_range()
    }

    /// Get the dedicated TLB flusher for this cursor.
    pub fn flusher(&mut self) -> &mut TlbFlusher<'a, DisabledPreemptGuard> {
        &mut self.flusher
    }

    /// Maps a frame into the current slot.
    ///
    /// # Panics
    ///
    /// Panics if the current virtual address is already mapped.
    pub fn map(&mut self, frame: UFrame, prop: PageProperty) {
        let item = VmItem::new_tracked(frame, prop);

        // SAFETY: It is safe to map untyped memory into the userspace.
        unsafe { self.pt_cursor.map(item) };
    }

    /// Maps a range of [`IoMem`] into the current slot.
    ///
    /// The memory region to be mapped is the [`IoMem`] range starting at
    /// `offset` and extending to `offset + len`, or to the end of [`IoMem`],
    /// whichever comes first.
    ///
    /// # Limitations
    ///
    /// Once an instance of `IoMem` is mapped to a `VmSpace`,
    /// then the `IoMem` instance will only be dropped when the `VmSpace` is
    /// dropped, not when all the mappings backed by the `IoMem` are destroyed
    /// with the `unmap` method.
    ///
    /// # Panics
    ///
    /// Panics if
    ///  - `len` or `offset` is not aligned to the page size;
    ///  - the current virtual address is already mapped.
    pub fn map_iomem(&mut self, io_mem: IoMem, prop: PageProperty, len: usize, offset: usize) {
        assert_eq!(len % PAGE_SIZE, 0);
        assert_eq!(offset % PAGE_SIZE, 0);

        if offset >= io_mem.size() {
            return;
        }

        let paddr_begin = io_mem.paddr() + offset;
        let map_size = if io_mem.size() - offset < len {
            io_mem.size() - offset
        } else {
            len
        };

        let cur_va = self.pt_cursor.virt_addr();

        for (va, pa, level) in largest_pages::<UserPtConfig>(cur_va, paddr_begin, map_size) {
            self.pt_cursor.jump(va).unwrap();
            self.pt_cursor.adjust_level(level);
            // SAFETY: It is safe to map I/O memory into the userspace.
            unsafe {
                self.pt_cursor
                    .map(VmItem::new_untracked_io(pa, level, prop))
            };
        }

        // If the `iomems` list in `VmSpace` does not contain the current I/O
        // memory, push it to maintain the correct reference count.
        let mut iomems = self.vmspace.iomems.lock();
        if !iomems
            .iter()
            .any(|iomem| iomem.paddr() == io_mem.paddr() && iomem.size() == io_mem.size())
        {
            iomems.push(io_mem);
        }
    }

    /// Finds an [`IoMem`] that was previously mapped to by [`Self::map_iomem`] and contains the
    /// physical address.
    ///
    /// This method can recover the originally mapped `IoMem` from the physical address returned by
    /// [`Self::query`]. If the query returns a [`VmQueriedItem::MappedIoMem`], this method is
    /// guaranteed to succeed with the specific physical address. However, if the corresponding
    /// mapping is subsequently unmapped, it is unspecified whether this method will still succeed
    /// or not.
    ///
    /// On success, this method returns the `IoMem` and the offset from the `IoMem` start to the
    /// given physical address. Otherwise, this method returns `None`.
    pub fn find_iomem_by_paddr(&self, paddr: Paddr) -> Option<(IoMem, usize)> {
        self.vmspace.find_iomem_by_paddr(paddr)
    }

    /// Removes all the mappings at the current PTE.
    ///
    /// The unmapped virtual address range depends on the current level of the
    /// cursor, and can be queried via [`Self::cur_va_range`]. Adjust the
    /// level via [`Self::adjust_level`] before unmapping to change the
    ///
    /// The number of unmapped frames is returned.
    ///
    /// # Panics
    ///
    /// Panics if the current level is at the top level.
    pub fn unmap(&mut self) -> usize {
        // SAFETY: It is safe to un-map memory in the userspace. And the
        // un-mapped items are dropped after TLB flushes.
        let Some(frag) = (unsafe { self.pt_cursor.unmap() }) else {
            return 0;
        };

        match frag {
            PageTableFrag::Mapped { va, item, .. } => {
                // SAFETY: If the item is not a scalar (e.g., a frame
                // pointer), we will drop it after the RCU grace period
                // (see `issue_tlb_flush_with`).
                let (item, panic_guard) = unsafe { RcuDrop::into_inner(item) };

                match item {
                    VmItem {
                        mapped_item: MappedItem::TrackedFrame(old_frame),
                        ..
                    } => {
                        let rcu_frame = RcuDrop::new(old_frame);
                        panic_guard.forget();
                        let rcu_frame = Frame::rcu_from_unsized(rcu_frame);
                        self.flusher
                            .issue_tlb_flush_with(TlbFlushOp::for_single(va), rcu_frame);

                        1
                    }
                    VmItem {
                        mapped_item: MappedItem::UntrackedIoMem { .. },
                        ..
                    } => {
                        panic_guard.forget();

                        // Flush the TLB entry for the current address, but
                        // in the current design, we cannot drop the
                        // corresponding `IoMem`. This is because we manage
                        // the range of I/O as a whole, but the frames
                        // handled here might be one segment of it.
                        self.flusher.issue_tlb_flush(TlbFlushOp::for_single(va));

                        0
                    }
                }
            }
            PageTableFrag::StrayPageTable {
                pt,
                va,
                len,
                num_frames,
            } => {
                self.flusher.issue_tlb_flush_with(
                    TlbFlushOp::for_range(va..va + len),
                    Frame::rcu_from_unsized(pt),
                );

                num_frames
            }
        }
    }

    /// Applies the operation to the current PTE.
    ///
    /// The unmapped virtual address range depends on the current level of the
    /// cursor, and can be queried via [`Self::cur_va_range`]. Adjust the
    /// level via [`Self::adjust_level`] before unmapping to change the
    pub fn protect(&mut self, mut op: impl FnMut(&mut PageFlags, &mut CachePolicy)) {
        // SAFETY: It is safe to set `PageFlags` and `CachePolicy` of memory
        // in the userspace.
        unsafe {
            self.pt_cursor.protect(&mut |prop| {
                op(&mut prop.flags, &mut prop.cache);
            })
        }
    }
}

cpu_local_cell! {
    /// The `Arc` pointer to the activated VM space's `cpus` field on this CPU.
    /// If the pointer is NULL, it means that the activated page table is the
    /// kernel page table.
    // TODO: If we are enabling ASID, we need to maintain the TLB state of each
    // CPU, rather than merely the activated `VmSpace`. When ASID is enabled,
    // the non-active `VmSpace`s can still have their TLB entries in the CPU!
    static ACTIVATED_VM_SPACE_CPUSET: *const AtomicCpuSet = core::ptr::null();
}

/// The result of a query over the VM space.
pub enum VmQueriedItem<'a> {
    /// The current PTE is absent.
    None,
    /// The current PTE points to a child page table.
    PageTable,
    /// The current slot is mapped, the frame within is allocated from the
    /// physical memory.
    MappedRam {
        /// The mapped frame.
        frame: FrameRef<'a, dyn AnyUFrameMeta>,
        /// The property of the slot.
        prop: PageProperty,
    },
    /// The current slot is mapped, the frame within is allocated from the
    /// MMIO memory.
    MappedIoMem {
        /// The physical address of the corresponding I/O memory.
        paddr: Paddr,
        /// The property of the slot.
        prop: PageProperty,
    },
}

impl VmQueriedItem<'_> {
    /// Returns `true` if the queried item is not [`VmQueriedItem::None`].
    pub fn is_some(&self) -> bool {
        !self.is_none()
    }

    /// Returns `true` if the queried item is [`VmQueriedItem::None`].
    pub fn is_none(&self) -> bool {
        matches!(self, VmQueriedItem::None)
    }
}

/// Internal representation of a VM item.
///
/// This is kept private to ensure memory safety. The public interface
/// should use `VmQueriedItem` for querying mapping information.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct VmItem {
    prop: PageProperty,
    mapped_item: MappedItem,
}

/// A reference to a VM item.
#[derive(Debug)]
pub(crate) struct VmItemRef<'a> {
    prop: PageProperty,
    mapped_item: MappedItemRef<'a>,
}

#[derive(Debug, Clone, PartialEq)]
enum MappedItem {
    TrackedFrame(UFrame),
    UntrackedIoMem { paddr: Paddr, level: PagingLevel },
}

#[derive(Debug)]
enum MappedItemRef<'a> {
    TrackedFrame(FrameRef<'a, dyn AnyUFrameMeta>),
    UntrackedIoMem { paddr: Paddr, level: PagingLevel },
}

impl VmItem {
    /// Creates a new `VmItem` that maps a tracked frame.
    pub(super) fn new_tracked(frame: UFrame, prop: PageProperty) -> Self {
        Self {
            prop,
            mapped_item: MappedItem::TrackedFrame(frame),
        }
    }

    /// Creates a new `VmItem` that maps an untracked I/O memory.
    fn new_untracked_io(paddr: Paddr, level: PagingLevel, prop: PageProperty) -> Self {
        Self {
            prop,
            mapped_item: MappedItem::UntrackedIoMem { paddr, level },
        }
    }
}

impl<'a, A: AuxPageTableMeta> From<PteStateRef<'a, UserPtConfig<A>>> for VmQueriedItem<'a> {
    fn from(state: PteStateRef<'a, UserPtConfig<A>>) -> Self {
        match state {
            PteStateRef::Absent => VmQueriedItem::None,
            PteStateRef::PageTable { .. } => VmQueriedItem::PageTable,
            PteStateRef::Mapped(item) => match item.mapped_item {
                MappedItemRef::TrackedFrame(frame) => VmQueriedItem::MappedRam {
                    frame,
                    prop: item.prop,
                },
                MappedItemRef::UntrackedIoMem { paddr, level } => {
                    debug_assert_eq!(level, 1);
                    VmQueriedItem::MappedIoMem {
                        paddr,
                        prop: item.prop,
                    }
                }
            },
        }
    }
}

/// The page table configuration for user space page tables.
#[derive(Clone, Debug)]
pub struct UserPtConfig<A: AuxPageTableMeta = ()> {
    _phantom: core::marker::PhantomData<A>,
}

// SAFETY: `item_raw_info`, `item_into_raw`, `item_from_raw`, and
// `item_ref_from_raw` are correctly implemented with respect to the `Item` and
// `ItemRef` types.
unsafe impl<A: AuxPageTableMeta> PageTableConfig for UserPtConfig<A> {
    const TOP_LEVEL_INDEX_RANGE: Range<usize> = 0..256;

    type E = PageTableEntry;
    type C = PagingConsts;
    type Aux = A;

    type Item = VmItem;
    type ItemRef<'a> = VmItemRef<'a>;

    fn item_raw_info(item: &Self::Item) -> (Paddr, PagingLevel, PageProperty) {
        match &item.mapped_item {
            MappedItem::TrackedFrame(frame) => {
                let mut prop = item.prop;
                prop.priv_flags -= PrivilegedPageFlags::AVAIL1; // Clear AVAIL1 for tracked frames
                let level = frame.map_level();
                let paddr = frame.paddr();
                (paddr, level, prop)
            }
            MappedItem::UntrackedIoMem { paddr, level } => {
                let mut prop = item.prop;
                prop.priv_flags |= PrivilegedPageFlags::AVAIL1; // Set AVAIL1 for I/O memory
                (*paddr, *level, prop)
            }
        }
    }

    unsafe fn item_from_raw(paddr: Paddr, level: PagingLevel, prop: PageProperty) -> Self::Item {
        if prop.priv_flags.contains(PrivilegedPageFlags::AVAIL1) {
            // `AVAIL1` is set, this is I/O memory.
            VmItem::new_untracked_io(paddr, level, prop)
        } else {
            debug_assert_eq!(level, 1);
            // `AVAIL1` is clear, this is tracked memory.
            // SAFETY: The caller ensures safety.
            let frame = unsafe { Frame::<dyn AnyUFrameMeta>::from_raw(paddr) };
            VmItem::new_tracked(frame, prop)
        }
    }

    unsafe fn item_ref_from_raw<'a>(
        paddr: Paddr,
        level: PagingLevel,
        prop: PageProperty,
    ) -> Self::ItemRef<'a> {
        debug_assert_eq!(level, 1);
        if prop.priv_flags.contains(PrivilegedPageFlags::AVAIL1) {
            // `AVAIL1` is set, this is I/O memory.
            VmItemRef {
                prop,
                mapped_item: MappedItemRef::UntrackedIoMem { paddr, level },
            }
        } else {
            // `AVAIL1` is clear, this is tracked memory.
            // SAFETY: The caller ensures that the frame outlives `'a` and that
            // the type matches the frame.
            let frame_ref = unsafe { FrameRef::<dyn AnyUFrameMeta>::borrow_paddr(paddr) };
            VmItemRef {
                prop,
                mapped_item: MappedItemRef::TrackedFrame(frame_ref),
            }
        }
    }
}
