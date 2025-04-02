// SPDX-License-Identifier: MPL-2.0

//! TLB flush operations.

#[cfg(feature = "lazy_tlb_flush_on_unmap")]
pub(crate) mod latr;

use alloc::vec::Vec;
use core::{
    ops::Range,
    sync::atomic::{AtomicBool, Ordering},
};

use super::{
    frame::{meta::AnyFrameMeta, Frame},
    Vaddr, PAGE_SIZE,
};
use crate::{
    arch::irq,
    cpu::{AtomicCpuSet, CpuSet, PinCurrentCpu},
    cpu_local,
    sync::{LocalIrqDisabled, SpinLock},
};

/// A TLB flusher that is aware of which CPUs are needed to be flushed.
///
/// The flusher needs to stick to the current CPU.
pub struct TlbFlusher<'a, G: PinCurrentCpu> {
    target_cpus: &'a AtomicCpuSet,
    have_unsynced_flush: CpuSet,
    ops_stack: OpsStack,
    _pin_current: G,
}

impl<'a, G: PinCurrentCpu> TlbFlusher<'a, G> {
    /// Creates a new TLB flusher with the specified CPUs to be flushed.
    ///
    /// The target CPUs should be a reference to an [`AtomicCpuSet`] that will
    /// be loaded upon [`Self::dispatch_tlb_flush`].
    ///
    /// The flusher needs to stick to the current CPU. So please provide a
    /// guard that implements [`PinCurrentCpu`].
    pub fn new(target_cpus: &'a AtomicCpuSet, pin_current_guard: G) -> Self {
        Self {
            target_cpus,
            have_unsynced_flush: CpuSet::new_empty(),
            ops_stack: OpsStack::new(),
            _pin_current: pin_current_guard,
        }
    }

    /// Issues a pending TLB flush request.
    ///
    /// This function does not guarantee to flush the TLB entries on either
    /// this CPU or remote CPUs. The flush requests are only performed when
    /// [`Self::dispatch_tlb_flush`] is called.
    pub fn issue_tlb_flush(&mut self, op: TlbFlushOp) {
        self.ops_stack.push(op, None);
    }

    /// Issues a TLB flush request that must happen before dropping the page.
    ///
    /// If we need to remove a mapped page from the page table, we can only
    /// recycle the page after all the relevant TLB entries in all CPUs are
    /// flushed. Otherwise if the page is recycled for other purposes, the user
    /// space program can still access the page through the TLB entries. This
    /// method is designed to be used in such cases.
    pub fn issue_tlb_flush_with(
        &mut self,
        op: TlbFlushOp,
        drop_after_flush: Frame<dyn AnyFrameMeta>,
    ) {
        self.ops_stack.push(op, Some(drop_after_flush));
    }

    /// Do an LATR operation.
    #[cfg(feature = "lazy_tlb_flush_on_unmap")]
    pub fn latr_with(&mut self, op: TlbFlushOp, drop_after_flush: Frame<dyn AnyFrameMeta>) {
        match latr::add_lazy_frame(
            &self.target_cpus.load(Ordering::Relaxed),
            op.clone(),
            drop_after_flush,
        ) {
            Ok(()) => {}
            Err(frame) => {
                // If we cannot add the frame to the lazy TLB flush, fallback.
                self.issue_tlb_flush_with(op, frame);
            }
        }
    }

    /// Dispatches all the pending TLB flush requests.
    ///
    /// All previous pending requests issued by [`Self::issue_tlb_flush`] or
    /// [`Self::issue_tlb_flush_with`] starts to be processed after this
    /// function. But it may not be synchronous. Upon the return of this
    /// function, the TLB entries may not be coherent.
    pub fn dispatch_tlb_flush(&mut self) {
        let irq_guard = crate::trap::irq::disable_local();

        if self.ops_stack.is_empty() {
            return;
        }

        // `Release` to make sure our modification on the PT is visible to CPUs
        // that are going to activate the PT.
        let mut target_cpus = self.target_cpus.load(Ordering::Release);

        let cur_cpu = irq_guard.current_cpu();
        let mut need_flush_on_self = false;

        if target_cpus.contains(cur_cpu) {
            target_cpus.remove(cur_cpu);
            need_flush_on_self = true;
        }

        for cpu in target_cpus.iter() {
            {
                let mut flush_ops = FLUSH_OPS.get_on_cpu(cpu).lock();
                flush_ops.push_from(&self.ops_stack);

                // Clear ACK before dropping the lock to avoid false ACKs.
                ACK_REMOTE_FLUSH
                    .get_on_cpu(cpu)
                    .store(false, Ordering::Relaxed);
            }
            self.have_unsynced_flush.add(cpu);
        }

        crate::smp::inter_processor_call(&target_cpus, do_remote_flush);

        #[cfg(feature = "lazy_tlb_flush_on_unmap")]
        latr::flush_local_gather(&irq_guard);

        // Flush ourselves after sending all IPIs to save some time.
        if need_flush_on_self {
            self.ops_stack.flush_all();
        } else {
            self.ops_stack.clear_without_flush();
        }
    }

