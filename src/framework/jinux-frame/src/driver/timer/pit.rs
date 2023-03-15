//! used for PIT Timer

use spin::Mutex;
use x86_64::instructions::port::PortWriteOnly;

use crate::config::TIMER_FREQ;

const TIMER_RATE: u32 = 1193182;

static TIMER_PERIOD: Mutex<PortWriteOnly<u8>> = Mutex::new(PortWriteOnly::new(0x40));
static TIMER_MOD: Mutex<PortWriteOnly<u8>> = Mutex::new(PortWriteOnly::new(0x43));
static TIMER_SQUARE_WAVE: u8 = 0x36;

pub(crate) fn init() {
    // Initialize timer.
    let cycle = TIMER_RATE / TIMER_FREQ as u32;
    unsafe {
        TIMER_MOD.lock().write(TIMER_SQUARE_WAVE);
        TIMER_PERIOD.lock().write((cycle & 0xFF) as _);
        TIMER_PERIOD.lock().write((cycle >> 8) as _);
    }
}
