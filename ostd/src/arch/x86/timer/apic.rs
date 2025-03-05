// SPDX-License-Identifier: MPL-2.0

#![expect(unused_variables)]

use core::{
    arch::x86_64::_rdtsc,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

use log::info;
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

pub(super) static IN_APIC_TSC_DEADLINE_MODE: AtomicBool = AtomicBool::new(false);

/// Initializes APIC with tsc deadline mode or periodic mode.
/// Return the corresponding [`IrqLine`] for the System Timer.
pub(super) fn init_global_and_bsp() -> IrqLine {
    init_tsc_freq();

    if is_tsc_deadline_mode_supported() {
        info!("[Timer]: Enable APIC TSC deadline mode.");

        let timer_irq = IrqLine::alloc().unwrap();

        IN_APIC_TSC_DEADLINE_MODE.store(true, Ordering::Relaxed);

        init_tsc_mode(&timer_irq);

        timer_irq
    } else {
        info!("[Timer]: Enable APIC periodic mode.");

        let mut timer_irq = IrqLine::alloc().unwrap();
        timer_irq.on_active(pit_callback);

        init_periodic_mode_global(&timer_irq);
        init_periodic_mode(&timer_irq);

        timer_irq
    }
}

pub(super) fn init_ap(timer_irq: &IrqLine) {
    if is_tsc_deadline_mode_supported() {
        init_tsc_mode(timer_irq);
    } else {
        init_periodic_mode(timer_irq);
    }
}

/// Determines if the current system supports tsc_deadline mode APIC timer
fn is_tsc_deadline_mode_supported() -> bool {
    const TSC_DEADLINE_MODE_SUPPORT: u32 = 1 << 24;
    let cpuid = cpuid!(1);
    (cpuid.ecx & TSC_DEADLINE_MODE_SUPPORT) > 0
}

fn init_tsc_mode(timer_irq: &IrqLine) {
    // Enable tsc deadline mode
    apic::with_borrow(|apic| {
        apic.set_lvt_timer(timer_irq.num() as u64 | (1 << 18));
    });

    apic_tsc_deadline_mode_callback();
}

pub(super) fn apic_tsc_deadline_mode_callback() {
    let tsc_value = unsafe { _rdtsc() };
    let tsc_step = TSC_FREQ.load(Ordering::Relaxed) / TIMER_FREQ;
    let next_tsc_value = tsc_step + tsc_value;
    unsafe { wrmsr(IA32_TSC_DEADLINE, next_tsc_value) };
}

fn init_periodic_mode_global(timer_irq: &IrqLine) {
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

    x86_64::instructions::interrupts::enable();
    while !IS_FINISH.load(Ordering::Acquire) {
        x86_64::instructions::hlt();
    }
    x86_64::instructions::interrupts::disable();
    drop(irq);
}

fn init_periodic_mode(timer_irq: &IrqLine) {
    apic::with_borrow(|apic| {
        apic.set_timer_init_count(INIT_COUNT.load(Ordering::Relaxed));
        apic.set_lvt_timer(timer_irq.num() as u64 | (1 << 17));
        apic.set_timer_div_config(DivideConfig::Divide64);
    });
}

static IS_FINISH: AtomicBool = AtomicBool::new(false);
static INIT_COUNT: AtomicU64 = AtomicU64::new(0);

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
    let ticks =
        (0xFFFF_FFFF - remain_ticks - APIC_FIRST_COUNT.load(Ordering::Relaxed)) / CALLBACK_TIMES;
    info!(
        "APIC Timer ticks count:{:x}, remain ticks: {:x},Timer Freq:{} Hz",
        ticks, remain_ticks, TIMER_FREQ
    );
    INIT_COUNT.store(ticks, Ordering::Release);
    IS_FINISH.store(true, Ordering::Release);
}
