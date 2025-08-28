// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use log::info;

use crate::{
    arch::{
        timer::{
            pit::{self, OperatingMode},
            TIMER_FREQ,
        },
        trap::TrapFrame,
    },
    trap::irq::IrqLine,
};

/// The frequency in Hz of the Time Stamp Counter (TSC).
pub(in crate::arch) static TSC_FREQ: AtomicU64 = AtomicU64::new(0);

pub fn init_tsc_freq() {
    use crate::arch::cpu::cpuid::query_tsc_freq as determine_tsc_freq_via_cpuid;

    let tsc_freq = determine_tsc_freq_via_cpuid().unwrap_or_else(determine_tsc_freq_via_pit);
    TSC_FREQ.store(tsc_freq, Ordering::Relaxed);
    info!("TSC frequency: {:?} Hz", tsc_freq);
}

/// Determines the TSC frequency with the help of the Programmable Interval Timer (PIT).
///
/// When the TSC frequency is not enumerated in the results of the CPUID instruction, it can
/// leverage the PIT to calculate the TSC frequency.
pub fn determine_tsc_freq_via_pit() -> u64 {
    // Allocate IRQ
    let mut irq = IrqLine::alloc().unwrap();
    irq.on_active(pit_callback);

    // Enable PIT
    pit::init(OperatingMode::RateGenerator);
    let irq = pit::enable_interrupt(irq);

    static IS_FINISH: AtomicBool = AtomicBool::new(false);
    static FREQUENCY: AtomicU64 = AtomicU64::new(0);

    // Wait until `FREQUENCY` is ready
    loop {
        crate::arch::irq::enable_local_and_halt();

        // Disable local IRQs so they won't come after checking `IS_FINISH`
        // but before halting the CPU.
        crate::arch::irq::disable_local();

        if IS_FINISH.load(Ordering::Acquire) {
            break;
        }
    }

    // Disable PIT
    drop(irq);

    return FREQUENCY.load(Ordering::Acquire);

    fn pit_callback(_trap_frame: &TrapFrame) {
        static IN_TIME: AtomicU64 = AtomicU64::new(0);
        static TSC_FIRST_COUNT: AtomicU64 = AtomicU64::new(0);
        // Set a certain times of callbacks to calculate the frequency
        const CALLBACK_TIMES: u64 = TIMER_FREQ / 10;

        let tsc_current_count = crate::arch::read_tsc();

        if IN_TIME.load(Ordering::Relaxed) < CALLBACK_TIMES || IS_FINISH.load(Ordering::Acquire) {
            if IN_TIME.load(Ordering::Relaxed) == 0 {
                TSC_FIRST_COUNT.store(tsc_current_count, Ordering::Relaxed);
            }
            IN_TIME.fetch_add(1, Ordering::Relaxed);
            return;
        }

        let tsc_first_count = TSC_FIRST_COUNT.load(Ordering::Relaxed);
        let freq = (tsc_current_count - tsc_first_count) * (TIMER_FREQ / CALLBACK_TIMES);
        FREQUENCY.store(freq, Ordering::Release);
        IS_FINISH.store(true, Ordering::Release);
    }
}
