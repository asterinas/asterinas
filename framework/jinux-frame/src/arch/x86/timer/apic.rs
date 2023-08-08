use core::sync::atomic::{AtomicBool, Ordering};
use log::info;
use trapframe::TrapFrame;

use crate::arch::x86::kernel::{
    apic::{DivideConfig, APIC_INSTANCE},
    pic,
};
use crate::config;

pub fn init() {
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
            ticks,
            remain_ticks,
            config::TIMER_FREQ
        );
        IS_FINISH.store(true, Ordering::Release);
    }
}