    /// Waits for all the previous TLB flush requests to be completed.
    ///
    /// After this function, all TLB entries corresponding to previous
    /// dispatched TLB flush requests are guaranteed to be coherent.
    ///
    /// The TLB flush requests are issued with [`Self::issue_tlb_flush`] and
    /// dispatched with [`Self::dispatch_tlb_flush`]. This method will not
    /// dispatch any issued requests so it will not guarantee TLB coherence
    /// of requests that are not dispatched.
    ///
    /// # Panics
    ///
    /// This method panics if the IRQs are disabled. Since the remote flush are
    /// processed in IRQs, two CPUs may deadlock if they are waiting for each
    /// other's TLB coherence.
    pub fn sync_tlb_flush(&mut self) {
        assert!(
            irq::is_local_enabled(),
            "Waiting for remote flush with IRQs disabled"
        );

        for cpu in self.have_unsynced_flush.iter() {
            while !ACK_REMOTE_FLUSH.get_on_cpu(cpu).load(Ordering::Relaxed) {
                core::hint::spin_loop();
            }
        }

        self.have_unsynced_flush = CpuSet::new_empty();
    }
}

impl<G: PinCurrentCpu> Drop for TlbFlusher<'_, G> {
    fn drop(&mut self) {
        let irq_guard = crate::trap::irq::disable_local();
        let local_flush_ops = FLUSH_OPS.get_with(&irq_guard);
        local_flush_ops.lock().flush_all();
        #[cfg(feature = "lazy_tlb_flush_on_unmap")]
        latr::flush_local_gather(&irq_guard);
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

// The queues of pending requests on each CPU.
cpu_local! {
    static FLUSH_OPS: SpinLock<OpsStack, LocalIrqDisabled> = SpinLock::new(OpsStack::new());
    /// Whether this CPU finishes the last remote flush request.
    static ACK_REMOTE_FLUSH: AtomicBool = AtomicBool::new(true);
}

fn do_remote_flush() {
    // No races because we are in IRQs or have disabled preemption.
    let current_cpu = crate::cpu::CpuId::current_racy();

    let mut new_op_queue = OpsStack::new();
    {
        let mut op_queue = FLUSH_OPS.get_on_cpu(current_cpu).lock();

        core::mem::swap(&mut *op_queue, &mut new_op_queue);

        // ACK before dropping the lock so that we won't miss flush requests.
        ACK_REMOTE_FLUSH
            .get_on_cpu(current_cpu)
            .store(true, Ordering::Relaxed);
    }
    // Unlock the locks quickly to avoid contention. ACK before flushing is
    // fine since we cannot switch back to userspace now.
    new_op_queue.flush_all();
}

/// If a TLB flushing request exceeds this threshold, we flush all.
const FLUSH_ALL_RANGE_THRESHOLD: usize = 32 * PAGE_SIZE;

/// If the number of pending requests exceeds this threshold, we flush all the
/// TLB entries instead of flushing them one by one.
const FLUSH_ALL_OPS_THRESHOLD: usize = 32;

struct OpsStack {
    ops: [Option<TlbFlushOp>; FLUSH_ALL_OPS_THRESHOLD],
    need_flush_all: bool,
    size: usize,
    page_keeper: Vec<Frame<dyn AnyFrameMeta>>,
}

impl OpsStack {
    const fn new() -> Self {
        Self {
            ops: [const { None }; FLUSH_ALL_OPS_THRESHOLD],
            need_flush_all: false,
            size: 0,
            page_keeper: Vec::new(),
        }
    }

    fn is_empty(&self) -> bool {
        !self.need_flush_all && self.size == 0
    }

    fn push(&mut self, op: TlbFlushOp, drop_after_flush: Option<Frame<dyn AnyFrameMeta>>) {
        if let Some(frame) = drop_after_flush {
            self.page_keeper.push(frame);
        }

        if self.need_flush_all {
            return;
        }
        let op = op.optimize_for_large_range();
        if op == TlbFlushOp::All || self.size >= FLUSH_ALL_OPS_THRESHOLD {
            self.need_flush_all = true;
            self.size = 0;
            return;
        }

        self.ops[self.size] = Some(op);
        self.size += 1;
    }

    fn push_from(&mut self, other: &OpsStack) {
        self.page_keeper.extend(other.page_keeper.iter().cloned());

        if self.need_flush_all {
            return;
        }
        if other.need_flush_all || self.size + other.size >= FLUSH_ALL_OPS_THRESHOLD {
            self.need_flush_all = true;
            self.size = 0;
            return;
        }

        for i in 0..other.size {
            self.ops[self.size] = other.ops[i].clone();
            self.size += 1;
        }
    }

    fn flush_all(&mut self) {
        if self.need_flush_all {
            crate::arch::mm::tlb_flush_all_excluding_global();
        } else {
            for i in 0..self.size {
                if let Some(op) = &self.ops[i] {
                    op.perform_on_current();
                }
            }
        }

        self.clear_without_flush();
    }

    fn clear_without_flush(&mut self) {
        self.need_flush_all = false;
        self.size = 0;
        self.page_keeper.clear();
    }
}
