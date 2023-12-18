use core::sync::atomic::AtomicU8;
use core::sync::atomic::Ordering::Relaxed;
use jinux_frame::arch::x86::device::cmos::{get_century_register, CMOS_ADDRESS, CMOS_DATA};

pub(crate) static CENTURY_REGISTER: AtomicU8 = AtomicU8::new(0);

pub fn init() {
    let Some(century_register) = get_century_register() else {
        return;
    };
    CENTURY_REGISTER.store(century_register, Relaxed);
}

pub fn get_cmos(reg: u8) -> u8 {
    CMOS_ADDRESS.write(reg);
    CMOS_DATA.read()
}

pub fn is_updating() -> bool {
    CMOS_ADDRESS.write(0x0A);
    CMOS_DATA.read() & 0x80 != 0
}
