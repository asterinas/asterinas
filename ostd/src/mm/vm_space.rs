// SPDX-License-Identifier: MPL-2.0

//! Virtual memory space management.
//!
//! The [`VmSpace`] struct is provided to manage the virtual memory space of a
//! user. Cursors are used to traverse and modify over the virtual memory space
//! concurrently. The VM space cursor [`self::Cursor`] is just a wrapper over
//! the page table cursor [`super::page_table::Cursor`], providing efficient,
//! powerful concurrent accesses to the page table, and suffers from the same
//! validity concerns as described in [`super::page_table::cursor`].

use core::{ops::Range, sync::atomic::Ordering};

use crate::{
    arch::mm::{
        current_page_table_paddr, invpcid_all_excluding_global, tlb_flush_all_excluding_global,
        PageTableEntry, PagingConsts,
    },
    cpu::{AtomicCpuSet, CpuSet, PinCurrentCpu},
    cpu_local_cell,
    mm::{
        asid_allocation::{self, ASID_FLUSH_REQUIRED},
        io::Fallible,
        kspace::KERNEL_PAGE_TABLE,
        page_table::{self, PageTable, PageTableItem, UserMode},
        tlb::{TlbFlushOp, TlbFlusher, FLUSH_ALL_RANGE_THRESHOLD},
        PageProperty, UFrame, VmReader, VmWriter, MAX_USERSPACE_VADDR,
    },
    prelude::*,
    sync::{PreemptDisabled, RwLock, RwLockReadGuard},
    task::{disable_preempt, DisabledPreemptGuard},
    Error,
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
pub struct VmSpace {
    pt: PageTable<UserMode>,
    /// A CPU can only activate a `VmSpace` when no mutable cursors are alive.
    /// Cursors hold read locks and activation require a write lock.
    activation_lock: RwLock<()>,
    cpus: AtomicCpuSet,
    /// ASID
    asid: u16,
    asid_generation: u16,
}

impl VmSpace {
    /// Creates a new VM address space.
    pub fn new() -> Self {
        Self {
            pt: KERNEL_PAGE_TABLE.get().unwrap().create_user_page_table(),
            activation_lock: RwLock::new(()),
            cpus: AtomicCpuSet::new(CpuSet::new_empty()),
            asid: asid_allocation::allocate(),
            asid_generation: asid_allocation::current_generation(),
        }
    }

    /// Returns the ASID of this VM space.
    pub fn asid(&self) -> u16 {
        self.asid
    }

    /// Returns the ASID generation of this VM space.
    pub fn asid_generation(&self) -> u16 {
        self.asid_generation
    }

    /// Clears the user space mappings in the page table.
    ///
    /// This method returns error if the page table is activated on any other
    /// CPUs or there are any cursors alive.
    pub fn clear(&self) -> core::result::Result<(), VmSpaceClearError> {
        let preempt_guard = disable_preempt();
        let _guard = self
            .activation_lock
            .try_write()
            .ok_or(VmSpaceClearError::CursorsAlive)?;

        let cpus = self.cpus.load();
        let cpu = preempt_guard.current_cpu();
        let cpus_set_is_empty = cpus.is_empty();
        let cpus_set_is_single_self = cpus.count() == 1 && cpus.contains(cpu);

        if cpus_set_is_empty || cpus_set_is_single_self {
            // SAFETY: We have ensured that the page table is not activated on
            // other CPUs and no cursors are alive.
            unsafe { self.pt.clear() };
            if cpus_set_is_single_self {
                tlb_flush_all_excluding_global();
            }
            Ok(())
        } else {
            Err(VmSpaceClearError::PageTableActivated(cpus))
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
    pub fn cursor(&self, va: &Range<Vaddr>) -> Result<Cursor<'_>> {
        Ok(self.pt.cursor(va).map(Cursor)?)
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
    pub fn cursor_mut(&self, va: &Range<Vaddr>) -> Result<CursorMut<'_, '_>> {
        Ok(self.pt.cursor_mut(va).map(|pt_cursor| {
            let activation_lock = self.activation_lock.read();

            CursorMut {
                pt_cursor,
                activation_lock,
                flusher: TlbFlusher::new(self.cpus.load(), disable_preempt()),
            }
        })?)
    }

    /// Activates the page table on the current CPU.
    pub fn activate(self: &Arc<Self>) {
        let preempt_guard = disable_preempt();
        let cpu = preempt_guard.current_cpu();

        let last_ptr = ACTIVATED_VM_SPACE.load();

        if last_ptr == Arc::as_ptr(self) {
            return;
        }

        // Ensure no mutable cursors (which holds read locks) are alive before
        // we add the CPU to the CPU set.
        let _activation_lock = self.activation_lock.write();

        let current_generation = asid_allocation::current_generation();
        let need_flush =
            self.asid == ASID_FLUSH_REQUIRED || self.asid_generation != current_generation;

        if need_flush {
            unsafe {
                invpcid_all_excluding_global();
            }
        }

        // Record ourselves in the CPU set and the activated VM space pointer.
        self.cpus.add(cpu, Ordering::Relaxed);
        let self_ptr = Arc::into_raw(Arc::clone(self)) as *mut VmSpace;
        ACTIVATED_VM_SPACE.store(self_ptr);

        if !last_ptr.is_null() {
            // SAFETY: The pointer is cast from an `Arc` when it's activated
            // the last time, so it can be restored and only restored once.
            let last = unsafe { Arc::from_raw(last_ptr) };
            last.cpus.remove(cpu, Ordering::Relaxed);
        }

        self.pt.activate_with_asid(self.asid);
    }

    /// Creates a reader to read data from the user space of the current task.
    ///
    /// Returns `Err` if this `VmSpace` is not belonged to the user space of the current task
    /// or the `vaddr` and `len` do not represent a user space memory range.
    ///
    /// Users must ensure that no other page table is activated in the current task during the
    /// lifetime of the created `VmReader`. This guarantees that the `VmReader` can operate correctly.
    pub fn reader(&self, vaddr: Vaddr, len: usize) -> Result<VmReader<'_, Fallible>> {
        if current_page_table_paddr() != unsafe { self.pt.root_paddr() } {
            return Err(Error::AccessDenied);
        }

        if vaddr.checked_add(len).unwrap_or(usize::MAX) > MAX_USERSPACE_VADDR {
            return Err(Error::AccessDenied);
        }

        // SAFETY: The memory range is in user space, as checked above.
        Ok(unsafe { VmReader::<Fallible>::from_user_space(vaddr as *const u8, len) })
    }

    /// Creates a writer to write data into the user space.
    ///
    /// Returns `Err` if this `VmSpace` is not belonged to the user space of the current task
    /// or the `vaddr` and `len` do not represent a user space memory range.
    ///
    /// Users must ensure that no other page table is activated in the current task during the
    /// lifetime of the created `VmWriter`. This guarantees that the `VmWriter` can operate correctly.
    pub fn writer(&self, vaddr: Vaddr, len: usize) -> Result<VmWriter<'_, Fallible>> {
        if current_page_table_paddr() != unsafe { self.pt.root_paddr() } {
            return Err(Error::AccessDenied);
        }

        if vaddr.checked_add(len).unwrap_or(usize::MAX) > MAX_USERSPACE_VADDR {
            return Err(Error::AccessDenied);
        }

        // `VmWriter` is neither `Sync` nor `Send`, so it will not live longer than the current
        // task. This ensures that the correct page table is activated during the usage period of
        // the `VmWriter`.
        //
        // SAFETY: The memory range is in user space, as checked above.
        Ok(unsafe { VmWriter::<Fallible>::from_user_space(vaddr as *mut u8, len) })
    }
}

impl Drop for VmSpace {
    fn drop(&mut self) {
        // Ensure the VmSpace is not activated on any CPU
        let cpus = self.cpus.load();
        assert!(cpus.is_empty(), "attempt to drop an activated VmSpace");

        asid_allocation::deallocate(self.asid);
    }
}

impl Default for VmSpace {
    fn default() -> Self {
        Self::new()
    }
}

/// An error that may occur when doing [`VmSpace::clear`].
#[derive(Debug)]
pub enum VmSpaceClearError {
    /// The page table is activated on other CPUs.
    ///
    /// The activated CPUs detected are contained in the error.
    PageTableActivated(CpuSet),
    /// There are still cursors alive.
    CursorsAlive,
}

/// The cursor for querying over the VM space without modifying it.
///
/// It exclusively owns a sub-tree of the page table, preventing others from
/// reading or modifying the same sub-tree. Two read-only cursors can not be
/// created from the same virtual address range either.
pub struct Cursor<'a>(page_table::Cursor<'a, UserMode, PageTableEntry, PagingConsts>);

