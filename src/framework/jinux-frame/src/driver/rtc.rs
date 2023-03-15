use core::sync::atomic::AtomicU8;
use core::sync::atomic::Ordering::Relaxed;

use acpi::{fadt::Fadt, sdt::Signature};
use lazy_static::lazy_static;
use spin::Mutex;
use x86_64::instructions::port::{PortReadOnly, PortWriteOnly};

use crate::time::Time;

use super::acpi::ACPI_TABLES;

static CMOS_ADDRESS: Mutex<PortWriteOnly<u8>> = Mutex::new(PortWriteOnly::new(0x70));
static CMOS_DATA: Mutex<PortReadOnly<u8>> = Mutex::new(PortReadOnly::new(0x71));

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
    unsafe {
        CMOS_ADDRESS.lock().write(reg);
        CMOS_DATA.lock().read()
    }
}

pub fn is_updating() -> bool {
    unsafe {
        CMOS_ADDRESS.lock().write(0x0A);
        CMOS_DATA.lock().read() & 0x80 != 0
    }
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
