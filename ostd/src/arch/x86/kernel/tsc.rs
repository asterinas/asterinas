// SPDX-License-Identifier: MPL-2.0

#![expect(unused_variables)]

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use log::info;
use x86::cpuid::cpuid;

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

/// The frequency of TSC(Hz)
pub(in crate::arch) static TSC_FREQ: AtomicU64 = AtomicU64::new(0);

pub fn init_tsc_freq() {
    let tsc_freq =
        determine_tsc_freq_via_cpuid().map_or_else(determine_tsc_freq_via_pit, |freq| freq);
    TSC_FREQ.store(tsc_freq, Ordering::Relaxed);
    info!("TSC frequency:{:?} Hz", tsc_freq);
}

/// Determines TSC frequency via CPUID. If the CPU does not support calculating TSC frequency by
/// CPUID, the function will return None. The unit of the return value is KHz.
///
/// Ref: function `native_calibrate_tsc` in linux `arch/x86/kernel/tsc.c`
///
pub fn determine_tsc_freq_via_cpuid() -> Option<u64> {
    // Check the max cpuid supported
    let cpuid = cpuid!(0);
    let max_cpuid = cpuid.eax;
    if max_cpuid <= 0x15 {
        return None;
    }

    // TSC frequecny = ecx * ebx / eax
    // CPUID 0x15: Time Stamp Counter and Nominal Core Crystal Clock Information Leaf
    let mut cpuid = cpuid!(0x15);
    if cpuid.eax == 0 || cpuid.ebx == 0 {
        return None;
    }
    let eax_denominator = cpuid.eax;
    let ebx_numerator = cpuid.ebx;
    let mut crystal_khz = cpuid.ecx / 1000;

    // Some Intel SoCs like Skylake and Kabylake don't report the crystal
    // clock, but we can easily calculate it to a high degree of accuracy
    // by considering the crystal ratio and the CPU speed.
    if crystal_khz == 0 && max_cpuid >= 0x16 {
        cpuid = cpuid!(0x16);
        let base_mhz = cpuid.eax;
        crystal_khz = base_mhz * 1000 * eax_denominator / ebx_numerator;
    }

    if crystal_khz == 0 {
        None
    } else {
        let crystal_hz = crystal_khz as u64 * 1000;
        Some(crystal_hz * ebx_numerator as u64 / eax_denominator as u64)
    }
}

/// When kernel cannot get the TSC frequency from CPUID, it can leverage
/// the PIT to calculate this frequency.
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

    fn pit_callback(trap_frame: &TrapFrame) {
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
