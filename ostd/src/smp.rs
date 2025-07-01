// SPDX-License-Identifier: MPL-2.0

//! Symmetric Multi-Processing (SMP) support.
//!
//! This module provides a way to execute code on other processors via inter-
//! processor interrupts.

use alloc::{boxed::Box, collections::VecDeque};

use spin::Once;

use crate::{
    arch::{
        irq::{send_ipi, HwCpuId},
        trap::TrapFrame,
    },
    cpu::{CpuSet, PinCurrentCpu},
    cpu_local,
    sync::SpinLock,
    trap::{self, irq::IrqLine},
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
    let irq_guard = trap::irq::disable_local();
    let this_cpu_id = irq_guard.current_cpu();

    let ipi_data = IPI_GLOBAL_DATA.get().unwrap();
    let irq_num = ipi_data.irq.num();

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
        // SAFETY: The value of `irq_num` corresponds to a valid IRQ line and
        // triggering it will not cause any safety issues.
        unsafe {
            send_ipi(
                ipi_data.hw_cpu_ids[cpu_id.as_usize()],
                irq_num,
                &irq_guard as _,
            )
        };
    }
    if call_on_self {
        // Execute the function synchronously.
        f();
    }
}

struct IpiGlobalData {
    irq: IrqLine,
    hw_cpu_ids: Box<[HwCpuId]>,
}

static IPI_GLOBAL_DATA: Once<IpiGlobalData> = Once::new();

cpu_local! {
    static CALL_QUEUES: SpinLock<VecDeque<fn()>> = SpinLock::new(VecDeque::new());
}

fn do_inter_processor_call(_trapframe: &TrapFrame) {
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
    IPI_GLOBAL_DATA.call_once(|| {
        let mut irq = IrqLine::alloc().unwrap();
        irq.on_active(do_inter_processor_call);

        let hw_cpu_ids = crate::boot::smp::construct_hw_cpu_id_mapping();

        IpiGlobalData { irq, hw_cpu_ids }
    });
}
