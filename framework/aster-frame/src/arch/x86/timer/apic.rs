// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;
use core::{
    arch::x86_64::_rdtsc,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

use log::info;
use spin::Once;
use trapframe::TrapFrame;
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
            apic::{DivideConfig, APIC_INSTANCE},
            tsc::TSC_FREQ,
        },
    },
    trap::IrqLine,
};

pub fn init() {
    init_tsc_freq();
    if is_tsc_deadline_mode_supported() {
        info!("[Timer]: Enable APIC TSC deadline mode.");
        init_tsc_mode();
    } else {
        info!("[Timer]: Enable APIC periodic mode.");
        init_periodic_mode();
    }
}

pub(super) static APIC_TIMER_CALLBACK: Once<Arc<dyn Fn() + Sync + Send>> = Once::new();

/// Determine if the current system supports tsc_deadline mode APIC timer
fn is_tsc_deadline_mode_supported() -> bool {
    const TSC_DEADLINE_MODE_SUPPORT: u32 = 1 << 24;
    let cpuid = cpuid!(1);
    (cpuid.ecx & TSC_DEADLINE_MODE_SUPPORT) > 0
}

fn init_tsc_mode() {
    let mut apic_lock = APIC_INSTANCE.get().unwrap().lock();
    // Enable tsc deadline mode
    apic_lock.set_lvt_timer(super::TIMER_IRQ_NUM.load(Ordering::Relaxed) as u64 | (1 << 18));
    drop(apic_lock);
    let tsc_step = TSC_FREQ.load(Ordering::Relaxed) / TIMER_FREQ;

    let callback = move || unsafe {
        let tsc_value = _rdtsc();
        let next_tsc_value = tsc_step + tsc_value;
        wrmsr(IA32_TSC_DEADLINE, next_tsc_value);
    };

    callback.call(());
    APIC_TIMER_CALLBACK.call_once(|| Arc::new(callback));
}

fn init_periodic_mode() {
    // Allocate IRQ
    let mut irq = IrqLine::alloc_specific(super::TIMER_IRQ_NUM.load(Ordering::Relaxed)).unwrap();
    irq.on_active(pit_callback);

    // Enable PIT
    super::pit::init(OperatingMode::RateGenerator);
    super::pit::enable_ioapic_line(irq.clone());

    // Set APIC timer count
    let mut apic_lock = APIC_INSTANCE.get().unwrap().lock();
    apic_lock.set_timer_div_config(DivideConfig::Divide64);
    apic_lock.set_timer_init_count(0xFFFF_FFFF);
    drop(apic_lock);

    static IS_FINISH: AtomicBool = AtomicBool::new(false);
    x86_64::instructions::interrupts::enable();
    while !IS_FINISH.load(Ordering::Acquire) {
        x86_64::instructions::hlt();
    }
    x86_64::instructions::interrupts::disable();
    drop(irq);

    fn pit_callback(trap_frame: &TrapFrame) {
        static IN_TIME: AtomicU64 = AtomicU64::new(0);
        static APIC_FIRST_COUNT: AtomicU64 = AtomicU64::new(0);
        // Set a certain times of callbacks to calculate the frequency
        const CALLBACK_TIMES: u64 = TIMER_FREQ / 10;

        if IN_TIME.load(Ordering::Relaxed) < CALLBACK_TIMES || IS_FINISH.load(Ordering::Acquire) {
            if IN_TIME.load(Ordering::Relaxed) == 0 {
                let apic_lock = APIC_INSTANCE.get().unwrap().lock();
                let remain_ticks = apic_lock.timer_current_count();
                APIC_FIRST_COUNT.store(0xFFFF_FFFF - remain_ticks, Ordering::Relaxed);
            }
            IN_TIME.fetch_add(1, Ordering::Relaxed);
            return;
        }

        // Stop PIT and APIC Timer
        super::pit::disable_ioapic_line();
        let mut apic_lock = APIC_INSTANCE.get().unwrap().lock();
        let remain_ticks = apic_lock.timer_current_count();
        apic_lock.set_timer_init_count(0);

        // Init APIC Timer
        let ticks = (0xFFFF_FFFF - remain_ticks - APIC_FIRST_COUNT.load(Ordering::Relaxed))
            / CALLBACK_TIMES;
        apic_lock.set_timer_init_count(ticks);
        apic_lock.set_lvt_timer(super::TIMER_IRQ_NUM.load(Ordering::Relaxed) as u64 | (1 << 17));
        apic_lock.set_timer_div_config(DivideConfig::Divide64);
        info!(
            "APIC Timer ticks count:{:x}, remain ticks: {:x},Timer Freq:{} Hz",
            ticks, remain_ticks, TIMER_FREQ
        );
        IS_FINISH.store(true, Ordering::Release);
    }
}
