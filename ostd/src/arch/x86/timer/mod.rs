// SPDX-License-Identifier: MPL-2.0

//! The timer support.

mod apic;
mod hpet;
pub(in crate::arch) mod pit;

use spin::Once;

use super::trap::TrapFrame;
use crate::irq::IrqLine;

static TIMER_IRQ: Once<IrqLine> = Once::new();

/// Initializes the timer state and enable timer interrupts on BSP.
pub(super) fn init_on_bsp() {
    // TODO: Currently, we only enable per-CPU APIC timers. We may also need to enable a global
    // timer, such as a PIT or HPET.

    let mut timer_irq = apic::init_on_bsp();

    timer_irq.on_active(timer_callback);

    TIMER_IRQ.call_once(|| timer_irq);
}

/// Enables timer interrupt on this AP.
pub(super) fn init_on_ap() {
    apic::init_on_ap(TIMER_IRQ.get().unwrap());
}

fn timer_callback(trapframe: &TrapFrame) {
    crate::timer::call_timer_callback_functions(trapframe);

    apic::timer_callback();
}
