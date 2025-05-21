// SPDX-License-Identifier: MPL-2.0

use core::{
    arch::x86_64::_rdtsc,
    sync::atomic::{AtomicU64, Ordering},
};

use log::info;

use super::TIMER_FREQ;
use crate::{
    arch::{
        kernel::apic::{self, DivideConfig},
        timer::pit::OperatingMode,
        tsc_freq,
    },
    trap::{IrqLine, TrapFrame},
};

/// Initializes APIC with TSC-deadline mode or periodic mode.
///
/// Return the corresponding [`IrqLine`] for the system timer.
pub(super) fn init_bsp() -> IrqLine {
    if is_tsc_deadline_mode_supported() {
        init_deadline_mode_config();
    } else {
        init_periodic_mode_config();
    }

    let timer_irq = IrqLine::alloc().unwrap();
    init_timer(&timer_irq);
    timer_irq
}

/// Initializes APIC timer on AP.
///
/// The caller should provide the [`IrqLine`] for the system timer.
pub(super) fn init_ap(timer_irq: &IrqLine) {
    init_timer(timer_irq);
}

/// A callback that needs to be called on timer interrupt.
pub(super) fn timer_callback() {
    use x86::msr::{wrmsr, IA32_TSC_DEADLINE};

    match CONFIG.get().expect("ACPI timer config is not initialized") {
        Config::DeadlineMode { tsc_interval } => {
            let tsc_value = unsafe { _rdtsc() };
            let next_tsc_value = tsc_interval + tsc_value;
            unsafe { wrmsr(IA32_TSC_DEADLINE, next_tsc_value) };
        }
        Config::PeriodicMode { .. } => {}
    }
}

/// Determines if the current system supports tsc_deadline mode APIC timer
fn is_tsc_deadline_mode_supported() -> bool {
    use x86::cpuid::cpuid;

    const TSC_DEADLINE_MODE_SUPPORT: u32 = 1 << 24;
    let cpuid = cpuid!(1);
    (cpuid.ecx & TSC_DEADLINE_MODE_SUPPORT) > 0
}

fn init_timer(timer_irq: &IrqLine) {
    match CONFIG.get().expect("ACPI timer config is not initialized") {
        Config::DeadlineMode { .. } => {
            init_deadline_mode(timer_irq);
        }
        Config::PeriodicMode { init_count } => {
            init_periodic_mode(timer_irq, *init_count);
        }
    }
}

fn init_deadline_mode(timer_irq: &IrqLine) {
    // Enable tsc deadline mode
    apic::with_borrow(|apic| {
        apic.set_lvt_timer(timer_irq.num() as u64 | (1 << 18));
    });

    timer_callback();
}

fn init_periodic_mode(timer_irq: &IrqLine, init_count: u64) {
    apic::with_borrow(|apic| {
        apic.set_timer_init_count(init_count);
        apic.set_lvt_timer(timer_irq.num() as u64 | (1 << 17));
        apic.set_timer_div_config(DivideConfig::Divide64);
    });
}

static CONFIG: spin::Once<Config> = spin::Once::new();

enum Config {
    DeadlineMode { tsc_interval: u64 },
    PeriodicMode { init_count: u64 },
}

fn init_deadline_mode_config() {
    info!("[Timer]: Enable APIC TSC deadline mode");

    let tsc_interval = tsc_freq() / TIMER_FREQ;
    CONFIG.call_once(|| Config::DeadlineMode { tsc_interval });
}

fn init_periodic_mode_config() {
    info!("[Timer]: Enable APIC periodic mode");

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
    while !CONFIG.is_completed() {
        x86_64::instructions::hlt();
    }
    x86_64::instructions::interrupts::disable();
    drop(irq);

    fn pit_callback(_trap_frame: &TrapFrame) {
        static IN_TIME: AtomicU64 = AtomicU64::new(0);
        static APIC_FIRST_COUNT: AtomicU64 = AtomicU64::new(0);
        // Set a certain times of callbacks to calculate the frequency
        // The number of callbacks needed to calculate the APIC timer frequency.
        // This is set to 1/10th of the TIMER_FREQ to ensure enough samples for accurate calculation.
        const CALLBACK_TIMES: u64 = TIMER_FREQ / 10;

        if IN_TIME.load(Ordering::Relaxed) < CALLBACK_TIMES || CONFIG.is_completed() {
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
        CONFIG.call_once(|| Config::PeriodicMode { init_count: ticks });
    }
}
