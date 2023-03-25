use log::info;
use trapframe::TrapFrame;

use crate::arch::x86::kernel::{pic, xapic::XAPIC_INSTANCE};
use crate::config;

use x86::apic::xapic;

pub fn init() {
    let mut apic_lock = XAPIC_INSTANCE.get().unwrap().lock();
    let handle = unsafe { crate::trap::IrqLine::acquire(super::TIMER_IRQ_NUM) };
    let a = handle.on_active(init_function);
    // divide by 64
    apic_lock.write(xapic::XAPIC_TIMER_DIV_CONF, 0b1001);
    apic_lock.write(xapic::XAPIC_TIMER_INIT_COUNT, 0xFFFF_FFFF);
    // apic_lock.lvt_timer_register.write(timer::TIMER_IRQ_NUM as u32);
    drop(apic_lock);

    // init pic for now, disable it after the APIC Timer init is done
    pic::enable_temp();
    super::pit::init();

    static mut IS_FINISH: bool = false;
    // wait until it is finish
    x86_64::instructions::interrupts::enable();
    unsafe {
        while !IS_FINISH {
            x86_64::instructions::hlt();
        }
    }
    x86_64::instructions::interrupts::disable();
    drop(a);
    drop(handle);

    fn init_function(trap_frame: &TrapFrame) {
        static mut IN_TIME: u8 = 0;
        static mut FIRST_TIME_COUNT: u32 = 0;
        unsafe {
            if IS_FINISH || IN_TIME == 0 {
                // drop the first entry, since it may not be the time we want
                IN_TIME += 1;
                let apic_lock = XAPIC_INSTANCE.get().unwrap().lock();
                let remain_ticks = apic_lock.read(xapic::XAPIC_TIMER_CURRENT_COUNT);
                FIRST_TIME_COUNT = 0xFFFF_FFFF - remain_ticks;
                pic::ack();
                return;
            }
        }
        pic::disable_temp();
        // stop APIC Timer, get the number of tick we need
        let mut apic_lock = XAPIC_INSTANCE.get().unwrap().lock();
        let remain_ticks = apic_lock.read(xapic::XAPIC_TIMER_CURRENT_COUNT);
        apic_lock.write(xapic::XAPIC_TIMER_INIT_COUNT, 0);
        let ticks = unsafe { 0xFFFF_FFFF - remain_ticks - FIRST_TIME_COUNT };
        // periodic mode, divide 64, freq: TIMER_FREQ Hz
        apic_lock.write(xapic::XAPIC_TIMER_INIT_COUNT, ticks as u32);
        apic_lock.write(
            xapic::XAPIC_LVT_TIMER,
            super::TIMER_IRQ_NUM as u32 | (1 << 17),
        );
        apic_lock.write(xapic::XAPIC_TIMER_DIV_CONF, 0b1001);

        info!(
            "APIC Timer ticks count:{:x}, remain ticks: {:x},Timer Freq:{} Hz",
            ticks,
            remain_ticks,
            config::TIMER_FREQ
        );
        unsafe {
            IS_FINISH = true;
        }
    }
}
