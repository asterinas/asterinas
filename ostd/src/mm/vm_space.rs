// SPDX-License-Identifier: MPL-2.0

//! Virtual memory space management.
//!
//! The [`VmSpace`] struct is provided to manage the virtual memory space of a
//! user. Cursors are used to traverse and modify over the virtual memory space
//! concurrently. The VM space cursor [`self::Cursor`] is just a wrapper over
//! the page table cursor, providing efficient, powerful concurrent accesses
//! to the page table.

use core::{ops::Range, sync::atomic::Ordering};

use super::PAGE_SIZE;
use crate::{
    arch::mm::{current_page_table_paddr, PageTableEntry, PagingConsts},
    cpu::{AtomicCpuSet, CpuSet, PinCurrentCpu},
    cpu_local_cell,
    mm::{
        io::Fallible,
        kspace::KERNEL_PAGE_TABLE,
        page_size,
        page_table::{self, PageTable, PageTableConfig, PageTableFrag},
        tlb::{TlbFlushOp, TlbFlusher},
        AnyUFrameMeta, Frame, PageProperty, PagingConstsTrait, PagingLevel, UFrame, VmReader,
        VmWriter, MAX_USERSPACE_VADDR,
    },
    prelude::*,
    task::{atomic_mode::AsAtomicModeGuard, disable_preempt, DisabledPreemptGuard},
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
    pt: PageTable<UserPtConfig>,
    cpus: AtomicCpuSet,
}

impl VmSpace {
    /// Creates a new VM address space.
    pub fn new() -> Self {
        Self {
            pt: KERNEL_PAGE_TABLE.get().unwrap().create_user_page_table(),
            cpus: AtomicCpuSet::new(CpuSet::new_empty()),
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
    ) -> Result<Cursor<'a>> {
        Ok(self.pt.cursor(guard, va).map(Cursor)?)
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
    ) -> Result<CursorMut<'a>> {
        Ok(self.pt.cursor_mut(guard, va).map(|pt_cursor| CursorMut {
            pt_cursor,
            flusher: TlbFlusher::new(&self.cpus, disable_preempt()),
        })?)
    }

    /// Activates the page table on the current CPU.
    ///
    /// Return if it's a new activation.
    pub fn activate(self: &Arc<Self>) -> bool {
        let preempt_guard = disable_preempt();
        let cpu = preempt_guard.current_cpu();

        let last_ptr = ACTIVATED_VM_SPACE.load();

        if last_ptr == Arc::as_ptr(self) {
            return false;
        }

        // Record ourselves in the CPU set and the activated VM space pointer.
        // `Acquire` to ensure the modification to the PT is visible by this CPU.
        self.cpus.add(cpu, Ordering::Acquire);

        let self_ptr = Arc::into_raw(Arc::clone(self)) as *mut VmSpace;
        ACTIVATED_VM_SPACE.store(self_ptr);

        if !last_ptr.is_null() {
            // SAFETY: The pointer is cast from an `Arc` when it's activated
            // the last time, so it can be restored and only restored once.
            let last = unsafe { Arc::from_raw(last_ptr) };
            last.cpus.remove(cpu, Ordering::Relaxed);
        }

        self.pt.activate();

        true
    }

    /// Creates a reader to read data from the user space of the current task.
    ///
    /// Returns `Err` if this `VmSpace` is not belonged to the user space of the current task
    /// or the `vaddr` and `len` do not represent a user space memory range.
    ///
    /// Users must ensure that no other page table is activated in the current task during the
    /// lifetime of the created `VmReader`. This guarantees that the `VmReader` can operate correctly.
    pub fn reader(&self, vaddr: Vaddr, len: usize) -> Result<VmReader<'_, Fallible>> {
        if current_page_table_paddr() != self.pt.root_paddr() {
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
        if current_page_table_paddr() != self.pt.root_paddr() {
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

impl Default for VmSpace {
    fn default() -> Self {
        Self::new()
    }
}

/// The cursor for querying over the VM space without modifying it.
///
/// It exclusively owns a sub-tree of the page table, preventing others from
/// reading or modifying the same sub-tree. Two read-only cursors can not be
/// created from the same virtual address range either.
pub struct Cursor<'a>(page_table::Cursor<'a, UserPtConfig>);

impl Iterator for Cursor<'_> {
    type Item = (Range<Vaddr>, Option<VmItem>);

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next()
    }
}

