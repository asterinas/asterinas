// SPDX-License-Identifier: MPL-2.0

use spin::Once;

use crate::{arch::irq::IRQ_LIST, cpu_local_cell, task::disable_preempt, trap::TrapFrame};

static BOTTOM_HALF_HANDLER: Once<fn()> = Once::new();

/// Registers a function to the interrupt bottom half execution.
///
/// The handling of an interrupt can be divided into two parts: top half and bottom half.
/// Top half typically performs critical tasks and runs at a high priority.
/// Relatively, bottom half defers less critical tasks to reduce the time spent in
/// hardware interrupt context, thus allowing the interrupts to be handled more quickly.
///
/// The bottom half handler will be called after the top half with interrupts enabled.
///
/// This function can only be registered once. Subsequent calls will do nothing.
pub fn register_bottom_half_handler(func: fn()) {
    BOTTOM_HALF_HANDLER.call_once(|| func);
}

fn process_top_half(trap_frame: &TrapFrame, irq_number: usize) {
    let irq_line = IRQ_LIST.get().unwrap().get(irq_number).unwrap();
    let callback_functions = irq_line.callback_list();
    for callback_function in callback_functions.iter() {
        callback_function.call(trap_frame);
    }
}

fn process_bottom_half() {
    let Some(handler) = BOTTOM_HALF_HANDLER.get() else {
        return;
    };

    // We need to disable preemption when processing bottom half since
    // the interrupt is enabled in this context.
    // This needs to be done before enabling the local interrupts to
    // avoid race conditions.
    let preempt_guard = disable_preempt();
    crate::arch::irq::enable_local();

    handler();

    crate::arch::irq::disable_local();
    drop(preempt_guard);
}

pub(crate) fn call_irq_callback_functions(trap_frame: &TrapFrame, irq_number: usize) {
    // We do not provide support for reentrant interrupt handlers. Otherwise, it's very hard to
    // guarantee the absence of stack overflows.
    //
    // As a concrete example, Linux once supported them in its early days, but has dropped support
    // for this very reason. See
    // <https://github.com/torvalds/linux/commit/d8bf368d0631d4bc2612d8bf2e4e8e74e620d0cc>.
    //
    // Nevertheless, we still need a counter to track the nested level because interrupts are
    // enabled while the bottom half is being processing. The counter cannot exceed two because the
    // bottom half cannot be reentrant for the same reason.
    INTERRUPT_NESTED_LEVEL.add_assign(1);

    process_top_half(trap_frame, irq_number);
    crate::arch::interrupts_ack(irq_number);

    if INTERRUPT_NESTED_LEVEL.load() == 1 {
        process_bottom_half();
    }

    INTERRUPT_NESTED_LEVEL.sub_assign(1);
}

cpu_local_cell! {
    static INTERRUPT_NESTED_LEVEL: u8 = 0;
}

/// Returns whether we are in the interrupt context.
///
/// Note that both the top half and the bottom half is processed in the interrupt context.
pub fn in_interrupt_context() -> bool {
    INTERRUPT_NESTED_LEVEL.load() != 0
}
