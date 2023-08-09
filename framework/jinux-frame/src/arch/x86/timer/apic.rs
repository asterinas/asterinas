use core::arch::x86_64::_rdtsc;
use core::sync::atomic::{AtomicBool, Ordering};

use alloc::boxed::Box;
use alloc::sync::Arc;
use log::info;
use spin::Once;
use trapframe::TrapFrame;
use x86::cpuid::cpuid;
use x86::msr::{wrmsr, IA32_TSC_DEADLINE};

use crate::{
    arch::x86::kernel::{
        apic::{DivideConfig, APIC_INSTANCE},
        pic,
        tsc::tsc_freq,
    },
    config::TIMER_FREQ,
};

pub fn init() {
    if tsc_mode_support() {
        info!("APIC Timer: Enable TSC deadline mode.");
        tsc_mode_init();
    } else {
        info!("APIC Timer: Enable periodic mode.");
        periodic_mode_init();
    }
}

fn tsc_mode_support() -> bool {
    let tsc_rate = tsc_freq();
    if tsc_rate.is_none() {
        return false;
    }
    let cpuid = cpuid!(0x1);
    // bit 24
    cpuid.ecx & 0x100_0000 != 0
}

pub(super) static APIC_TIMER_CALLBACK: Once<Arc<dyn Fn() + Sync + Send>> = Once::new();

fn tsc_mode_init() {
    let mut apic_lock = APIC_INSTANCE.get().unwrap().lock();
    // Enable tsc deadline mode.
    apic_lock.set_lvt_timer(super::TIMER_IRQ_NUM as u64 | (1 << 18));

    let tsc_step = {
        let tsc_rate = tsc_freq().unwrap() as u64;
        info!("TSC frequency:{:?} Hz", tsc_rate * 1000);
        tsc_rate * 1000 / TIMER_FREQ
    };

    let callback = move || unsafe {
        let tsc_value = _rdtsc();
        let next_tsc_value = tsc_step + tsc_value;
        wrmsr(IA32_TSC_DEADLINE, next_tsc_value);
    };

    callback.call(());
    APIC_TIMER_CALLBACK.call_once(|| Arc::new(callback));
}

fn periodic_mode_init() {
    let mut apic_lock = APIC_INSTANCE.get().unwrap().lock();
    let handle = unsafe { crate::trap::IrqLine::acquire(super::TIMER_IRQ_NUM) };
    let a = handle.on_active(init_function);
    // divide by 64
    apic_lock.set_timer_div_config(DivideConfig::Divide64);
    apic_lock.set_timer_init_count(0xFFFF_FFFF);
    drop(apic_lock);

    // init pic for now, disable it after the APIC Timer init is done
    pic::enable_temp();
    super::pit::init();

    // wait until it is finish
    x86_64::instructions::interrupts::enable();
    static IS_FINISH: AtomicBool = AtomicBool::new(false);
    while !IS_FINISH.load(Ordering::Acquire) {
        x86_64::instructions::hlt();
    }
    x86_64::instructions::interrupts::disable();
    drop(a);
    drop(handle);

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
                pic::ack();
                return;
            }
        }
        pic::disable_temp();
        // stop APIC Timer, get the number of tick we need
        let mut apic_lock = APIC_INSTANCE.get().unwrap().lock();
        let remain_ticks = apic_lock.timer_current_count();
        apic_lock.set_timer_init_count(0);
        let ticks = unsafe { 0xFFFF_FFFF - remain_ticks - FIRST_TIME_COUNT };
        // periodic mode, divide 64, freq: TIMER_FREQ Hz
        apic_lock.set_timer_init_count(ticks);
        apic_lock.set_lvt_timer(super::TIMER_IRQ_NUM as u64 | (1 << 17));
        apic_lock.set_timer_div_config(DivideConfig::Divide64);

        info!(
            "APIC Timer ticks count:{:x}, remain ticks: {:x},Timer Freq:{} Hz",
            ticks, remain_ticks, TIMER_FREQ
        );
        IS_FINISH.store(true, Ordering::Release);
    }
}
