//! used for PIT Timer

use crate::{config::TIMER_FREQ, x86_64_util::out8};

const TIMER_RATE: u32 = 1193182;

const TIMER_PERIOD_IO_PORT: u16 = 0x40;
const TIMER_MODE_IO_PORT: u16 = 0x43;
const TIMER_SQUARE_WAVE: u8 = 0x36;

pub(crate) fn init() {
    // Initialize timer.
    let cycle = TIMER_RATE / TIMER_FREQ as u32;
    out8(TIMER_MODE_IO_PORT, TIMER_SQUARE_WAVE);
    out8(TIMER_PERIOD_IO_PORT, (cycle & 0xFF) as _);
    out8(TIMER_PERIOD_IO_PORT, (cycle >> 8) as _);
}
