// SPDX-License-Identifier: MPL-2.0

//! Symmetric Multi-Processing (SMP) support.
//!
//! This module provides a way to execute code on other processors via inter-
//! processor interrupts.

use alloc::{boxed::Box, collections::VecDeque};

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
/// The provided function `f` will be executed on all target processors
/// specified by `targets`. It can also be executed on the current processor.
/// The function should be short and non-blocking, as it will be executed in
/// interrupt context with interrupts disabled.
///
/// This function does not block until all the target processors acknowledges
/// the interrupt. So if any of the target processors disables IRQs for too
/// long that the controller cannot queue them, the function will not be
/// executed.
///
/// The function `f` will be executed asynchronously on the target processors.
/// However if called on the current processor, it will be synchronous.
pub fn inter_processor_call(targets: &CpuSet, f: fn()) {
    let ipi_sender = IPI_SENDER.get().unwrap();
    ipi_sender.inter_processor_call(targets, f);
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
    pub(crate) fn inter_processor_call(&self, targets: &CpuSet, f: fn()) {
        let irq_guard = irq::disable_local();
        let this_cpu_id = irq_guard.current_cpu();

        let mut call_on_self = false;
        for cpu_id in targets.iter() {
            if cpu_id == this_cpu_id {
                call_on_self = true;
                continue;
            }
            CALL_QUEUES.get_on_cpu(cpu_id).lock().push_back(f);
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
            f();
        }
    }
}

cpu_local! {
    static CALL_QUEUES: SpinLock<VecDeque<fn()>> = SpinLock::new(VecDeque::new());
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

    let mut queue = CALL_QUEUES.get_on_cpu(this_cpu_id).lock();
    while let Some(f) = queue.pop_front() {
        log::trace!(
            "Performing inter-processor call to {:#?} on CPU {:#?}",
            f,
            this_cpu_id,
        );
        f();
    }
}

pub(super) fn init() {
    IPI_SENDER.call_once(|| {
        let hw_cpu_ids = crate::boot::smp::construct_hw_cpu_id_mapping();
        IpiSender { hw_cpu_ids }
    });
}
