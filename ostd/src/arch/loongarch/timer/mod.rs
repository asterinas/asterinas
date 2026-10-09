// SPDX-License-Identifier: MPL-2.0

//! The timer support.

use loongArch64::register::{
    ecfg::{self, LineBasedInterrupt},
    tcfg, ticlr,
};
use spin::Once;

use crate::{
    arch::{
        self,
        irq::{HwIrqLine, InterruptSource},
        trap::TrapFrame,
    },
    cpu::PrivilegeLevel,
    irq::{self, IrqLine},
    timer::TIMER_FREQ,
};

static TIMER_IRQ: Once<IrqLine> = Once::new();

/// Initializes the timer on the BSP.
///
/// # Safety
///
/// This function must be called only once on the BSP, with local IRQs disabled,
/// after trap handling has been initialized and before timer interrupts can occur.
pub(super) unsafe fn init_on_bsp() {
    TIMER_IRQ.call_once(|| {
        let mut timer_irq = IrqLine::alloc().unwrap();
        timer_irq.on_active(timer_callback);
        timer_irq
    });

    // `TCFG`'s initial countdown value is expressed in multiples of four counter cycles.
    // Reference: <https://loongson.github.io/LoongArch-Documentation/LoongArch-Vol1-EN.html#timer-configuration>
    let interval_cycles = ((arch::tsc_freq() / TIMER_FREQ) as usize) & !0b11;
    assert!(
        interval_cycles > 0,
        "TSC frequency is too low for the timer frequency"
    );

    // TODO: `TCFG`, `TICLR` and `ECFG.LIE` are per-hart. Program them on each AP
    // once `arch::init_on_ap` is implemented, mirroring the RISC-V port's
    // `timer::init_on_ap`. Today only the BSP reaches this code.
    tcfg::set_en(false);
    ticlr::clear_timer_interrupt();
    tcfg::set_init_val(interval_cycles);
    tcfg::set_periodic(true);
    tcfg::set_en(true);

    ecfg::set_lie(ecfg::read().lie() | LineBasedInterrupt::TIMER);
}

pub(super) fn handle_irq(trap_frame: &TrapFrame, privilege_level: PrivilegeLevel) {
    irq::call_irq_callback_functions(
        trap_frame,
        &HwIrqLine::new(TIMER_IRQ.get().unwrap().num(), InterruptSource::Timer),
        privilege_level,
    );
}

fn timer_callback(trapframe: &TrapFrame) {
    crate::timer::call_timer_callback_functions(trapframe);
}