impl Iterator for Cursor<'_> {
    type Item = VmItem;

    fn next(&mut self) -> Option<Self::Item> {
        let result = self.query();
        if result.is_ok() {
            self.0.move_forward();
        }
        result.ok()
    }
}

impl Cursor<'_> {
    /// Query about the current slot.
    ///
    /// This function won't bring the cursor to the next slot.
    pub fn query(&mut self) -> Result<VmItem> {
        Ok(self.0.query().map(|item| item.try_into().unwrap())?)
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
}

/// The cursor for modifying the mappings in VM space.
///
/// It exclusively owns a sub-tree of the page table, preventing others from
/// reading or modifying the same sub-tree.
pub struct CursorMut<'a, 'b> {
    pt_cursor: page_table::CursorMut<'a, UserMode, PageTableEntry, PagingConsts>,
    #[expect(dead_code)]
    activation_lock: RwLockReadGuard<'b, (), PreemptDisabled>,
    // We have a read lock so the CPU set in the flusher is always a superset
    // of actual activated CPUs.
    flusher: TlbFlusher<DisabledPreemptGuard>,
}

impl CursorMut<'_, '_> {
    /// Query about the current slot.
    ///
    /// This is the same as [`Cursor::query`].
    ///
    /// This function won't bring the cursor to the next slot.
    pub fn query(&mut self) -> Result<VmItem> {
        Ok(self
            .pt_cursor
            .query()
            .map(|item| item.try_into().unwrap())?)
    }

