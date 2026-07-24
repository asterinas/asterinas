// SPDX-License-Identifier: MPL-2.0

//! Symmetric Multi-Processing (SMP) support.
//!
//! This module provides asynchronous and synchronous inter-processor calls over
//! architecture-specific inter-processor interrupts. Public call helpers enqueue
//! work in CPU-local queues, while `IpiSender` maps logical CPU IDs to
//! hardware CPU IDs and delegates interrupt delivery to [`crate::arch::irq`].

use alloc::{boxed::Box, collections::VecDeque, sync::Arc};
use core::sync::atomic::{AtomicUsize, Ordering};

use spin::Once;

use crate::{
    arch::{irq::HwCpuId, trap::TrapFrame},
    cpu::{CpuSet, PinCurrentCpu},
    cpu_local, irq,
    sync::SpinLock,
    util::id_set::Id,
};

/// Executes a function on other processors.
///
/// The provided function `call_fn` will be executed on all target processors
/// specified by `targets`. It can also be executed on the current processor.
/// The function should be short and non-blocking, as it will be executed in
/// interrupt context with interrupts disabled.
///
/// This function does not block until all the target processors acknowledges
/// the interrupt. So if any of the target processors disables IRQs for too
/// long that the controller cannot queue them, the function will not be
/// executed.
///
/// The function `call_fn` will be executed asynchronously on the target
/// processors. However if called on the current processor, it will be
/// synchronous.
pub fn inter_processor_call(targets: &CpuSet, call_fn: fn()) {
    let ipi_sender = IPI_SENDER.get().unwrap();
    ipi_sender.inter_processor_call(targets, call_fn);
}

/// Executes a function on target processors and waits for its completion.
///
/// The provided function `call_fn` follows the same execution-context
/// constraints as [`inter_processor_call`]: it is executed in interrupt context with
/// interrupts disabled on remote processors, so it must be short and
/// non-blocking.
///
/// This function blocks until all target processors have executed `call_fn`.
/// The current processor executes `call_fn` synchronously if it is included in
/// `targets`. It assumes that the architecture IPI backend can deliver an IPI
/// to every started target CPU, or keep it pending until the target enables
/// interrupts.
///
/// # Panics
///
/// This function panics if local IRQs are disabled. Waiting for remote
/// processors while local IRQs are disabled can deadlock if those processors
/// are also waiting for this processor to handle an IPI.
pub fn sync_inter_processor_call(targets: &CpuSet, call_fn: fn()) {
    let ipi_sender = IPI_SENDER.get().unwrap();
    ipi_sender.sync_inter_processor_call(targets, call_fn);
}

/// Executes a full memory barrier on target processors and waits for it.
pub fn sync_inter_processor_memory_barrier(targets: &CpuSet) {
    sync_inter_processor_call(targets, || {
        core::sync::atomic::fence(Ordering::SeqCst);
    });
}

/// A sender that carries necessary information to send inter-processor interrupts.
///
/// The purpose of exporting this type is to enable the users to check whether
/// [`IPI_SENDER`] has been initialized.
pub(crate) struct IpiSender {
    hw_cpu_ids: Box<[HwCpuId]>,
}

/// The [`IpiSender`] singleton.
pub(crate) static IPI_SENDER: Once<IpiSender> = Once::new();

impl IpiSender {
    /// Executes a function on other processors.
    ///
    /// See [`inter_processor_call`] for details. The purpose of exporting this
    /// method is to enable callers to check whether [`IPI_SENDER`] has been
    /// initialized.
    pub(crate) fn inter_processor_call(&self, targets: &CpuSet, call_fn: fn()) {
        let irq_guard = irq::disable_local();
        let this_cpu_id = irq_guard.current_cpu();

        let mut call_on_self = false;
        for cpu_id in targets.iter() {
            if cpu_id == this_cpu_id {
                call_on_self = true;
                continue;
            }
            CALL_QUEUES.get_on_cpu(cpu_id).lock().push_back(call_fn);
        }
        for cpu_id in targets.iter() {
            if cpu_id == this_cpu_id {
                continue;
            }
            let hw_cpu_id = self.hw_cpu_ids[cpu_id.as_usize()];
            crate::arch::irq::send_ipi(hw_cpu_id, &irq_guard as _);
        }
        if call_on_self {
            // Execute the function synchronously.
            call_fn();
        }
    }

