// SPDX-License-Identifier: MPL-2.0

//! Timer support backed by the ARM generic timer (the EL1 physical timer,
//! `CNTP_*_EL0`).
//!
//! On the QEMU `virt` machine the non-secure EL1 physical timer is wired to
//! GIC PPI 14, i.e. interrupt ID 30.

use core::sync::atomic::{AtomicU64, Ordering};

use spin::Once;

use crate::{
    arch::{irq::IRQ_CHIP, trap::TrapFrame},
    irq::IrqLine,
    timer::TIMER_FREQ,
};

/// GIC interrupt ID of the non-secure EL1 physical timer (PPI 14 + 16).
const TIMER_INTID: u8 = 30;

pub(super) static TIMER_IRQ: Once<IrqLine> = Once::new();

static TIMEBASE_FREQ: AtomicU64 = AtomicU64::new(0);
static TIMER_INTERVAL: AtomicU64 = AtomicU64::new(0);

/// `CNTP_CTL_EL0.ENABLE` (bit 0); leaving `IMASK` (bit 1) clear keeps the
/// interrupt unmasked at the timer.
const CNTP_CTL_ENABLE: u64 = 1 << 0;

fn write_cntp_tval(value: u64) {
    // SAFETY: Programming the timer countdown register is safe.
    unsafe { core::arch::asm!("msr cntp_tval_el0, {}", in(reg) value, options(nostack, nomem)) };
}

fn write_cntp_ctl(value: u64) {
    // SAFETY: Programming the timer control register is safe.
    unsafe { core::arch::asm!("msr cntp_ctl_el0, {}", in(reg) value, options(nostack, nomem)) };
}

/// Initializes the timer module on the BSP.
///
/// # Safety
///
/// Must be called once during BSP boot, before any other timer function.
pub(super) unsafe fn init_on_bsp() {
    let freq = get_timebase_freq_raw();
    TIMEBASE_FREQ.store(freq, Ordering::Relaxed);
    TIMER_INTERVAL.store(freq / TIMER_FREQ, Ordering::Relaxed);

    TIMER_IRQ.call_once(|| {
        let mut irq = IrqLine::alloc_specific(TIMER_INTID).unwrap();
        irq.on_active(timer_callback);
        irq
    });

    // Route the timer interrupt through the GIC.
    if let Some(chip) = IRQ_CHIP.get() {
        chip.enable(TIMER_INTID);
    }

    // SAFETY: Called once on the BSP during timer initialization.
    unsafe { init_current_cpu() };
}

/// Initializes the timer on this AP.
///
/// # Safety
///
/// Must be called once on each AP during boot.
pub(super) unsafe fn init_on_ap() {
    // SAFETY: Called once on this AP during timer initialization.
    unsafe { init_current_cpu() };
}

/// Arms the generic timer on the current CPU.
///
/// # Safety
///
/// Must be called once per CPU during timer initialization.
unsafe fn init_current_cpu() {
    write_cntp_tval(TIMER_INTERVAL.load(Ordering::Relaxed));
    write_cntp_ctl(CNTP_CTL_ENABLE);
}

fn timer_callback(trap_frame: &TrapFrame) {
    crate::timer::call_timer_callback_functions(trap_frame);
    // Re-arm the timer for the next tick.
    write_cntp_tval(TIMER_INTERVAL.load(Ordering::Relaxed));
}

fn get_timebase_freq_raw() -> u64 {
    let freq: u64;
    // SAFETY: Reading `CNTFRQ_EL0` has no side effects.
    unsafe { core::arch::asm!("mrs {}, cntfrq_el0", out(reg) freq, options(nostack, nomem)) };
    freq
}

/// Returns the frequency of the system counter, in Hz.
pub(crate) fn get_timebase_freq() -> u64 {
    TIMEBASE_FREQ.load(Ordering::Relaxed)
}
