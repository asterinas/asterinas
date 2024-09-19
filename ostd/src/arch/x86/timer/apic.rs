// SPDX-License-Identifier: MPL-2.0

#![allow(unused_variables)]

use alloc::sync::Arc;
use core::{
    arch::x86_64::_rdtsc,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

use log::info;
use spin::Once;
use x86::{
    cpuid::cpuid,
    msr::{wrmsr, IA32_TSC_DEADLINE},
};

use super::TIMER_FREQ;
use crate::{
    arch::{
        kernel::tsc::init_tsc_freq,
        timer::pit::OperatingMode,
        x86::kernel::{
            apic::{self, DivideConfig},
            tsc::TSC_FREQ,
        },
    },
    trap::{IrqLine, TrapFrame},
};

/// Initializes APIC with tsc deadline mode or periodic mode.
/// Return the corresponding [`IrqLine`] for the System Timer.
pub(super) fn init() -> IrqLine {
    init_tsc_freq();
    if is_tsc_deadline_mode_supported() {
        info!("[Timer]: Enable APIC TSC deadline mode.");
        init_tsc_mode()
    } else {
        info!("[Timer]: Enable APIC periodic mode.");
        init_periodic_mode()
    }
}

pub(super) static APIC_TIMER_CALLBACK: Once<Arc<dyn Fn() + Sync + Send>> = Once::new();

/// Determines if the current system supports tsc_deadline mode APIC timer
fn is_tsc_deadline_mode_supported() -> bool {
    const TSC_DEADLINE_MODE_SUPPORT: u32 = 1 << 24;
    let cpuid = cpuid!(1);
    (cpuid.ecx & TSC_DEADLINE_MODE_SUPPORT) > 0
}

fn init_tsc_mode() -> IrqLine {
    let timer_irq = IrqLine::alloc().unwrap();
    // Enable tsc deadline mode
    apic::with_borrow(|apic| {
        apic.set_lvt_timer(timer_irq.num() as u64 | (1 << 18));
    });
    let tsc_step = TSC_FREQ.load(Ordering::Relaxed) / TIMER_FREQ;

    let callback = move || unsafe {
        let tsc_value = _rdtsc();
        let next_tsc_value = tsc_step + tsc_value;
        wrmsr(IA32_TSC_DEADLINE, next_tsc_value);
    };

    callback.call(());
    APIC_TIMER_CALLBACK.call_once(|| Arc::new(callback));

    timer_irq
}

fn init_periodic_mode() -> IrqLine {
    // Allocate IRQ
    let mut irq = IrqLine::alloc().unwrap();
    irq.on_active(pit_callback);

    // Enable PIT
    super::pit::init(OperatingMode::RateGenerator);
    super::pit::enable_ioapic_line(irq.clone());

    // Set APIC timer count
    apic::with_borrow(|apic| {
        apic.set_timer_div_config(DivideConfig::Divide64);
        apic.set_timer_init_count(0xFFFF_FFFF);
    });

    static IS_FINISH: AtomicBool = AtomicBool::new(false);
    static INIT_COUNT: AtomicU64 = AtomicU64::new(0);

    x86_64::instructions::interrupts::enable();
    while !IS_FINISH.load(Ordering::Acquire) {
        x86_64::instructions::hlt();
    }
    x86_64::instructions::interrupts::disable();
    drop(irq);

    // Init APIC Timer
    let timer_irq = IrqLine::alloc().unwrap();

    apic::with_borrow(|apic| {
        apic.set_timer_init_count(INIT_COUNT.load(Ordering::Relaxed));
        apic.set_lvt_timer(timer_irq.num() as u64 | (1 << 17));
        apic.set_timer_div_config(DivideConfig::Divide64);
    });

    return timer_irq;

    fn pit_callback(trap_frame: &TrapFrame) {
        static IN_TIME: AtomicU64 = AtomicU64::new(0);
        static APIC_FIRST_COUNT: AtomicU64 = AtomicU64::new(0);
        // Set a certain times of callbacks to calculate the frequency
        const CALLBACK_TIMES: u64 = TIMER_FREQ / 10;

        if IN_TIME.load(Ordering::Relaxed) < CALLBACK_TIMES || IS_FINISH.load(Ordering::Acquire) {
            if IN_TIME.load(Ordering::Relaxed) == 0 {
                let remain_ticks = apic::with_borrow(|apic| apic.timer_current_count());
                APIC_FIRST_COUNT.store(0xFFFF_FFFF - remain_ticks, Ordering::Relaxed);
            }
            IN_TIME.fetch_add(1, Ordering::Relaxed);
            return;
        }

        // Stop PIT and APIC Timer
        super::pit::disable_ioapic_line();
        let remain_ticks = apic::with_borrow(|apic| {
            let remain_ticks = apic.timer_current_count();
            apic.set_timer_init_count(0);
            remain_ticks
        });
        let ticks = (0xFFFF_FFFF - remain_ticks - APIC_FIRST_COUNT.load(Ordering::Relaxed))
            / CALLBACK_TIMES;
        info!(
            "APIC Timer ticks count:{:x}, remain ticks: {:x},Timer Freq:{} Hz",
            ticks, remain_ticks, TIMER_FREQ
        );
        INIT_COUNT.store(ticks, Ordering::Release);
        IS_FINISH.store(true, Ordering::Release);
    }
}
