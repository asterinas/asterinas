// SPDX-License-Identifier: MPL-2.0

//! ARM Generic Timer support.

use core::sync::atomic::{AtomicU64, Ordering};

use spin::Once;

use crate::{irq::IrqLine, timer::TIMER_FREQ};

/// The timer IRQ line, allocated during BSP initialization.
pub(crate) static TIMER_IRQ: Once<IrqLine> = Once::new();

/// INTID for the physical timer PPI (CNTPIRQ on QEMU virt).
/// Used by `IrqChip::claim_interrupt()` to map this INTID to the
/// `TIMER_IRQ` software IRQ number.
pub(crate) const TIMER_PPI_INTID: u32 = 30;

/// Timer counter frequency in Hz (from CNTFRQ_EL0).
static TIMEBASE_FREQ: AtomicU64 = AtomicU64::new(0);
/// Timer interval in counter ticks.
static TIMER_INTERVAL: AtomicU64 = AtomicU64::new(0);

/// Initializes the timer module on the BSP.
///
/// # Safety
///
/// This function must be called once and at most once at a proper timing
/// in the boot context of the BSP.
pub(super) unsafe fn init_on_bsp() {
    let freq: u64;
    // SAFETY: Reading CNTFRQ_EL0 is always safe.
    unsafe { core::arch::asm!("mrs {0}, cntfrq_el0", out(reg) freq) };

    TIMEBASE_FREQ.store(freq, Ordering::Relaxed);
    TIMER_INTERVAL.store(freq / TIMER_FREQ, Ordering::Relaxed);

    TIMER_IRQ.call_once(|| {
        // PPI 30 (INTID 30) is the physical timer interrupt.
        // Use alloc() — irq_num is independent of INTID.
        // IrqChip::claim_interrupt() maps INTID 30 to this irq_num
        // via InterruptSource::Timer.
        let mut timer_irq = IrqLine::alloc().unwrap();
        timer_irq.on_active(timer_callback);
        timer_irq
    });

    // SAFETY: Called once during BSP init.
    unsafe { init_current_cpu() };
}

/// Initializes the timer on this AP.
///
/// # Safety
///
/// This function must be called on an AP that hasn't called this function.
pub(super) unsafe fn init_on_ap() {
    // SAFETY: The caller ensures this is only called once on this AP.
    unsafe { init_current_cpu() };
}

/// Enables the timer on the current CPU.
///
/// # Safety
///
/// Must be called once per CPU during init.
unsafe fn init_current_cpu() {
    arm_next_timer();
    // Enable the physical timer (CNTP_CTL_EL0.ENABLE = 1).
    // Use the physical timer (PPI 30, CNTPIRQ) instead of the virtual timer
    // (PPI 27, CNTVIRQ) because QEMU TCG without EL2 may not route CNTVIRQ.
    // Set ENABLE (bit 0) and clear IMASK (bit 1) to unmask the timer interrupt.
    // ISTATUS (bit 2) is read-only and doesn't need explicit clearing;
    // writing CNTP_TVAL_EL0 in arm_next_timer() already clears it.
    unsafe {
        core::arch::asm!("mrs x9, cntp_ctl_el0", "orr x9, x9, #1", "bic x9, x9, #2", "msr cntp_ctl_el0, x9", out("x9") _)
    };
}

/// Timer interrupt callback, called from IRQ context.
fn timer_callback(_trapframe: &crate::arch::trap::TrapFrame) {
    crate::timer::call_timer_callback_functions(_trapframe);
    arm_next_timer();
}

/// Programs the next timer interrupt.
fn arm_next_timer() {
    let interval = TIMER_INTERVAL.load(Ordering::Relaxed);
    // SAFETY: CNTP_TVAL_EL0 writes are always safe.
    unsafe {
        core::arch::asm!(
            "msr cntp_tval_el0, {0}",
            "isb",
            in(reg) interval,
        );
    }
}

/// Returns the timer frequency in Hz.
pub(crate) fn get_timer_freq() -> u64 {
    TIMEBASE_FREQ.load(Ordering::Relaxed)
}

/// Reads the current counter value.
pub(crate) fn read_counter() -> u64 {
    let cnt: u64;
    // SAFETY: Reading CNTPCT_EL0 is always safe.
    unsafe { core::arch::asm!("isb", "mrs {0}, cntpct_el0", out(reg) cnt) };
    cnt
}
