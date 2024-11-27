// SPDX-License-Identifier: MPL-2.0

//! TLB flush operations.

use alloc::vec::Vec;
use core::{
    ops::Range,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

use super::{page::DynPage, Vaddr, PAGE_SIZE};
use crate::{
    cpu::{all_cpus, AtomicCpuSet, CpuId, CpuSet, PinCurrentCpu},
    cpu_local,
    smp::inter_processor_call,
    sync::SpinLock,
    trap::{self, DisabledLocalIrqGuard},
};

/// A TLB flusher that is aware of which CPUs are needed to be flushed.
///
/// The flusher needs to stick to the current CPU.
pub struct TlbFlusher<'c> {
    target_cpus: &'c AtomicCpuSet,
    irq_guard: DisabledLocalIrqGuard,
    need_flush_all: bool,
    flush_ops: Vec<TlbFlushOp>,
    defer_pages: Vec<(TlbFlushOp, DynPage)>,
}

impl<'c> TlbFlusher<'c> {
    /// Creates a new TLB flusher with the specified CPUs to be flushed.
    ///
    /// The flusher needs to stick to the current CPU. So please provide a
    /// guard that implements [`PinCurrentCpu`].
    pub fn new(target_cpus: &'c AtomicCpuSet) -> Self {
        Self {
            target_cpus,
            irq_guard: trap::disable_local(),
            need_flush_all: false,
            flush_ops: Vec::new(),
            defer_pages: Vec::new(),
        }
    }

    /// Issues a pending TLB flush request.
    ///
    /// On SMP systems, the notification is sent to all the relevant CPUs only
    /// when the remote buffer is full. Otherwise, this is non-blocking.
    pub fn issue_tlb_flush(&mut self, op: TlbFlushOp) {
        if self.need_flush_all {
            return;
        }

        let op = op.optimize_for_large_range();

        if op == TlbFlushOp::All || self.flush_ops.len() >= FLUSH_ALL_OPS_THRESHOLD {
            self.flush_ops.clear();
            self.need_flush_all = true;
        } else {
            self.flush_ops.push(op);
        }
    }

    /// Issues a TLB flush request that must happen before dropping the page.
    ///
    /// If we need to remove a mapped page from the page table, we can only
    /// recycle the page after all the relevant TLB entries in all CPUs are
    /// flushed. Otherwise if the page is recycled for other purposes, the user
    /// space program can still access the page through the TLB entries. This
    /// method is designed to be used in such cases.
    pub fn issue_tlb_flush_with(&mut self, op: TlbFlushOp, drop_after_flush: DynPage) {
        self.defer_pages.push((op, drop_after_flush));
    }

    fn dispatch_tlb_flush(&mut self) {
        let mut target_cpus = self.target_cpus.load();
        let this_cpu = self.irq_guard.current_cpu();

        let need_self_flush = target_cpus.contains(this_cpu);

        if need_self_flush {
            target_cpus.remove(this_cpu);
        }

        let target_cpu_size = target_cpus.count();

        let need_remote_flush = target_cpu_size > 1;

        if need_self_flush {
            if self.need_flush_all {
                TlbFlushOp::All.perform_on_current();
            } else {
                for op in &self.flush_ops {
                    op.perform_on_current();
                }
                for (op, _) in &self.defer_pages {
                    op.perform_on_current();
                }
            }
        }

        if need_remote_flush {
            let mut ops = Vec::new();
            core::mem::swap(&mut self.flush_ops, &mut ops);
            let mut defers = Vec::new();
            core::mem::swap(&mut self.defer_pages, &mut defers);
            PUBLIC_FLUSH_OPS
                .get_on_cpu(this_cpu)
                .add(ops, target_cpus.clone());
            PUBLIC_DEFER_PAGES
                .get_on_cpu(this_cpu)
                .add(defers, target_cpus);
        }
    }
}

impl Drop for TlbFlusher<'_> {
    fn drop(&mut self) {
        self.dispatch_tlb_flush();
    }
}

/// The operation to flush TLB entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TlbFlushOp {
    /// Flush all TLB entries except for the global entries.
    All,
    /// Flush the TLB entry for the specified virtual address.
    Address(Vaddr),
    /// Flush the TLB entries for the specified virtual address range.
    Range(Range<Vaddr>),
}

impl TlbFlushOp {
    /// Performs the TLB flush operation on the current CPU.
    pub fn perform_on_current(&self) {
        use crate::arch::mm::{
            tlb_flush_addr, tlb_flush_addr_range, tlb_flush_all_excluding_global,
        };
        match self {
            TlbFlushOp::All => tlb_flush_all_excluding_global(),
            TlbFlushOp::Address(addr) => tlb_flush_addr(*addr),
            TlbFlushOp::Range(range) => tlb_flush_addr_range(range),
        }
    }

