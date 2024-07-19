// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicBool, Ordering};

use trapframe::TrapFrame;

use crate::{arch::irq::IRQ_LIST, cpu_local};

pub(crate) fn call_irq_callback_functions(trap_frame: &TrapFrame, irq_number: usize) {
    // For x86 CPUs, interrupts are not re-entrant. Local interrupts will be disabled when
    // an interrupt handler is called (Unless interrupts are re-enabled in an interrupt handler).
    //
    // FIXME: For arch that supports re-entrant interrupts, we may need to record nested level here.
    IN_INTERRUPT_CONTEXT.store(true, Ordering::Release);

    let irq_line = IRQ_LIST.get().unwrap().get(irq_number).unwrap();
    let callback_functions = irq_line.callback_list();
    for callback_function in callback_functions.iter() {
        callback_function.call(trap_frame);
    }
    drop(callback_functions);

    crate::arch::interrupts_ack(irq_number);

    IN_INTERRUPT_CONTEXT.store(false, Ordering::Release);

    crate::arch::irq::enable_local();
    crate::trap::softirq::process_pending();
}

cpu_local! {
    static IN_INTERRUPT_CONTEXT: AtomicBool = AtomicBool::new(false);
}

/// Returns whether we are in the interrupt context.
///
/// FIXME: Here only hardware irq is taken into account. According to linux implementation, if
/// we are in softirq context, or bottom half is disabled, this function also returns true.
pub fn in_interrupt_context() -> bool {
    IN_INTERRUPT_CONTEXT.load(Ordering::Acquire)
}
