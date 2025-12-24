// SPDX-License-Identifier: MPL-2.0

//! TLB flush operations.

use alloc::vec::Vec;
use core::{
    mem::MaybeUninit,
    ops::Range,
    sync::atomic::{AtomicBool, Ordering},
};

use super::{
    PAGE_SIZE, Vaddr,
    frame::{Frame, meta::AnyFrameMeta},
};
use crate::{
    arch::irq,
    const_assert,
    cpu::{AtomicCpuSet, CpuSet, PinCurrentCpu},
    cpu_local,
    smp::IpiSender,
    sync::{LocalIrqDisabled, SpinLock},
};

/// A TLB flusher that is aware of which CPUs are needed to be flushed.
///
/// The flusher needs to stick to the current CPU.
pub struct TlbFlusher<'a, G: PinCurrentCpu> {
    target_cpus: &'a AtomicCpuSet,
    have_unsynced_flush: CpuSet,
    ops_stack: OpsStack,
    ipi_sender: Option<&'static IpiSender>,
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
            ipi_sender: crate::smp::IPI_SENDER.get(),
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

    /// Dispatches all the pending TLB flush requests.
    ///
    /// All previous pending requests issued by [`Self::issue_tlb_flush`] or
    /// [`Self::issue_tlb_flush_with`] starts to be processed after this
    /// function. But it may not be synchronous. Upon the return of this
    /// function, the TLB entries may not be coherent.
    pub fn dispatch_tlb_flush(&mut self) {
        let irq_guard = crate::irq::disable_local();

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

        if let Some(ipi_sender) = self.ipi_sender {
            for cpu in target_cpus.iter() {
                self.have_unsynced_flush.add(cpu);

                let mut flush_ops = FLUSH_OPS.get_on_cpu(cpu).lock();
                flush_ops.push_from(&self.ops_stack);
                // Clear ACK before dropping the lock to avoid false ACKs.
                ACK_REMOTE_FLUSH
                    .get_on_cpu(cpu)
                    .store(false, Ordering::Relaxed);
            }

            ipi_sender.inter_processor_call(&target_cpus, do_remote_flush);
        }

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
        if self.ipi_sender.is_none() {
            // We performed some TLB flushes in the boot context. The AP's boot
            // process should take care of them.
            return;
        }

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

/// The operation to flush TLB entries.
///
/// The variants of this structure are:
///  - Flushing all TLB entries except for the global entries;
///  - Flushing the TLB entry associated with an address;
///  - Flushing the TLB entries for a specific range of virtual addresses;
///
/// This is a `struct` instead of an `enum` because if trivially representing
/// the three variants with an `enum`, it would be 24 bytes. To minimize the
/// memory footprint, we encode all three variants into an 8-byte integer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TlbFlushOp(Vaddr);

// We require the address to be page-aligned, so the in-page offset part of the
// address can be used to store the length. A sanity check to ensure that we
// don't allow ranged flush operations with a too long length.
const_assert!(TlbFlushOp::FLUSH_RANGE_NPAGES_MASK | (PAGE_SIZE - 1) == PAGE_SIZE - 1);

impl TlbFlushOp {
    const FLUSH_ALL_VAL: Vaddr = Vaddr::MAX;
    const FLUSH_RANGE_NPAGES_MASK: Vaddr =
        (1 << (usize::BITS - FLUSH_ALL_PAGES_THRESHOLD.leading_zeros())) - 1;

    /// Performs the TLB flush operation on the current CPU.
    pub fn perform_on_current(&self) {
        use crate::arch::mm::{
            tlb_flush_addr, tlb_flush_addr_range, tlb_flush_all_excluding_global,
        };
        match self.0 {
            Self::FLUSH_ALL_VAL => tlb_flush_all_excluding_global(),
            addr => {
                let start = addr & !Self::FLUSH_RANGE_NPAGES_MASK;
                let num_pages = addr & Self::FLUSH_RANGE_NPAGES_MASK;

                debug_assert!((addr & (PAGE_SIZE - 1)) < FLUSH_ALL_PAGES_THRESHOLD);
                debug_assert!(num_pages != 0);

                if num_pages == 1 {
                    tlb_flush_addr(start);
                } else {
                    tlb_flush_addr_range(&(start..start + num_pages * PAGE_SIZE));
                }
            }
        }
    }

    /// Creates a new TLB flush operation that flushes all TLB entries except
    /// for the global entries.
    pub const fn for_all() -> Self {
        TlbFlushOp(Self::FLUSH_ALL_VAL)
    }

    /// Creates a new TLB flush operation that flushes the TLB entry associated
    /// with the provided virtual address.
    pub const fn for_single(addr: Vaddr) -> Self {
        TlbFlushOp(addr | 1)
    }

    /// Creates a new TLB flush operation that flushes the TLB entries for the
    /// specified virtual address range.
    ///
    /// If the range is too large, the resulting [`TlbFlushOp`] will flush all
    /// TLB entries instead.
    ///
    /// # Panics
    ///
    /// Panics if the range is not page-aligned or if the range is empty.
    pub const fn for_range(range: Range<Vaddr>) -> Self {
        assert!(
            range.start.is_multiple_of(PAGE_SIZE),
            "Range start must be page-aligned"
        );
        assert!(
            range.end.is_multiple_of(PAGE_SIZE),
            "Range end must be page-aligned"
        );
        assert!(range.start < range.end, "Range must not be empty");
        let num_pages = (range.end - range.start) / PAGE_SIZE;
        if num_pages >= FLUSH_ALL_PAGES_THRESHOLD {
            return TlbFlushOp::for_all();
        }
        TlbFlushOp(range.start | (num_pages as Vaddr))
    }

    /// Returns the number of pages to flush.
    ///
    /// If it returns `u32::MAX`, it means to flush all the entries. Otherwise
    /// the return value should be less than [`FLUSH_ALL_PAGES_THRESHOLD`] and
    /// non-zero.
    fn num_pages(&self) -> u32 {
        if self.0 == Self::FLUSH_ALL_VAL {
            u32::MAX
        } else {
            debug_assert!((self.0 & (PAGE_SIZE - 1)) < FLUSH_ALL_PAGES_THRESHOLD);
            let num_pages = (self.0 & Self::FLUSH_RANGE_NPAGES_MASK) as u32;
            debug_assert!(num_pages != 0);
            num_pages
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

/// If the number of pending pages to flush exceeds this threshold, we flush all the
/// TLB entries instead of flushing them one by one.
const FLUSH_ALL_PAGES_THRESHOLD: usize = 32;

struct OpsStack {
    /// From 0 to `num_ops`, the array entry must be initialized.
    ops: [MaybeUninit<TlbFlushOp>; FLUSH_ALL_PAGES_THRESHOLD],
    num_ops: u32,
    /// If this is `u32::MAX`, we should flush all entries irrespective of the
    /// contents of `ops`. And in this case `num_ops` must be zero.
    ///
    /// Otherwise, it counts the number of pages to flush in `ops`.
    num_pages_to_flush: u32,
    page_keeper: Vec<Frame<dyn AnyFrameMeta>>,
}

impl OpsStack {
    const fn new() -> Self {
        Self {
            ops: [const { MaybeUninit::uninit() }; FLUSH_ALL_PAGES_THRESHOLD],
            num_ops: 0,
            num_pages_to_flush: 0,
            page_keeper: Vec::new(),
        }
    }

    fn is_empty(&self) -> bool {
        self.num_ops == 0 && self.num_pages_to_flush == 0
    }

    fn need_flush_all(&self) -> bool {
        self.num_pages_to_flush == u32::MAX
    }

    fn push(&mut self, op: TlbFlushOp, drop_after_flush: Option<Frame<dyn AnyFrameMeta>>) {
        if let Some(frame) = drop_after_flush {
            self.page_keeper.push(frame);
        }

        if self.need_flush_all() {
            return;
        }
        let op_num_pages = op.num_pages();
        if op == TlbFlushOp::for_all()
            || self.num_pages_to_flush + op_num_pages >= FLUSH_ALL_PAGES_THRESHOLD as u32
        {
            self.num_pages_to_flush = u32::MAX;
            self.num_ops = 0;
            return;
        }

        self.ops[self.num_ops as usize].write(op);
        self.num_ops += 1;
        self.num_pages_to_flush += op_num_pages;
    }

    fn push_from(&mut self, other: &OpsStack) {
        self.page_keeper.extend(other.page_keeper.iter().cloned());

        if self.need_flush_all() {
            return;
        }
        if other.need_flush_all()
            || self.num_pages_to_flush + other.num_pages_to_flush
                >= FLUSH_ALL_PAGES_THRESHOLD as u32
        {
            self.num_pages_to_flush = u32::MAX;
            self.num_ops = 0;
            return;
        }

        for other_op in other.ops_iter() {
            self.ops[self.num_ops as usize].write(other_op.clone());
            self.num_ops += 1;
        }
        self.num_pages_to_flush += other.num_pages_to_flush;
    }

    fn flush_all(&mut self) {
        if self.need_flush_all() {
            crate::arch::mm::tlb_flush_all_excluding_global();
        } else {
            self.ops_iter().for_each(|op| {
                op.perform_on_current();
            });
        }

        self.clear_without_flush();
    }

    fn clear_without_flush(&mut self) {
        self.num_pages_to_flush = 0;
        self.num_ops = 0;
        self.page_keeper.clear();
    }

    fn ops_iter(&self) -> impl Iterator<Item = &TlbFlushOp> {
        self.ops.iter().take(self.num_ops as usize).map(|op| {
            // SAFETY: From 0 to `num_ops`, the array entry must be initialized.
            unsafe { op.assume_init_ref() }
        })
    }
}
