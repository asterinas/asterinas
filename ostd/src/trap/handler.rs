// SPDX-License-Identifier: MPL-2.0

use crate::{arch::irq::IRQ_LIST, cpu_local_cell, trap::TrapFrame};

pub(crate) fn call_irq_callback_functions(trap_frame: &TrapFrame, irq_number: usize) {
    // For x86 CPUs, interrupts are not re-entrant. Local interrupts will be disabled when
    // an interrupt handler is called (Unless interrupts are re-enabled in an interrupt handler).
    //
    // FIXME: For arch that supports re-entrant interrupts, we may need to record nested level here.
    IN_INTERRUPT_CONTEXT.store(true);

    let irq_line = IRQ_LIST.get().unwrap().get(irq_number).unwrap();
    let callback_functions = irq_line.callback_list();
    for callback_function in callback_functions.iter() {
        callback_function.call(trap_frame);
    }
    drop(callback_functions);

    crate::arch::interrupts_ack(irq_number);

    crate::arch::irq::enable_local();
    crate::trap::softirq::process_pending();

    IN_INTERRUPT_CONTEXT.store(false);
}

cpu_local_cell! {
    static IN_INTERRUPT_CONTEXT: bool = false;
}

/// Returns whether we are in the interrupt context.
pub fn in_interrupt_context() -> bool {
    IN_INTERRUPT_CONTEXT.load()
}