impl Cursor<'_> {
    /// Queries the mapping at the current virtual address.
    ///
    /// If the cursor is pointing to a valid virtual address that is locked,
    /// it will return the virtual address range and the mapped item.
    pub fn query(&mut self) -> Result<(Range<Vaddr>, Option<VmItem>)> {
        Ok(self.0.query()?)
    }

    /// Moves the cursor forward to the next mapped virtual address.
    ///
    /// If there is mapped virtual address following the current address within
    /// next `len` bytes, it will return that mapped address. In this case,
    /// the cursor will stop at the mapped address.
    ///
    /// Otherwise, it will return `None`. And the cursor may stop at any
    /// address after `len` bytes.
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
}

/// The cursor for modifying the mappings in VM space.
///
/// It exclusively owns a sub-tree of the page table, preventing others from
/// reading or modifying the same sub-tree.
pub struct CursorMut<'a> {
    pt_cursor: page_table::CursorMut<'a, UserPtConfig>,
    // We have a read lock so the CPU set in the flusher is always a superset
    // of actual activated CPUs.
    flusher: TlbFlusher<'a, DisabledPreemptGuard>,
}

impl<'a> CursorMut<'a> {
    /// Queries the mapping at the current virtual address.
    ///
    /// This is the same as [`Cursor::query`].
    ///
    /// If the cursor is pointing to a valid virtual address that is locked,
    /// it will return the virtual address range and the mapped item.
    pub fn query(&mut self) -> Result<(Range<Vaddr>, Option<VmItem>)> {
        Ok(self.pt_cursor.query()?)
    }

    /// Moves the cursor forward to the next mapped virtual address.
    ///
    /// This is the same as [`Cursor::find_next`].
    pub fn find_next(&mut self, len: usize) -> Option<Vaddr> {
        self.pt_cursor.find_next(len)
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
    pub fn flusher(&mut self) -> &mut TlbFlusher<'a, DisabledPreemptGuard> {
        &mut self.flusher
    }

    /// Map a frame into the current slot.
    ///
    /// This method will bring the cursor to the next slot after the modification.
    pub fn map(&mut self, item: VmItem) {
        let start_va = self.virt_addr();

        // SAFETY: It is safe to map untyped memory into the userspace.
        let Err(frag) = (unsafe { self.pt_cursor.map(item) }) else {
            return; // No mapping exists at the current address.
        };

        match frag {
            PageTableFrag::Mapped { va, item } => {
                debug_assert_eq!(va, start_va);
                let VmItem::Frame(old_frame, _) = item else {
                    return;
                };
                self.flusher
                    .issue_tlb_flush_with(TlbFlushOp::Address(start_va), old_frame.into());
                self.flusher.dispatch_tlb_flush();
            }
            PageTableFrag::StrayPageTable { .. } => {
                panic!("`UFrame` is base page sized but re-mapping out a child PT");
            }
        }
    }