    /// Jump to the virtual address.
    ///
    /// This is the same as [`Cursor::jump`].
    pub fn jump(&mut self, va: Vaddr) -> Result<()> {
        self.pt_cursor.jump(va)?;
        Ok(())
    }

    /// Get the virtual address of the current slot.
    pub fn virt_addr(&self) -> Vaddr {
        self.pt_cursor.virt_addr()
    }

    /// Get the dedicated TLB flusher for this cursor.
    pub fn flusher(&mut self) -> &mut TlbFlusher<DisabledPreemptGuard> {
        &mut self.flusher
    }

    /// Map a frame into the current slot.
    ///
    /// This method will bring the cursor to the next slot after the modification.
    pub fn map(&mut self, frame: UFrame, prop: PageProperty) {
        let start_va = self.virt_addr();
        // SAFETY: It is safe to map untyped memory into the userspace.
        let old = unsafe { self.pt_cursor.map(frame.into(), prop) };

        if let Some(old) = old {
            self.flusher
                .issue_tlb_flush_with(TlbFlushOp::Address(start_va), old);
            self.flusher.dispatch_tlb_flush();
        }
    }

    /// Clear the mapping starting from the current slot.
    ///
    /// This method will bring the cursor forward by `len` bytes in the virtual
    /// address space after the modification.
    ///
    /// Already-absent mappings encountered by the cursor will be skipped. It
    /// is valid to unmap a range that is not mapped.
    ///
    /// It must issue and dispatch a TLB flush after the operation. Otherwise,
    /// the memory safety will be compromised. Please call this function less
    /// to avoid the overhead of TLB flush. Using a large `len` is wiser than
    /// splitting the operation into multiple small ones.
    ///
    /// # Panics
    ///
    /// This method will panic if `len` is not page-aligned.
    pub fn unmap(&mut self, len: usize) {
        assert!(len % super::PAGE_SIZE == 0);
        let end_va = self.virt_addr() + len;
        let tlb_prefer_flush_all = len > FLUSH_ALL_RANGE_THRESHOLD;

        loop {
            // SAFETY: It is safe to un-map memory in the userspace.
            let result = unsafe { self.pt_cursor.take_next(end_va - self.virt_addr()) };
            match result {
                PageTableItem::Mapped { va, page, .. } => {
                    if !self.flusher.need_remote_flush() && tlb_prefer_flush_all {
                        // Only on single-CPU cases we can drop the page immediately before flushing.
                        drop(page);
                        continue;
                    }
                    self.flusher
                        .issue_tlb_flush_with(TlbFlushOp::Address(va), page);
                }
                PageTableItem::NotMapped { .. } => {
                    break;
                }
                PageTableItem::MappedUntracked { .. } => {
                    panic!("found untracked memory mapped into `VmSpace`");
                }
            }
        }

        if !self.flusher.need_remote_flush() && tlb_prefer_flush_all {
            self.flusher.issue_tlb_flush(TlbFlushOp::All);
        }

        self.flusher.dispatch_tlb_flush();
    }