    fn optimize_for_large_range(self) -> Self {
        match self {
            TlbFlushOp::Range(range) => {
                if range.len() > FLUSH_ALL_RANGE_THRESHOLD {
                    TlbFlushOp::All
                } else {
                    TlbFlushOp::Range(range)
                }
            }
            _ => self,
        }
    }
}

/// If a TLB flushing request exceeds this threshold, we flush all.
const FLUSH_ALL_RANGE_THRESHOLD: usize = 32 * PAGE_SIZE;

/// If the number of pending requests exceeds this threshold, we flush all the
/// TLB entries instead of flushing them one by one.
const FLUSH_ALL_OPS_THRESHOLD: usize = 32;

// The queues of pending requests publicly seen on each CPU.
//
// On scheduler ticks or some timer interrupts, we will process the pending
// requests on all CPUs and recycle the pages on the current CPU.
cpu_local! {
    static PUBLIC_FLUSH_OPS: OpsArray = OpsArray::new();
    static PUBLIC_DEFER_PAGES: DeferPagesArray = DeferPagesArray::new();
}

cpu_local! {
    static COHERENT: AtomicBool = AtomicBool::new(false);
}

/// Recycle the local pages that is delayed to be recycled.
///
/// This function checks if all the issued TLB flush requests of local pages
/// are processed on all the relevant CPUs. If so, the page can be recycled.
pub(crate) fn delayed_recycle_pages() {
    let irq_guard = trap::disable_local();
    let cur_cpu = irq_guard.current_cpu();
    PUBLIC_FLUSH_OPS.get_on_cpu(cur_cpu).recycle();
    PUBLIC_DEFER_PAGES.get_on_cpu(cur_cpu).recycle();
    COHERENT.get_on_cpu(cur_cpu).store(true, Ordering::Relaxed);
}

/// Process the pending TLB flush requests on all the CPUs.
///
/// This function checks if there are any pending TLB flush requests on all the
/// remote CPUS. If so, it will process the requests.
pub(crate) fn process_pending_shootdowns() {
    let irq_guard = trap::disable_local();
    let cur_cpu = irq_guard.current_cpu();
    for cpu_id in all_cpus() {
        if cpu_id == cur_cpu {
            continue;
        }
        PUBLIC_FLUSH_OPS
            .get_on_cpu(cpu_id)
            .process_remote_requests(cur_cpu);
        PUBLIC_DEFER_PAGES
            .get_on_cpu(cpu_id)
            .process_remote_requests(cur_cpu);
    }
}

struct OpsArray {
    ops: [SpinLock<Option<(TlbFlushOp, CpuSet)>>; FLUSH_ALL_OPS_THRESHOLD],
    size: AtomicUsize,
    pending_flush_all: SpinLock<Option<CpuSet>>,
}

impl OpsArray {
    const fn new() -> Self {
        Self {
            ops: [const { SpinLock::new(None) }; FLUSH_ALL_OPS_THRESHOLD],
            size: AtomicUsize::new(0),
            pending_flush_all: SpinLock::new(None),
        }
    }

    /// Recycle the operations that can be recycled.
    ///
    /// This should be called by the current CPU.
    fn recycle(&self) {
        let size = self.size.load(Ordering::Relaxed);
        if size == 0 {
            return;
        }
        for i in 0..FLUSH_ALL_OPS_THRESHOLD {
            let mut lock = self.ops[i].lock();
            if let Some((_, target_cpus)) = &*lock {
                if target_cpus.is_empty() {
                    *lock = None;
                    self.size.fetch_sub(1, Ordering::Relaxed);
                }
            }
        }
    }

    /// Adds TLB flush operations to the array.
    ///
    /// This should be called by the current CPU.
    fn add(&self, mut ops: Vec<TlbFlushOp>, target_cpus: CpuSet) {
        let size = self.size.load(Ordering::Relaxed);
        if size + ops.len() >= FLUSH_ALL_OPS_THRESHOLD {
            self.add_flush_all(&target_cpus);
            return;
        }

        // Find an empty slot to store the operation.
        for i in 0..FLUSH_ALL_OPS_THRESHOLD {
            let mut lock = self.ops[i].lock();
            if lock.is_none() {
                let Some(op) = ops.pop() else {
                    return;
                };
                *lock = Some((op, target_cpus.clone()));
                self.size.fetch_add(1, Ordering::Relaxed);
                if ops.is_empty() {
                    return;
                }
            }
        }
        // Somebody filled the array while we were trying to add an element.
        self.add_flush_all(&target_cpus);
    }

