use core::sync::atomic::AtomicU8;
use core::sync::atomic::Ordering::Relaxed;

use crate::SystemTime;

use jinux_frame::arch::x86::device::cmos::{get_century, CMOS_ADDRESS, CMOS_DATA};
use jinux_frame::sync::Mutex;

pub(crate) static CENTURY_REGISTER: AtomicU8 = AtomicU8::new(0);

static READ_TIME: Mutex<SystemTime> = Mutex::new(SystemTime::zero());

pub fn init() {
    CENTURY_REGISTER.store(get_century(), Relaxed);
}

pub fn get_cmos(reg: u8) -> u8 {
    CMOS_ADDRESS.write(reg);
    CMOS_DATA.read()
}

pub fn is_updating() -> bool {
    CMOS_ADDRESS.write(0x0A);
    CMOS_DATA.read() & 0x80 != 0
}

pub fn read() -> SystemTime {
    update_time();
    *READ_TIME.lock()
}

/// read year,month,day and other data
/// ref: https://wiki.osdev.org/CMOS#Reading_All_RTC_Time_and_Date_Registers
fn update_time() {
    let mut last_time: SystemTime;

    let mut lock = READ_TIME.lock();

    lock.update_from_rtc();

    last_time = *lock;

    lock.update_from_rtc();

    while *lock != last_time {
        last_time = *lock;

        lock.update_from_rtc();
    }

    let register_b: u8 = get_cmos(0x0B);

    lock.convert_bcd_to_binary(register_b);
    lock.convert_12_hour_to_24_hour(register_b);
    lock.modify_year();
}
