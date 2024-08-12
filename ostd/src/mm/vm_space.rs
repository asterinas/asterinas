// SPDX-License-Identifier: MPL-2.0

//! Virtual memory space management.
//!
//! The [`VmSpace`] struct is provided to manage the virtual memory space of a
//! user. Cursors are used to traverse and modify over the virtual memory space
//! concurrently. The VM space cursor [`self::Cursor`] is just a wrapper over
//! the page table cursor [`super::page_table::Cursor`], providing efficient,
//! powerful concurrent accesses to the page table, and suffers from the same
//! validity concerns as described in [`super::page_table::cursor`].

use core::ops::Range;

use spin::Once;

use super::{
    io::UserSpace,
    kspace::KERNEL_PAGE_TABLE,
    page_table::{PageTable, UserMode},
    PageFlags, PageProperty, VmReader, VmWriter,
};
use crate::{
    arch::mm::{
        current_page_table_paddr, tlb_flush_addr, tlb_flush_addr_range, PageTableEntry,
        PagingConsts,
    },
    cpu::CpuExceptionInfo,
    mm::{
        page_table::{self, PageTableItem},
        Frame, MAX_USERSPACE_VADDR,
    },
    prelude::*,
    Error,
};

/// Virtual memory space.
///
/// A virtual memory space (`VmSpace`) can be created and assigned to a user
/// space so that the virtual memory of the user space can be manipulated
/// safely. For example,  given an arbitrary user-space pointer, one can read
/// and write the memory location referred to by the user-space pointer without
/// the risk of breaking the memory safety of the kernel space.
///
/// A newly-created `VmSpace` is not backed by any physical memory pages. To
/// provide memory pages for a `VmSpace`, one can allocate and map physical
/// memory ([`Frame`]s) to the `VmSpace` using the cursor.
///
/// A `VmSpace` can also attach a page fault handler, which will be invoked to
/// handle page faults generated from user space.
#[allow(clippy::type_complexity)]
#[derive(Debug)]
pub struct VmSpace {
    pt: PageTable<UserMode>,
    page_fault_handler: Once<fn(&VmSpace, &CpuExceptionInfo) -> core::result::Result<(), ()>>,
}

// Notes on TLB flushing:
//
// We currently assume that:
// 1. `VmSpace` _might_ be activated on the current CPU and the user memory _might_ be used
//    immediately after we make changes to the page table entries. So we must invalidate the
//    corresponding TLB caches accordingly.
// 2. `VmSpace` must _not_ be activated on another CPU. This assumption is trivial, since SMP
//    support is not yet available. But we need to consider this situation in the future (TODO).
impl VmSpace {
    /// Creates a new VM address space.
    pub fn new() -> Self {
        Self {
            pt: KERNEL_PAGE_TABLE.get().unwrap().create_user_page_table(),
            page_fault_handler: Once::new(),
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
    /// cursor may also block or be overriden the mapping of another cursor.
    pub fn cursor_mut(&self, va: &Range<Vaddr>) -> Result<CursorMut<'_>> {
        Ok(self.pt.cursor_mut(va).map(CursorMut)?)
    }

    /// Activates the page table.
    pub(crate) fn activate(&self) {
        self.pt.activate();
    }

    pub(crate) fn handle_page_fault(
        &self,
        info: &CpuExceptionInfo,
    ) -> core::result::Result<(), ()> {
        if let Some(func) = self.page_fault_handler.get() {
            return func(self, info);
        }
        Err(())
    }

    /// Registers the page fault handler in this `VmSpace`.
    ///
    /// The page fault handler of a `VmSpace` can only be initialized once.
    /// If it has been initialized before, calling this method will have no effect.
    pub fn register_page_fault_handler(
        &self,
        func: fn(&VmSpace, &CpuExceptionInfo) -> core::result::Result<(), ()>,
    ) {
        self.page_fault_handler.call_once(|| func);
    }

    /// Forks a new VM space with copy-on-write semantics.
    ///
    /// Both the parent and the newly forked VM space will be marked as
    /// read-only. And both the VM space will take handles to the same
    /// physical memory pages.
    pub fn fork_copy_on_write(&self) -> Self {
        // Protect the parent VM space as read-only.
        let end = MAX_USERSPACE_VADDR;
        let mut cursor = self.pt.cursor_mut(&(0..end)).unwrap();
        let mut op = |prop: &mut PageProperty| {
            prop.flags -= PageFlags::W;
        };

        // SAFETY: It is safe to protect memory in the userspace.
        while let Some(range) = unsafe { cursor.protect_next(end - cursor.virt_addr(), &mut op) } {
            tlb_flush_addr(range.start);
        }

        let page_fault_handler = {
            let new_handler = Once::new();
            if let Some(handler) = self.page_fault_handler.get() {
                new_handler.call_once(|| *handler);
            }
            new_handler
        };

        Self {
            pt: self.pt.clone_with(cursor),
            page_fault_handler,
        }
    }

    /// Creates a reader to read data from the user space of the current task.
    ///
    /// Returns `Err` if this `VmSpace` is not belonged to the user space of the current task
    /// or the `vaddr` and `len` do not represent a user space memory range.
    pub fn reader(&self, vaddr: Vaddr, len: usize) -> Result<VmReader<'_, UserSpace>> {
        if current_page_table_paddr() != unsafe { self.pt.root_paddr() } {
            return Err(Error::AccessDenied);
        }

        if vaddr.checked_add(len).unwrap_or(usize::MAX) > MAX_USERSPACE_VADDR {
            return Err(Error::AccessDenied);
        }

        // SAFETY: As long as the current task owns user space, the page table of
        // the current task will be activated during the execution of the current task.
        // Since `VmReader` is neither `Sync` nor `Send`, it will not live longer than
        // the current task. Hence, it is ensured that the correct page table
        // is activated during the usage period of the `VmReader`.
        Ok(unsafe { VmReader::<UserSpace>::from_user_space(vaddr as *const u8, len) })
    }

