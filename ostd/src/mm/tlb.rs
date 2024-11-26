// SPDX-License-Identifier: MPL-2.0

//! TLB flush operations.

use alloc::vec::Vec;
use core::ops::Range;

use super::{page::DynPage, Vaddr, PAGE_SIZE};
use crate::{
    cpu::{CpuSet, PinCurrentCpu},
    cpu_local,
    sync::{LocalIrqDisabled, SpinLock},
    task::disable_preempt,
};

/// A TLB flusher that is aware of which CPUs are needed to be flushed.
///
/// The flusher needs to stick to the current CPU.
pub struct TlbFlusher<G: PinCurrentCpu> {
    target_cpus: CpuSet,
    // Better to store them here since loading and counting them from the CPUs
    // list brings non-trivial overhead.
    need_remote_flush: bool,
    need_self_flush: bool,
    _pin_current: G,
}

impl<G: PinCurrentCpu> TlbFlusher<G> {
    /// Creates a new TLB flusher with the specified CPUs to be flushed.
    ///
    /// The flusher needs to stick to the current CPU. So please provide a
    /// guard that implements [`PinCurrentCpu`].
    pub fn new(target_cpus: CpuSet, pin_current_guard: G) -> Self {
        let current_cpu = pin_current_guard.current_cpu();

        let mut need_self_flush = false;
        let mut need_remote_flush = false;

        for cpu in target_cpus.iter() {
            if cpu == current_cpu {
                need_self_flush = true;
            } else {
                need_remote_flush = true;
            }
        }
        Self {
            target_cpus,
            need_remote_flush,
            need_self_flush,
            _pin_current: pin_current_guard,
        }
    }

    /// Issues a pending TLB flush request.
    ///
    /// On SMP systems, the notification is sent to all the relevant CPUs only
    /// when [`Self::dispatch_tlb_flush`] is called.
    pub fn issue_tlb_flush(&self, op: TlbFlushOp) {
        self.issue_tlb_flush_(op, None);
    }

    /// Dispatches all the pending TLB flush requests.
    ///
    /// The pending requests are issued by [`Self::issue_tlb_flush`].
    pub fn dispatch_tlb_flush(&self) {
        if !self.need_remote_flush {
            return;
        }

        crate::smp::inter_processor_call(&self.target_cpus, do_remote_flush);
    }

    /// Issues a TLB flush request that must happen before dropping the page.
    ///
    /// If we need to remove a mapped page from the page table, we can only
    /// recycle the page after all the relevant TLB entries in all CPUs are
    /// flushed. Otherwise if the page is recycled for other purposes, the user
    /// space program can still access the page through the TLB entries. This
    /// method is designed to be used in such cases.
    pub fn issue_tlb_flush_with(&self, op: TlbFlushOp, drop_after_flush: DynPage) {
        self.issue_tlb_flush_(op, Some(drop_after_flush));
    }

    /// Whether the TLB flusher needs to flush the TLB entries on other CPUs.
    pub fn need_remote_flush(&self) -> bool {
        self.need_remote_flush
    }

    /// Whether the TLB flusher needs to flush the TLB entries on the current CPU.
    pub fn need_self_flush(&self) -> bool {
        self.need_self_flush
    }

    fn issue_tlb_flush_(&self, op: TlbFlushOp, drop_after_flush: Option<DynPage>) {
        let op = op.optimize_for_large_range();

        // Fast path for single CPU cases.
        if !self.need_remote_flush {
            if self.need_self_flush {
                op.perform_on_current();
            }
            return;
        }

        // Slow path for multi-CPU cases.
        for cpu in self.target_cpus.iter() {
            let mut op_queue = FLUSH_OPS.get_on_cpu(cpu).lock();
            if let Some(drop_after_flush) = drop_after_flush.clone() {
                PAGE_KEEPER.get_on_cpu(cpu).lock().push(drop_after_flush);
            }
            op_queue.push(op.clone());
        }
    }
}

/// The operation to flush TLB entries.
#[derive(Debug, Clone)]
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
//
// Lock ordering: lock FLUSH_OPS before PAGE_KEEPER.
cpu_local! {
    static FLUSH_OPS: SpinLock<OpsStack, LocalIrqDisabled> = SpinLock::new(OpsStack::new());
    static PAGE_KEEPER: SpinLock<Vec<DynPage>, LocalIrqDisabled> = SpinLock::new(Vec::new());
}

fn do_remote_flush() {
    let preempt_guard = disable_preempt();
    let current_cpu = preempt_guard.current_cpu();

    let mut op_queue = FLUSH_OPS.get_on_cpu(current_cpu).lock();
    op_queue.flush_all();
    PAGE_KEEPER.get_on_cpu(current_cpu).lock().clear();
}

/// If a TLB flushing request exceeds this threshold, we flush all.
pub(crate) const FLUSH_ALL_RANGE_THRESHOLD: usize = 32 * PAGE_SIZE;

/// If the number of pending requests exceeds this threshold, we flush all the
/// TLB entries instead of flushing them one by one.
const FLUSH_ALL_OPS_THRESHOLD: usize = 32;

struct OpsStack {
    ops: [Option<TlbFlushOp>; FLUSH_ALL_OPS_THRESHOLD],
    need_flush_all: bool,
    size: usize,
}

impl OpsStack {
    const fn new() -> Self {
        const ARRAY_REPEAT_VALUE: Option<TlbFlushOp> = None;
        Self {
            ops: [ARRAY_REPEAT_VALUE; FLUSH_ALL_OPS_THRESHOLD],
            need_flush_all: false,
            size: 0,
        }
    }

    fn push(&mut self, op: TlbFlushOp) {
        if self.need_flush_all {
            return;
        }

        if self.size < FLUSH_ALL_OPS_THRESHOLD {
            self.ops[self.size] = Some(op);
            self.size += 1;
        } else {
            self.need_flush_all = true;
            self.size = 0;
        }
    }

    fn flush_all(&mut self) {
        if self.need_flush_all {
            crate::arch::mm::tlb_flush_all_excluding_global();
            self.need_flush_all = false;
        } else {
            for i in 0..self.size {
                if let Some(op) = &self.ops[i] {
                    op.perform_on_current();
                }
            }
        }

        self.size = 0;
    }
}