    /// Check the remote CPU's requests and process them.
    ///
    /// This should be called by the other CPUs.
    fn process_remote_requests(&self, current: CpuId) {
        let mut flushed_all = false;
        if self.flush_all_contains(current) {
            TlbFlushOp::All.perform_on_current();
            flushed_all = true;
        }
        for i in 0..FLUSH_ALL_OPS_THRESHOLD {
            let mut lock = self.ops[i].lock();
            if let Some((op, target_cpus)) = &mut *lock {
                if target_cpus.contains(current) {
                    if !flushed_all {
                        op.perform_on_current();
                    }
                    target_cpus.remove(current);
                    if target_cpus.is_empty() {
                        *lock = None;
                        self.size.fetch_sub(1, Ordering::Relaxed);
                    }
                }
            }
        }
    }

    fn add_flush_all(&self, target_cpus: &CpuSet) {
        let mut lock = self.pending_flush_all.lock();
        if let Some(cpus) = &mut *lock {
            cpus.add_set(target_cpus);
        } else {
            *lock = Some(target_cpus.clone());
        }
    }

    fn flush_all_contains(&self, current: CpuId) -> bool {
        self.pending_flush_all
            .lock()
            .as_ref()
            .map(|target_cpus| target_cpus.contains(current))
            .unwrap_or(false)
    }
}

/// If the number of pending pages exceeds this threshold, we need to IPI all
/// the relevant CPUs to flush the TLB entries.
const IPI_RECYCLE_THRESHOLD: usize = 64;

struct DeferPagesArray {
    pages: [SpinLock<Option<(TlbFlushOp, DynPage, CpuSet)>>; IPI_RECYCLE_THRESHOLD],
    size: AtomicUsize,
}

impl DeferPagesArray {
    const fn new() -> Self {
        Self {
            pages: [const { SpinLock::new(None) }; IPI_RECYCLE_THRESHOLD],
            size: AtomicUsize::new(0),
        }
    }

    /// Recycle the pages that can be recycled.
    ///
    /// This should be called by the current CPU.
    fn recycle(&self) {
        let size = self.size.load(Ordering::Relaxed);
        if size == 0 {
            return;
        }
        for i in 0..IPI_RECYCLE_THRESHOLD {
            let mut lock = self.pages[i].lock();
            if let Some((_op, _page, target_cpus)) = &*lock {
                if target_cpus.is_empty() {
                    *lock = None;
                    self.size.fetch_sub(1, Ordering::Relaxed);
                }
            }
        }
    }

    /// Adds the pages to the array.
    ///
    /// This should be called by the current CPU.
    fn add(&self, mut defers: Vec<(TlbFlushOp, DynPage)>, target_cpus: CpuSet) {
        while defers.is_empty() {
            let mut relevant_cpus = target_cpus.clone();
            for i in 0..IPI_RECYCLE_THRESHOLD {
                let mut lock = self.pages[i].lock();
                if let Some((_, _, target_cpus)) = &*lock {
                    relevant_cpus.add_set(target_cpus);
                } else {
                    let Some((op, page)) = defers.pop() else {
                        return;
                    };
                    *lock = Some((op, page, target_cpus.clone()));
                    self.size.fetch_add(1, Ordering::Relaxed);
                    if defers.is_empty() {
                        return;
                    }
                }
            }
            // Does not have enough slots to store all the defers.
            // Notify all the relevant CPUs to flush the TLB entries.
            for cpu_id in relevant_cpus.iter() {
                COHERENT.get_on_cpu(cpu_id).store(false, Ordering::Release);
            }
            inter_processor_call(&relevant_cpus, || {
                process_pending_shootdowns();
                delayed_recycle_pages();
            });
            // Wait for ACKs from all the relevant CPUs.
            for cpu_id in relevant_cpus.iter() {
                while !COHERENT.get_on_cpu(cpu_id).load(Ordering::Acquire) {
                    // We disabled interrupts, so we should try recycle when we are waiting.
                    process_pending_shootdowns();
                    delayed_recycle_pages();
                    // Wait for the remote CPU to finish the TLB flush.
                    core::hint::spin_loop();
                }
            }
        }
    }

    /// Check the remote CPU's requests and process them.
    ///
    /// This should be called by the other CPUs.
    fn process_remote_requests(&self, current: CpuId) {
        let size = self.size.load(Ordering::Relaxed);
        if size == 0 {
            return;
        }
        let prefer_flush_all = size >= FLUSH_ALL_OPS_THRESHOLD;
        let mut flushed_all = false;
        for i in 0..IPI_RECYCLE_THRESHOLD {
            let mut lock = self.pages[i].lock();
            if let Some((op, _page, target_cpus)) = &mut *lock {
                if target_cpus.contains(current) {
                    if prefer_flush_all && !flushed_all {
                        TlbFlushOp::All.perform_on_current();
                        flushed_all = true;
                    } else {
                        op.perform_on_current();
                    }
                    target_cpus.remove(current);
                }
            }
        }
    }
}