    /// Creates a writer to write data into the user space.
    ///
    /// Returns `Err` if this `VmSpace` is not belonged to the user space of the current task
    /// or the `vaddr` and `len` do not represent a user space memory range.
    pub fn writer(&self, vaddr: Vaddr, len: usize) -> Result<VmWriter<'_, UserSpace>> {
        if current_page_table_paddr() != unsafe { self.pt.root_paddr() } {
            return Err(Error::AccessDenied);
        }

        if vaddr.checked_add(len).unwrap_or(usize::MAX) > MAX_USERSPACE_VADDR {
            return Err(Error::AccessDenied);
        }

        // SAFETY: As long as the current task owns user space, the page table of
        // the current task will be activated during the execution of the current task.
        // Since `VmWriter` is neither `Sync` nor `Send`, it will not live longer than
        // the current task. Hence, it is ensured that the correct page table
        // is activated during the usage period of the `VmWriter`.
        Ok(unsafe { VmWriter::<UserSpace>::from_user_space(vaddr as *mut u8, len) })
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
    pub fn jump(&mut self, va: Vaddr) {
        self.0.jump(va);
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
pub struct CursorMut<'a>(page_table::CursorMut<'a, UserMode, PageTableEntry, PagingConsts>);

impl CursorMut<'_> {
    /// Query about the current slot.
    ///
    /// This is the same as [`Cursor::query`].
    ///
    /// This function won't bring the cursor to the next slot.
    pub fn query(&mut self) -> Result<VmItem> {
        Ok(self.0.query().map(|item| item.try_into().unwrap())?)
    }

    /// Jump to the virtual address.
    ///
    /// This is the same as [`Cursor::jump`].
    pub fn jump(&mut self, va: Vaddr) {
        self.0.jump(va);
    }

    /// Get the virtual address of the current slot.
    pub fn virt_addr(&self) -> Vaddr {
        self.0.virt_addr()
    }

    /// Map a frame into the current slot.
    ///
    /// This method will bring the cursor to the next slot after the modification.
    pub fn map(&mut self, frame: Frame, prop: PageProperty) {
        let start_va = self.virt_addr();
        let end_va = start_va + frame.size();

        // SAFETY: It is safe to map untyped memory into the userspace.
        unsafe {
            self.0.map(frame.into(), prop);
        }

        tlb_flush_addr_range(&(start_va..end_va));
    }

    /// Clear the mapping starting from the current slot.
    ///
    /// This method will bring the cursor forward by `len` bytes in the virtual
    /// address space after the modification.
    ///
    /// Already-absent mappings encountered by the cursor will be skipped. It
    /// is valid to unmap a range that is not mapped.
    ///
    /// # Panics
    ///
    /// This method will panic if `len` is not page-aligned.
    pub fn unmap(&mut self, len: usize) {
        assert!(len % super::PAGE_SIZE == 0);
        let end_va = self.virt_addr() + len;

        loop {
            // SAFETY: It is safe to un-map memory in the userspace.
            let result = unsafe { self.0.take_next(end_va - self.virt_addr()) };
            match result {
                PageTableItem::Mapped { va, page, .. } => {
                    // TODO: Ask other processors to flush the TLB before we
                    // release the page back to the allocator.
                    tlb_flush_addr(va);
                    drop(page);
                }
                PageTableItem::NotMapped { .. } => {
                    break;
                }
                PageTableItem::MappedUntracked { .. } => {
                    panic!("found untracked memory mapped into `VmSpace`");
                }
            }
        }
    }

    /// Change the mapping property starting from the current slot.
    ///
    /// This method will bring the cursor forward by `len` bytes in the virtual
    /// address space after the modification.
    ///
    /// The way to change the property is specified by the closure `op`.
    ///
    /// # Panics
    ///
    /// This method will panic if `len` is not page-aligned.
    pub fn protect(&mut self, len: usize, mut op: impl FnMut(&mut PageProperty)) {
        assert!(len % super::PAGE_SIZE == 0);
        let end = self.0.virt_addr() + len;

        // SAFETY: It is safe to protect memory in the userspace.
        while let Some(range) = unsafe { self.0.protect_next(end - self.0.virt_addr(), &mut op) } {
            tlb_flush_addr(range.start);
        }
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
        frame: Frame,
        /// The property of the slot.
        prop: PageProperty,
    },
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