    /// Executes a function on target processors and waits for its completion.
    ///
    /// See [`sync_inter_processor_call`] for details.
    pub(crate) fn sync_inter_processor_call(&self, targets: &CpuSet, call_fn: fn()) {
        assert!(
            crate::arch::irq::is_local_enabled(),
            "Waiting for remote inter-processor calls with IRQs disabled"
        );

        let completion = Arc::new(SyncCallCompletion {
            remaining: AtomicUsize::new(0),
        });
        let irq_guard = irq::disable_local();
        let this_cpu_id = irq_guard.current_cpu();
        let mut remote_targets = targets.clone();
        let call_on_self = remote_targets.contains(this_cpu_id);
        remote_targets.remove(this_cpu_id);

        // Each synchronous call owns its completion state. Remote CPUs drop
        // their queue entry after running `call_fn`, while the caller keeps this
        // reference alive until all targets have acknowledged completion.
        completion.remaining.store(
            remote_targets.count() + usize::from(call_on_self),
            Ordering::Relaxed,
        );

        for cpu_id in remote_targets.iter() {
            SYNC_CALL_QUEUES
                .get_on_cpu(cpu_id)
                .lock()
                .push_back(SyncCall {
                    completion: completion.clone(),
                    call_fn,
                });
        }
        for cpu_id in remote_targets.iter() {
            let hw_cpu_id = self.hw_cpu_ids[cpu_id.as_usize()];
            // The synchronous contract relies on the architecture IPI backend
            // to make this interrupt observable by the target CPU. Architectures
            // that can report delivery failure should surface it through
            // `send_ipi` before using this API for synchronous waits.
            crate::arch::irq::send_ipi(hw_cpu_id, &irq_guard as _);
        }
        if call_on_self {
            call_fn();
            completion.complete();
        }

        drop(irq_guard);

        while !completion.is_complete() {
            core::hint::spin_loop();
        }
    }
}

cpu_local! {
    static CALL_QUEUES: SpinLock<VecDeque<fn()>> = SpinLock::new(VecDeque::new());
    static SYNC_CALL_QUEUES: SpinLock<VecDeque<SyncCall>> = SpinLock::new(VecDeque::new());
}

struct SyncCall {
    completion: Arc<SyncCallCompletion>,
    call_fn: fn(),
}

struct SyncCallCompletion {
    remaining: AtomicUsize,
}

impl SyncCallCompletion {
    fn complete(&self) {
        self.remaining.fetch_sub(1, Ordering::Release);
    }

    fn is_complete(&self) -> bool {
        self.remaining.load(Ordering::Acquire) == 0
    }
}

/// Handles inter-processor calls.
///
/// # Safety
///
/// This function must be called from an IRQ handler that can be triggered by
/// inter-processor interrupts.
pub(crate) unsafe fn do_inter_processor_call(_trapframe: &TrapFrame) {
    // No races because we are in IRQs.
    let this_cpu_id = crate::cpu::CpuId::current_racy();

    loop {
        let call = SYNC_CALL_QUEUES.get_on_cpu(this_cpu_id).lock().pop_front();
        let Some(call) = call else {
            break;
        };

        crate::debug!(
            "Performing synchronous inter-processor call to {:#?} on CPU {:#?}",
            call.call_fn,
            this_cpu_id,
        );
        (call.call_fn)();
        call.completion.complete();
    }

    let mut queue = CALL_QUEUES.get_on_cpu(this_cpu_id).lock();
    while let Some(call_fn) = queue.pop_front() {
        crate::debug!(
            "Performing inter-processor call to {:#?} on CPU {:#?}",
            call_fn,
            this_cpu_id,
        );
        call_fn();
    }
}

pub(super) fn init() {
    IPI_SENDER.call_once(|| {
        let hw_cpu_ids = crate::boot::smp::construct_hw_cpu_id_mapping();
        IpiSender { hw_cpu_ids }
    });
}

#[cfg(ktest)]
mod test {
    use core::sync::atomic::{AtomicUsize, Ordering};

    use crate::{
        cpu::{self, PinCurrentCpu},
        prelude::ktest,
        task,
    };

    static SYNC_CALL_COUNT: AtomicUsize = AtomicUsize::new(0);

    fn count_sync_call() {
        SYNC_CALL_COUNT.fetch_add(1, Ordering::SeqCst);
    }

    #[ktest]
    fn sync_inter_processor_call_runs_on_all_cpus() {
        let before = SYNC_CALL_COUNT.load(Ordering::SeqCst);

        super::sync_inter_processor_call(&cpu::CpuSet::new_full(), count_sync_call);

        assert_eq!(
            SYNC_CALL_COUNT.load(Ordering::SeqCst) - before,
            cpu::num_cpus()
        );
    }

    #[ktest]
    fn sync_inter_processor_call_runs_on_current_cpu() {
        let preempt_guard = task::disable_preempt();
        let before = SYNC_CALL_COUNT.load(Ordering::SeqCst);

        super::sync_inter_processor_call(
            &cpu::CpuSet::from(preempt_guard.current_cpu()),
            count_sync_call,
        );

        assert_eq!(SYNC_CALL_COUNT.load(Ordering::SeqCst) - before, 1);
    }

    #[ktest]
    fn sync_inter_processor_call_runs_on_remote_cpu() {
        if cpu::num_cpus() < 2 {
            return;
        }

        let preempt_guard = task::disable_preempt();
        let target_cpu = cpu::all_cpus()
            .find(|cpu_id| *cpu_id != preempt_guard.current_cpu())
            .unwrap();
        let before = SYNC_CALL_COUNT.load(Ordering::SeqCst);

        super::sync_inter_processor_call(&cpu::CpuSet::from(target_cpu), count_sync_call);

        assert_eq!(SYNC_CALL_COUNT.load(Ordering::SeqCst) - before, 1);
    }

    #[ktest]
    fn sync_inter_processor_memory_barrier_runs_on_all_cpus() {
        super::sync_inter_processor_memory_barrier(&cpu::CpuSet::new_full());
    }
}
