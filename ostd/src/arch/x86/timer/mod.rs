// SPDX-License-Identifier: MPL-2.0

//! The timer support.

mod apic;
mod hpet;
pub(in crate::arch) mod pit;

use spin::Once;

use super::trap::TrapFrame;
use crate::{arch::kernel, irq::IrqLine};

static TIMER_IRQ: Once<IrqLine> = Once::new();

/// Initializes the timer state and enable timer interrupts on BSP.
pub(super) fn init_bsp() {
    let mut timer_irq = if kernel::apic::exists() {
        apic::init_bsp()
    } else {
        pit::init(pit::OperatingMode::SquareWaveGenerator);

        /// In PIT mode, channel 0 is connected directly to IRQ0, which is
        /// the `IrqLine` with the `irq_num` 32 (0-31 `IrqLine`s are reserved).
        ///
        /// Ref: https://wiki.osdev.org/Programmable_Interval_Timer#Outputs.
        const PIT_MODE_TIMER_IRQ_NUM: u8 = 32;

        IrqLine::alloc_specific(PIT_MODE_TIMER_IRQ_NUM).unwrap()
    };

    timer_irq.on_active(timer_callback);

    TIMER_IRQ.call_once(|| timer_irq);
}

/// Enables timer interrupt on this AP.
pub(super) fn init_ap() {
    if kernel::apic::exists() {
        apic::init_ap(TIMER_IRQ.get().unwrap());
    }
}

fn timer_callback(trapframe: &TrapFrame) {
    crate::timer::call_timer_callback_functions(trapframe);

    apic::timer_callback();
}