    /// Applies the operation to the next slot of mapping within the range.
    ///
    /// The range to be found in is the current virtual address with the
    /// provided length.
    ///
    /// The function stops and yields the actually protected range if it has
    /// actually protected a page, no matter if the following pages are also
    /// required to be protected.
    ///
    /// It also makes the cursor moves forward to the next page after the
    /// protected one. If no mapped pages exist in the following range, the
    /// cursor will stop at the end of the range and return [`None`].
    ///
    /// Note that it will **NOT** flush the TLB after the operation. Please
    /// make the decision yourself on when and how to flush the TLB using
    /// [`Self::flusher`].
    ///
    /// # Panics
    ///
    /// This function will panic if:
    ///  - the range to be protected is out of the range where the cursor
    ///    is required to operate;
    ///  - the specified virtual address range only covers a part of a page.
    pub fn protect_next(
        &mut self,
        len: usize,
        mut op: impl FnMut(&mut PageProperty),
    ) -> Option<Range<Vaddr>> {
        // SAFETY: It is safe to protect memory in the userspace.
        unsafe { self.pt_cursor.protect_next(len, &mut op) }
    }

    /// Copies the mapping from the given cursor to the current cursor.
    ///
    /// All the mappings in the current cursor's range must be empty. The
    /// function allows the source cursor to operate on the mapping before
    /// the copy happens. So it is equivalent to protect then duplicate.
    /// Only the mapping is copied, the mapped pages are not copied.
    ///
    /// After the operation, both cursors will advance by the specified length.
    ///
    /// Note that it will **NOT** flush the TLB after the operation. Please
    /// make the decision yourself on when and how to flush the TLB using
    /// the source's [`CursorMut::flusher`].
    ///
    /// # Panics
    ///
    /// This function will panic if:
    ///  - either one of the range to be copied is out of the range where any
    ///    of the cursor is required to operate;
    ///  - either one of the specified virtual address ranges only covers a
    ///    part of a page.
    ///  - the current cursor's range contains mapped pages.
    pub fn copy_from(
        &mut self,
        src: &mut Self,
        len: usize,
        op: &mut impl FnMut(&mut PageProperty),
    ) {
        // SAFETY: Operations on user memory spaces are safe if it doesn't
        // involve dropping any pages.
        unsafe { self.pt_cursor.copy_from(&mut src.pt_cursor, len, op) }
    }
}

cpu_local_cell! {
    /// The `Arc` pointer to the activated VM space on this CPU. If the pointer
    /// is NULL, it means that the activated page table is merely the kernel
    /// page table.
    // TODO: If we are enabling ASID, we need to maintain the TLB state of each
    // CPU, rather than merely the activated `VmSpace`. When ASID is enabled,
    // the non-active `VmSpace`s can still have their TLB entries in the CPU!
    static ACTIVATED_VM_SPACE: *const VmSpace = core::ptr::null();
}

#[cfg(ktest)]
pub(crate) fn get_activated_vm_space() -> Option<*const VmSpace> {
    let ptr = ACTIVATED_VM_SPACE.load();
    if ptr.is_null() {
        None
    } else {
        // SAFETY: The pointer is only set to a valid `Arc` pointer.
        Some(ptr)
    }
}

/// The result of a query over the VM space.
#[derive(Debug)]
pub enum VmItem {
    /// The current slot is not mapped.
    NotMapped {
        /// The virtual address of the slot.
        va: Vaddr,
        /// The length of the slot.
        len: usize,
    },
    /// The current slot is mapped.
    Mapped {
        /// The virtual address of the slot.
        va: Vaddr,
        /// The mapped frame.
        frame: UFrame,
        /// The property of the slot.
        prop: PageProperty,
    },
}

impl PartialEq for VmItem {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            // The `len` varies, so we only compare `va`.
            (VmItem::NotMapped { va: va1, len: _ }, VmItem::NotMapped { va: va2, len: _ }) => {
                va1 == va2
            }
            (
                VmItem::Mapped {
                    va: va1,
                    frame: frame1,
                    prop: prop1,
                },
                VmItem::Mapped {
                    va: va2,
                    frame: frame2,
                    prop: prop2,
                },
            ) => va1 == va2 && frame1.start_paddr() == frame2.start_paddr() && prop1 == prop2,
            _ => false,
        }
    }
}

impl TryFrom<PageTableItem> for VmItem {
    type Error = &'static str;

    fn try_from(item: PageTableItem) -> core::result::Result<Self, Self::Error> {
        match item {
            PageTableItem::NotMapped { va, len } => Ok(VmItem::NotMapped { va, len }),
            PageTableItem::Mapped { va, page, prop } => Ok(VmItem::Mapped {
                va,
                frame: page
                    .try_into()
                    .map_err(|_| "found typed memory mapped into `VmSpace`")?,
                prop,
            }),
            PageTableItem::MappedUntracked { .. } => {
                Err("found untracked memory mapped into `VmSpace`")
            }
        }
    }
}
