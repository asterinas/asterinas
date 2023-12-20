use alloc::sync::Arc;
use core::arch::x86_64::_rdtsc;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use log::info;
use spin::Once;
use trapframe::TrapFrame;
use x86::msr::{wrmsr, IA32_TSC_DEADLINE};

use crate::arch::kernel::apic::ioapic::IO_APIC;
use crate::arch::kernel::tsc::is_tsc_deadline_mode_supported;
use crate::arch::x86::kernel::apic::{DivideConfig, APIC_INSTANCE};
use crate::arch::x86::kernel::tsc::{determine_tsc_freq_via_cpuid, TSC_FREQ};
use crate::config::TIMER_FREQ;
use crate::trap::IrqLine;

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

fn init_tsc_freq() {
    let tsc_freq = determine_tsc_freq_via_cpuid()
        .map_or(determine_tsc_freq_via_pit(), |freq| freq as u64 * 1000);
    TSC_FREQ.store(tsc_freq, Ordering::Relaxed);
    info!("TSC frequency:{:?} Hz", tsc_freq);
}

fn init_tsc_mode() {
    let mut apic_lock = APIC_INSTANCE.get().unwrap().lock();
    // Enable tsc deadline mode.
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

/// When kernel cannot get the TSC frequency from CPUID, it can leverage
/// the PIT to calculate this frequency.
fn determine_tsc_freq_via_pit() -> u64 {
    let mut irq = IrqLine::alloc_specific(super::TIMER_IRQ_NUM.load(Ordering::Relaxed)).unwrap();
    irq.on_active(pit_callback);
    let mut io_apic = IO_APIC.get().unwrap().get(0).unwrap().lock();
    debug_assert_eq!(io_apic.interrupt_base(), 0);
    io_apic.enable(2, irq.clone()).unwrap();
    drop(io_apic);

    super::pit::init();

    x86_64::instructions::interrupts::enable();
    static IS_FINISH: AtomicBool = AtomicBool::new(false);
    static FREQUENCY: AtomicU64 = AtomicU64::new(0);
    while !IS_FINISH.load(Ordering::Acquire) {
        x86_64::instructions::hlt();
    }
    x86_64::instructions::interrupts::disable();
    drop(irq);
    return FREQUENCY.load(Ordering::Acquire);

    fn pit_callback(trap_frame: &TrapFrame) {
        static mut IN_TIME: u64 = 0;
        static mut TSC_FIRST_COUNT: u64 = 0;
        // Set a certain times of callbacks to calculate the frequency.
        const CALLBACK_TIMES: u64 = TIMER_FREQ / 10;
        unsafe {
            if IN_TIME < CALLBACK_TIMES || IS_FINISH.load(Ordering::Acquire) {
                // drop the first entry, since it may not be the time we want
                if IN_TIME == 0 {
                    TSC_FIRST_COUNT = _rdtsc();
                }
                IN_TIME += 1;
                return;
            }
            let mut io_apic = IO_APIC.get().unwrap().get(0).unwrap().lock();
            io_apic.disable(2).unwrap();
            drop(io_apic);
            let tsc_count = _rdtsc();
            let freq = (tsc_count - TSC_FIRST_COUNT) * (TIMER_FREQ / CALLBACK_TIMES);
            FREQUENCY.store(freq, Ordering::Release);
        }
        IS_FINISH.store(true, Ordering::Release);
    }
}

fn init_periodic_mode() {
    let mut apic_lock = APIC_INSTANCE.get().unwrap().lock();
    let mut irq = IrqLine::alloc_specific(super::TIMER_IRQ_NUM.load(Ordering::Relaxed)).unwrap();
    irq.on_active(init_function);
    let mut io_apic = IO_APIC.get().unwrap().get(0).unwrap().lock();
    debug_assert_eq!(io_apic.interrupt_base(), 0);
    io_apic.enable(2, irq.clone()).unwrap();
    drop(io_apic);
    // divide by 64
    apic_lock.set_timer_div_config(DivideConfig::Divide64);
    apic_lock.set_timer_init_count(0xFFFF_FFFF);
    drop(apic_lock);
    super::pit::init();
    // wait until it is finish
    x86_64::instructions::interrupts::enable();
    static IS_FINISH: AtomicBool = AtomicBool::new(false);
    while !IS_FINISH.load(Ordering::Acquire) {
        x86_64::instructions::hlt();
    }
    x86_64::instructions::interrupts::disable();
    drop(irq);

    fn init_function(trap_frame: &TrapFrame) {
        static mut IN_TIME: u8 = 0;
        static mut FIRST_TIME_COUNT: u64 = 0;
        unsafe {
            if IS_FINISH.load(Ordering::Acquire) || IN_TIME == 0 {
                // drop the first entry, since it may not be the time we want
                IN_TIME += 1;
                let apic_lock = APIC_INSTANCE.get().unwrap().lock();
                let remain_ticks = apic_lock.timer_current_count();
                FIRST_TIME_COUNT = 0xFFFF_FFFF - remain_ticks;
                return;
            }
        }
        let mut io_apic = IO_APIC.get().unwrap().get(0).unwrap().lock();
        io_apic.disable(2).unwrap();
        drop(io_apic);
        // stop APIC Timer, get the number of tick we need
        let mut apic_lock = APIC_INSTANCE.get().unwrap().lock();
        let remain_ticks = apic_lock.timer_current_count();
        apic_lock.set_timer_init_count(0);
        let ticks = unsafe { 0xFFFF_FFFF - remain_ticks - FIRST_TIME_COUNT };
        // periodic mode, divide 64, freq: TIMER_FREQ Hz
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
