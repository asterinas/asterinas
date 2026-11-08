// SPDX-License-Identifier: MPL-2.0

pub(crate) use cpu_clock::ProfClock;
#[cfg(ktest)]
pub(crate) use system_wide::init_for_ktest;
pub(crate) use system_wide::{
    BootTimeClock, JIFFIES_TIMER_MANAGER, MonotonicClock, MonotonicCoarseClock, MonotonicRawClock,
    RealTimeClock, RealTimeCoarseClock,
};

mod cpu_clock;
mod system_wide;

pub(super) fn init() {
    system_wide::init();
}
