// SPDX-License-Identifier: MPL-2.0

use spin::Once;

use super::irq::{disable_local, process_top_half, DisabledLocalIrqGuard};
use crate::{arch::trap::TrapFrame, cpu_local_cell, task::disable_preempt};

static BOTTOM_HALF_HANDLER: Once<fn(DisabledLocalIrqGuard) -> DisabledLocalIrqGuard> = Once::new();

/// Registers a function to the interrupt bottom half execution.
///
/// The handling of an interrupt can be divided into two parts: top half and bottom half.
/// Top half typically performs critical tasks and runs at a high priority.
/// Relatively, bottom half defers less critical tasks to reduce the time spent in
/// hardware interrupt context, thus allowing the interrupts to be handled more quickly.
///
/// The bottom half handler is called following the execution of the top half.
/// Because the handler accepts a [`DisabledLocalIrqGuard`] as a parameter,
/// interrupts are still disabled upon entering the handler.
/// However, the handler can enable interrupts by internally dropping the guard.
/// When the handler returns, interrupts should remain disabled,
/// as the handler is expected to return an IRQ guard.
///
/// This function can only be registered once. Subsequent calls will do nothing.
pub fn register_bottom_half_handler(func: fn(DisabledLocalIrqGuard) -> DisabledLocalIrqGuard) {
    BOTTOM_HALF_HANDLER.call_once(|| func);
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

    // We need to ensure that local interrupts are disabled
    // when the handler returns to prevent race conditions.
    // See <https://github.com/asterinas/asterinas/pull/1623#discussion_r1964709636> for more details.
    let irq_guard = disable_local();
    let irq_guard = handler(irq_guard);

    // Interrupts should remain disabled when `process_bottom_half` returns,
    // so we simply forget the guard.
    core::mem::forget(irq_guard);
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