    /// Clears the mapping starting from the current slot, and returns the
    /// number of unmapped pages.
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
    /// Panics if:
    ///  - the length is longer than the remaining range of the cursor;
    ///  - the length is not page-aligned.
    pub fn unmap(&mut self, len: usize) -> usize {
        let end_va = self.virt_addr() + len;
        let mut num_unmapped: usize = 0;
        loop {
            // SAFETY: It is safe to un-map memory in the userspace.
            let Some(frag) = (unsafe { self.pt_cursor.take_next(end_va - self.virt_addr()) })
            else {
                break; // No more mappings in the range.
            };

            match frag {
                PageTableFrag::Mapped { va, item, .. } => {
                    let VmItem::Frame(frame, _) = item else {
                        continue;
                    };
                    num_unmapped += 1;
                    #[cfg(not(feature = "lazy_tlb_flush_on_unmap"))]
                    self.flusher
                        .issue_tlb_flush_with(TlbFlushOp::Address(va), frame.into());
                    #[cfg(feature = "lazy_tlb_flush_on_unmap")]
                    self.flusher
                        .latr_with(TlbFlushOp::Address(va), frame.into());
                }
                PageTableFrag::StrayPageTable {
                    pt,
                    va,
                    len,
                    num_frames,
                } => {
                    num_unmapped += num_frames;
                    #[cfg(not(feature = "lazy_tlb_flush_on_unmap"))]
                    self.flusher
                        .issue_tlb_flush_with(TlbFlushOp::Range(va..va + len), pt);
                    #[cfg(feature = "lazy_tlb_flush_on_unmap")]
                    self.flusher.latr_with(TlbFlushOp::Range(va..va + len), pt);
                }
            }
        }

        #[cfg(not(feature = "lazy_tlb_flush_on_unmap"))]
        self.flusher.dispatch_tlb_flush();

        num_unmapped
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
    /// Panics if the length is longer than the remaining range of the cursor.
    pub fn protect_next(
        &mut self,
        len: usize,
        prot_op: &mut impl FnMut(&mut PageProperty),
        status_op: &mut impl FnMut(&mut Status),
    ) -> Option<Range<Vaddr>> {
        // SAFETY: It is safe to protect memory in the userspace.
        unsafe { self.pt_cursor.protect_next(len, prot_op, status_op) }
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
pub(super) fn get_activated_vm_space() -> *const VmSpace {
    ACTIVATED_VM_SPACE.load()
}

/// A status that can be used to mark a slot in the VM space.
///
/// The status can be converted to and from a [`usize`] value. Available status
/// are non-zero and capped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Status(usize);

impl Status {
    /// The mask that marks the available bits in a status.
    const MASK: usize = ((1 << 39) - 1) / PAGE_SIZE;

    pub(crate) fn into_raw_inner(self) -> usize {
        debug_assert!(self.0 & !Self::MASK == 0);
        debug_assert!(self.0 != 0);
        self.0
    }

    /// Creates a new status from a raw value.
    ///
    /// # Safety
    ///
    /// The raw value must be a valid status created by [`Self::into_raw_inner`].
    pub(crate) unsafe fn from_raw_inner(raw: usize) -> Self {
        debug_assert!(raw & !Self::MASK == 0);
        debug_assert!(raw != 0);
        Self(raw)
    }
}

impl TryFrom<usize> for Status {
    type Error = ();

    fn try_from(value: usize) -> core::result::Result<Self, Self::Error> {
        if (value & !Self::MASK == 0) && value != 0 {
            Ok(Self(value * PAGE_SIZE))
        } else {
            Err(())
        }
    }
}

impl From<Status> for usize {
    fn from(status: Status) -> usize {
        status.0 / PAGE_SIZE
    }
}

/// The item that can be mapped into the [`VmSpace`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VmItem {
    /// Actually mapped a physical frame into the VM space.
    Frame(UFrame, PageProperty),
    /// Marked with a [`Status`], without actually mapping a physical frame.
    Status(Status, PagingLevel),
}

/// Return largest pages.
pub fn largest_pages(
    mut va: Vaddr,
    mut len: usize,
    status: Status,
) -> impl Iterator<Item = VmItem> {
    assert_eq!(va % PAGE_SIZE, 0);
    assert_eq!(len % PAGE_SIZE, 0);

    core::iter::from_fn(move || {
        if len == 0 {
            return None;
        }

        let mut level = UserPtConfig::NR_LEVELS;
        while page_size::<UserPtConfig>(level) > len || va % page_size::<UserPtConfig>(level) != 0 {
            level -= 1;
        }

        va += page_size::<UserPtConfig>(level);
        len -= page_size::<UserPtConfig>(level);

        Some(VmItem::Status(status, level))
    })
}

#[derive(Clone, Debug)]
pub(crate) struct UserPtConfig {}

// SAFETY: `item_into_raw` and `item_from_raw` are implemented correctly,
unsafe impl PageTableConfig for UserPtConfig {
    const TOP_LEVEL_INDEX_RANGE: Range<usize> = 0..256;

    type E = PageTableEntry;
    type C = PagingConsts;

    type Item = VmItem;

    fn item_into_raw(item: Self::Item) -> (Paddr, PagingLevel, PageProperty) {
        match item {
            VmItem::Frame(frame, prop) => {
                let level = frame.map_level();
                let paddr = frame.into_raw();
                (paddr, level, prop)
            }
            VmItem::Status(status, level) => {
                let raw_inner = status.into_raw_inner();
                (raw_inner as Paddr, level, PageProperty::new_absent())
            }
        }
    }

    unsafe fn item_from_raw(paddr: Paddr, level: PagingLevel, prop: PageProperty) -> Self::Item {
        if prop.has_map {
            debug_assert_eq!(level, 1);
            // SAFETY: The caller ensures safety.
            let frame = unsafe { Frame::<dyn AnyUFrameMeta>::from_raw(paddr) };
            VmItem::Frame(frame, prop)
        } else {
            // SAFETY: The caller ensures safety.
            let status = unsafe { Status::from_raw_inner(paddr) };
            VmItem::Status(status, level)
        }
    }
}
