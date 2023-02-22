use log::info;

use crate::{
    config,
    driver::{apic::APIC_INSTANCE, pic, timer},
    x86_64_util, TrapFrame,
};

pub fn init() {
    let apic_lock = APIC_INSTANCE.get();
    let handle = unsafe { crate::trap::IrqLine::acquire(timer::TIMER_IRQ_NUM) };
    let a = handle.on_active(init_function);
    // divide by 64
    apic_lock.divide_configuration_register.write(0b1001);
    apic_lock.initial_count_register.write(0xFFFF_FFFF);
    // apic_lock.lvt_timer_register.write(timer::TIMER_IRQ_NUM as u32);
    drop(apic_lock);

    // init pic for now, disable it after the APIC Timer init is done
    pic::enable_temp();
    timer::pit::init();

    static mut IS_FINISH: bool = false;
    // wait until it is finish
    x86_64_util::enable_interrupts();
    unsafe {
        while !IS_FINISH {
            x86_64_util::hlt();
        }
    }
    x86_64_util::disable_interrupts();
    drop(a);
    drop(handle);

    fn init_function(trap_frame: &TrapFrame) {
        static mut IN_TIME: u8 = 0;
        static mut FIRST_TIME_COUNT: u32 = 0;
        unsafe {
            if IS_FINISH || IN_TIME == 0 {
                // drop the first entry, since it may not be the time we want
                IN_TIME += 1;
                let apic_lock = APIC_INSTANCE.get();
                let remain_ticks = apic_lock.current_count_register.read();
                FIRST_TIME_COUNT = 0xFFFF_FFFF - remain_ticks;
                pic::ack();
                return;
            }
        }
        pic::disable_temp();
        // stop APIC Timer, get the number of tick we need
        let apic_lock = APIC_INSTANCE.get();
        let remain_ticks = apic_lock.current_count_register.read();
        apic_lock.initial_count_register.write(0);
        let ticks = unsafe { 0xFFFF_FFFF - remain_ticks - FIRST_TIME_COUNT };
        // periodic mode, divide 64, freq: TIMER_FREQ Hz
        apic_lock.initial_count_register.write(ticks as u32);
        apic_lock
            .lvt_timer_register
            .write(timer::TIMER_IRQ_NUM as u32 | (1 << 17));
        apic_lock.divide_configuration_register.write(0b1001);

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
