// SPDX-License-Identifier: MPL-2.0

//! used for PIT Timer

use crate::config::TIMER_FREQ;

use crate::arch::x86::device::io_port::{IoPort, WriteOnlyAccess};

const TIMER_RATE: u32 = 1193182;

static TIMER_PERIOD: IoPort<u8, WriteOnlyAccess> = unsafe { IoPort::new(0x40) };
static TIMER_MOD: IoPort<u8, WriteOnlyAccess> = unsafe { IoPort::new(0x43) };
static TIMER_SQUARE_WAVE: u8 = 0x34;

pub(crate) fn init() {
    // Initialize timer.
    let cycle = TIMER_RATE / TIMER_FREQ as u32;
    TIMER_MOD.write(TIMER_SQUARE_WAVE);
    TIMER_PERIOD.write((cycle & 0xFF) as _);
    TIMER_PERIOD.write((cycle >> 8) as _);
}
