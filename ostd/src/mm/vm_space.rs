// SPDX-License-Identifier: MPL-2.0

//! Virtual memory space management.
//!
//! The [`VmSpace`] struct is provided to manage the virtual memory space of a
//! user. Cursors are used to traverse and modify over the virtual memory space
//! concurrently. The VM space cursor [`self::Cursor`] is just a wrapper over
//! the page table cursor [`super::page_table::Cursor`], providing efficient,
//! powerful concurrent accesses to the page table, and suffers from the same
//! validity concerns as described in [`super::page_table::cursor`].

use alloc::collections::vec_deque::VecDeque;
use core::{
    ops::Range,
    sync::atomic::{AtomicPtr, Ordering},
};

use spin::Once;

use super::{
    io::Fallible,
    kspace::KERNEL_PAGE_TABLE,
    page::DynPage,
    page_table::{PageTable, UserMode},
    PageFlags, PageProperty, VmReader, VmWriter, PAGE_SIZE,
};
use crate::{
    arch::mm::{current_page_table_paddr, PageTableEntry, PagingConsts},
    cpu::{num_cpus, CpuExceptionInfo, CpuSet, PinCurrentCpu},
    cpu_local,
    mm::{
        page_table::{self, PageTableItem},
        Frame, MAX_USERSPACE_VADDR,
    },
    prelude::*,
    sync::{RwLock, RwLockReadGuard, SpinLock},
    task::disable_preempt,
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
    /// A CPU can only activate a `VmSpace` when no mutable cursors are alive.
    /// Cursors hold read locks and activation require a write lock.
    activation_lock: RwLock<()>,
}

