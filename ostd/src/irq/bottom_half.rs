// SPDX-License-Identifier: MPL-2.0

//! The bottom half of interrupt handling.

use spin::Once;

use super::{DisabledLocalIrqGuard, InterruptLevel, disable_local};
use crate::task::disable_preempt;

/// Registers a bottom half callback to be executed at interrupt level 1.
///
/// The callback takes a [`DisabledLocalIrqGuard`] as the first argument.
/// This allows the callback to drop the guard
/// in order to re-enable IRQs on the current CPU.
/// The callback requires returning a `DisabledLocalIrqGuard`,
/// thus ensuring that local IRQs are disabled by the end of the callback.
/// The second argument is the IRQ number being processed.
///
/// The function may be called only once; subsequent calls take no effect.
pub fn register_bottom_half_handler_l1(
    func: fn(DisabledLocalIrqGuard, u8) -> DisabledLocalIrqGuard,
) {
    BOTTOM_HALF_HANDLER_L1.call_once(|| func);
}

/// Registers a bottom half callback to be executed at interrupt level 2.
///
/// Unlike the level 1 bottom half callback,
/// the level 2 bottom half callback registered with this function
/// cannot re-enable local IRQs.
/// The function takes the IRQ number being processed as argument.
///
/// The function may be called only once; subsequent calls take no effect.
pub fn register_bottom_half_handler_l2(func: fn(u8)) {
    BOTTOM_HALF_HANDLER_L2.call_once(|| func);
}

static BOTTOM_HALF_HANDLER_L1: Once<fn(DisabledLocalIrqGuard, u8) -> DisabledLocalIrqGuard> =
    Once::new();
static BOTTOM_HALF_HANDLER_L2: Once<fn(u8)> = Once::new();

pub(super) fn process(irq_num: u8) {
    match InterruptLevel::current() {
        InterruptLevel::L1(_) => process_l1(irq_num),
        InterruptLevel::L2 => process_l2(irq_num),
        _ => unreachable!("this function must have been call in interrupt context"),
    }
}

fn process_l1(irq_num: u8) {
    let Some(handler) = BOTTOM_HALF_HANDLER_L1.get() else {
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
    let irq_guard = handler(irq_guard, irq_num);

    // Interrupts should remain disabled when `process_bottom_half` returns,
    // so we simply forget the guard.
    core::mem::forget(irq_guard);
    drop(preempt_guard);
}

fn process_l2(irq_num: u8) {
    let Some(handler) = BOTTOM_HALF_HANDLER_L2.get() else {
        return;
    };

    handler(irq_num);
}
