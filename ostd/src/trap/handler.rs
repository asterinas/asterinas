// SPDX-License-Identifier: MPL-2.0

use alloc::boxed::Box;

use spin::Once;

use crate::{arch::irq::IRQ_LIST, cpu_local_cell, task::disable_preempt, trap::TrapFrame};

static BOTTOM_HALF_HANDLER: Once<Box<dyn Fn() + Sync + Send>> = Once::new();

/// Register a function to the interrupt bottom half execution.
///
/// The handling of an interrupt can be divided into two parts: top half and bottom half.
/// Top half typically performs critical tasks and runs at a high priority.
/// Relatively, bottom half defers less critical tasks to reduce the time spent in
/// hardware interrupt context, thus allowing the interrupts to be handled more quickly.
///
/// The bottom half handler will be called after the top half with interrupts enabled.
///
/// This function can only be registered once. Subsequent calls will do nothing.
pub fn register_bottom_half_handler<F>(func: F)
where
    F: Fn() + Sync + Send + 'static,
{
    BOTTOM_HALF_HANDLER.call_once(|| Box::new(func));
}

fn process_top_half(trap_frame: &TrapFrame, irq_number: usize) {
    let irq_line = IRQ_LIST.get().unwrap().get(irq_number).unwrap();
    let callback_functions = irq_line.callback_list();
    for callback_function in callback_functions.iter() {
        callback_function.call(trap_frame);
    }
}

fn process_bottom_half() {
    // We need to disable preemption when processing bottom half since
    // the interrupt is enabled in this context.
    let _preempt_guard = disable_preempt();

    if let Some(handler) = BOTTOM_HALF_HANDLER.get() {
        handler()
    }
}

pub(crate) fn call_irq_callback_functions(trap_frame: &TrapFrame, irq_number: usize) {
    // For x86 CPUs, interrupts are not re-entrant. Local interrupts will be disabled when
    // an interrupt handler is called (Unless interrupts are re-enabled in an interrupt handler).
    //
    // FIXME: For arch that supports re-entrant interrupts, we may need to record nested level here.
    IN_INTERRUPT_CONTEXT.store(true);

    process_top_half(trap_frame, irq_number);
    crate::arch::interrupts_ack(irq_number);
    crate::arch::irq::enable_local();
    process_bottom_half();

    IN_INTERRUPT_CONTEXT.store(false);
}

cpu_local_cell! {
    static IN_INTERRUPT_CONTEXT: bool = false;
}

/// Returns whether we are in the interrupt context.
pub fn in_interrupt_context() -> bool {
    IN_INTERRUPT_CONTEXT.load()
}