impl VmSpace {
    /// Creates a new VM address space.
    pub fn new() -> Self {
        Self {
            pt: KERNEL_PAGE_TABLE.get().unwrap().create_user_page_table(),
            page_fault_handler: Once::new(),
            activation_lock: RwLock::new(()),
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

            let cur_cpu = pt_cursor.preempt_guard().current_cpu();

            let mut activated_cpus = CpuSet::new_empty();
            let mut need_self_flush = false;
            let mut need_remote_flush = false;

            for cpu in 0..num_cpus() {
                // The activation lock is held; other CPUs cannot activate this `VmSpace`.
                let ptr =
                    ACTIVATED_VM_SPACE.get_on_cpu(cpu).load(Ordering::Relaxed) as *const VmSpace;
                if ptr == self as *const VmSpace {
                    activated_cpus.add(cpu);
                    if cpu == cur_cpu {
                        need_self_flush = true;
                    } else {
                        need_remote_flush = true;
                    }
                }
            }

            CursorMut {
                pt_cursor,
                activation_lock,
                activated_cpus,
                need_remote_flush,
                need_self_flush,
            }
        })?)
    }

    /// Activates the page table on the current CPU.
    pub(crate) fn activate(self: &Arc<Self>) {
        let preempt_guard = disable_preempt();

        // Ensure no mutable cursors (which holds read locks) are alive.
        let _activation_lock = self.activation_lock.write();

        let cpu = preempt_guard.current_cpu();
        let activated_vm_space = ACTIVATED_VM_SPACE.get_on_cpu(cpu);

        let last_ptr = activated_vm_space.load(Ordering::Relaxed) as *const VmSpace;

        if last_ptr != Arc::as_ptr(self) {
            self.pt.activate();
            let ptr = Arc::into_raw(Arc::clone(self)) as *mut VmSpace;
            activated_vm_space.store(ptr, Ordering::Relaxed);
            if !last_ptr.is_null() {
                // SAFETY: The pointer is cast from an `Arc` when it's activated
                // the last time, so it can be restored and only restored once.
                drop(unsafe { Arc::from_raw(last_ptr) });
            }
        }
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
        let mut cursor = self.cursor_mut(&(0..end)).unwrap();
        let mut op = |prop: &mut PageProperty| {
            prop.flags -= PageFlags::W;
        };

        cursor.protect(end, &mut op);

        let page_fault_handler = {
            let new_handler = Once::new();
            if let Some(handler) = self.page_fault_handler.get() {
                new_handler.call_once(|| *handler);
            }
            new_handler
        };

        let CursorMut {
            pt_cursor,
            activation_lock,
            ..
        } = cursor;

        let new_pt = self.pt.clone_with(pt_cursor);

        // Release the activation lock after the page table is cloned to
        // prevent modification to the parent page table while cloning.
        drop(activation_lock);

        Self {
            pt: new_pt,
            page_fault_handler,
            activation_lock: RwLock::new(()),
        }
    }

    /// Creates a reader to read data from the user space of the current task.
    ///
    /// Returns `Err` if this `VmSpace` is not belonged to the user space of the current task
    /// or the `vaddr` and `len` do not represent a user space memory range.
    pub fn reader(&self, vaddr: Vaddr, len: usize) -> Result<VmReader<'_, Fallible>> {
        if current_page_table_paddr() != unsafe { self.pt.root_paddr() } {
            return Err(Error::AccessDenied);
        }

        if vaddr.checked_add(len).unwrap_or(usize::MAX) > MAX_USERSPACE_VADDR {
            return Err(Error::AccessDenied);
        }

        // `VmReader` is neither `Sync` nor `Send`, so it will not live longer than the current
        // task. This ensures that the correct page table is activated during the usage period of
        // the `VmReader`.
        //
        // SAFETY: The memory range is in user space, as checked above.
        Ok(unsafe { VmReader::<Fallible>::from_user_space(vaddr as *const u8, len) })
    }

    /// Creates a writer to write data into the user space.
    ///
    /// Returns `Err` if this `VmSpace` is not belonged to the user space of the current task
    /// or the `vaddr` and `len` do not represent a user space memory range.
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
    #[allow(dead_code)]
    activation_lock: RwLockReadGuard<'b, ()>,
    // Better to store them here since loading and counting them from the CPUs
    // list brings non-trivial overhead. We have a read lock so the stored set
    // is always a superset of actual activated CPUs.
    activated_cpus: CpuSet,
    need_remote_flush: bool,
    need_self_flush: bool,
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

    /// Map a frame into the current slot.
    ///
    /// This method will bring the cursor to the next slot after the modification.
    pub fn map(&mut self, frame: Frame, prop: PageProperty) {
        let start_va = self.virt_addr();
        // SAFETY: It is safe to map untyped memory into the userspace.
        let old = unsafe { self.pt_cursor.map(frame.into(), prop) };

        if old.is_some() {
            self.issue_tlb_flush(TlbFlushOp::Address(start_va), old);
            self.dispatch_tlb_flush();
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
    /// # Panics
    ///
    /// This method will panic if `len` is not page-aligned.
    pub fn unmap(&mut self, len: usize) {
        assert!(len % super::PAGE_SIZE == 0);
        let end_va = self.virt_addr() + len;
        let tlb_prefer_flush_all = len > TLB_FLUSH_ALL_THRESHOLD * PAGE_SIZE;

        loop {
            // SAFETY: It is safe to un-map memory in the userspace.
            let result = unsafe { self.pt_cursor.take_next(end_va - self.virt_addr()) };
            match result {
                PageTableItem::Mapped { va, page, .. } => {
                    if !self.need_remote_flush && tlb_prefer_flush_all {
                        // Only on single-CPU cases we can drop the page immediately before flushing.
                        drop(page);
                        continue;
                    }
                    self.issue_tlb_flush(TlbFlushOp::Address(va), Some(page));
                }
                PageTableItem::NotMapped { .. } => {
                    break;
                }
                PageTableItem::MappedUntracked { .. } => {
                    panic!("found untracked memory mapped into `VmSpace`");
                }
            }
        }

        if !self.need_remote_flush && tlb_prefer_flush_all {
            self.issue_tlb_flush(TlbFlushOp::All, None);
        }

        self.dispatch_tlb_flush();
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
        let end = self.virt_addr() + len;
        let tlb_prefer_flush_all = len > TLB_FLUSH_ALL_THRESHOLD * PAGE_SIZE;

        // SAFETY: It is safe to protect memory in the userspace.
        while let Some(range) =
            unsafe { self.pt_cursor.protect_next(end - self.virt_addr(), &mut op) }
        {
            if !tlb_prefer_flush_all {
                self.issue_tlb_flush(TlbFlushOp::Range(range), None);
            }
        }

        if tlb_prefer_flush_all {
            self.issue_tlb_flush(TlbFlushOp::All, None);
        }
        self.dispatch_tlb_flush();
    }

    fn issue_tlb_flush(&self, op: TlbFlushOp, drop_after_flush: Option<DynPage>) {
        let request = TlbFlushRequest {
            op,
            drop_after_flush,
        };

        // Fast path for single CPU cases.
        if !self.need_remote_flush {
            if self.need_self_flush {
                request.do_flush();
            }
            return;
        }

        // Slow path for multi-CPU cases.
        for cpu in self.activated_cpus.iter() {
            let mut queue = TLB_FLUSH_REQUESTS.get_on_cpu(cpu).lock();
            queue.push_back(request.clone());
        }
    }

    fn dispatch_tlb_flush(&self) {
        if !self.need_remote_flush {
            return;
        }

        fn do_remote_flush() {
            let preempt_guard = disable_preempt();
            let mut requests = TLB_FLUSH_REQUESTS
                .get_on_cpu(preempt_guard.current_cpu())
                .lock();
            if requests.len() > TLB_FLUSH_ALL_THRESHOLD {
                // TODO: in most cases, we need only to flush all the TLB entries
                // for an ASID if it is enabled.
                crate::arch::mm::tlb_flush_all_excluding_global();
                requests.clear();
            } else {
                while let Some(request) = requests.pop_front() {
                    request.do_flush();
                    if matches!(request.op, TlbFlushOp::All) {
                        requests.clear();
                        break;
                    }
                }
            }
        }

        crate::smp::inter_processor_call(&self.activated_cpus.clone(), do_remote_flush);
    }
}

/// The threshold used to determine whether we need to flush all TLB entries
/// when handling a bunch of TLB flush requests. If the number of requests
/// exceeds this threshold, the overhead incurred by flushing pages
/// individually would surpass the overhead of flushing all entries at once.
const TLB_FLUSH_ALL_THRESHOLD: usize = 32;

cpu_local! {
    /// The queue of pending requests.
    static TLB_FLUSH_REQUESTS: SpinLock<VecDeque<TlbFlushRequest>> = SpinLock::new(VecDeque::new());
    /// The `Arc` pointer to the activated VM space on this CPU. If the pointer
    /// is NULL, it means that the activated page table is merely the kernel
    /// page table.
    // TODO: If we are enabling ASID, we need to maintain the TLB state of each
    // CPU, rather than merely the activated `VmSpace`. When ASID is enabled,
    // the non-active `VmSpace`s can still have their TLB entries in the CPU!
    static ACTIVATED_VM_SPACE: AtomicPtr<VmSpace> = AtomicPtr::new(core::ptr::null_mut());
}

#[derive(Debug, Clone)]
struct TlbFlushRequest {
    op: TlbFlushOp,
    // If we need to remove a mapped page from the page table, we can only
    // recycle the page after all the relevant TLB entries in all CPUs are
    // flushed. Otherwise if the page is recycled for other purposes, the user
    // space program can still access the page through the TLB entries.
    #[allow(dead_code)]
    drop_after_flush: Option<DynPage>,
}

#[derive(Debug, Clone)]
enum TlbFlushOp {
    All,
    Address(Vaddr),
    Range(Range<Vaddr>),
}

impl TlbFlushRequest {
    /// Perform the TLB flush operation on the current CPU.
    fn do_flush(&self) {
        use crate::arch::mm::{
            tlb_flush_addr, tlb_flush_addr_range, tlb_flush_all_excluding_global,
        };
        match &self.op {
            TlbFlushOp::All => tlb_flush_all_excluding_global(),
            TlbFlushOp::Address(addr) => tlb_flush_addr(*addr),
            TlbFlushOp::Range(range) => tlb_flush_addr_range(range),
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
