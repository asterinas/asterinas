use core::sync::atomic::AtomicU8;
use core::sync::atomic::Ordering::Relaxed;

use acpi::{fadt::Fadt, sdt::Signature};
use lazy_static::lazy_static;
use spin::Mutex;

use crate::{
    time::Time,
    x86_64_util::{in8, out8},
};

use super::acpi::ACPI_TABLES;

const CMOS_ADDRESS: u16 = 0x70;
const CMOS_DATA: u16 = 0x71;
pub(crate) static CENTURY_REGISTER: AtomicU8 = AtomicU8::new(0);

lazy_static! {
    static ref READ_TIME: Mutex<Time> = Mutex::new(Time::default());
}

pub fn init() {
    let c = ACPI_TABLES.lock();
    let r_century = unsafe {
        let a = c
            .get_sdt::<Fadt>(Signature::FADT)
            .unwrap()
            .expect("not found FACP in ACPI table");
        a.century
    };
    CENTURY_REGISTER.store(r_century, Relaxed);
}

pub fn get_cmos(reg: u8) -> u8 {
    out8(CMOS_ADDRESS, reg as u8);
    in8(CMOS_DATA)
}

pub fn is_updating() -> bool {
    out8(CMOS_ADDRESS, 0x0A);
    in8(CMOS_DATA) & 0x80 != 0
}

pub fn read() -> Time {
    update_time();
    READ_TIME.lock().clone()
}

/// read year,month,day and other data
/// ref: https://wiki.osdev.org/CMOS#Reading_All_RTC_Time_and_Date_Registers
fn update_time() {
    let mut last_time: Time;

    let register_b: u8;
    let mut lock = READ_TIME.lock();

    lock.update_from_rtc();

    last_time = lock.clone();

    lock.update_from_rtc();

    while *lock != last_time {
        last_time = lock.clone();

        lock.update_from_rtc();
    }

    register_b = get_cmos(0x0B);

    lock.convert_bcd_to_binary(register_b);
    lock.convert_12_hour_to_24_hour(register_b);
    lock.modify_year();
}
