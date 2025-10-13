// SPDX-License-Identifier: MPL-2.0

//! This module implements the CPU load average calculation.
//!
//! Reference: <https://github.com/torvalds/linux/blob/46132e3/kernel/sched/loadavg.c>

use core::sync::atomic::{AtomicU64, Ordering::Relaxed};

use aster_util::fixed_point::FixedU32;
use ostd::{
    sync::RwLock,
    timer::{self, TIMER_FREQ},
};

/// Fixed-point representation of the load average.
///
/// This is an equivalent of an u32 with 21 bits for the integer part and 11 bits for the fractional part.
pub type LoadAvgFixed = FixedU32<11>;

/// 5 sec intervals
const LOAD_FREQ: u64 = 5 * TIMER_FREQ + 1;
/// 1/exp(5sec/1min) as fixed-point
const EXP_1: LoadAvgFixed = LoadAvgFixed::from_raw(1884);
/// 1/exp(5sec/5min)
const EXP_5: LoadAvgFixed = LoadAvgFixed::from_raw(2014);
/// 1/exp(5sec/15min)
const EXP_15: LoadAvgFixed = LoadAvgFixed::from_raw(2037);

/// Load average of all CPU cores.
///
/// The load average is calculated as an exponential moving average of the load
/// over the last 1, 5, and 15 minutes.
static LOAD_AVG: RwLock<[LoadAvgFixed; 3]> = RwLock::new([LoadAvgFixed::ZERO; 3]);

/// Next time the load average will be updated (in jiffies).
static LOAD_AVG_NEXT_UPDATE: AtomicU64 = AtomicU64::new(0);

/// Returns the calculated load average of the system.
pub fn get_loadavg() -> [LoadAvgFixed; 3] {
    *LOAD_AVG.read()
}

/// Updates the load average of the system.
///
/// This function should be called periodically to update the load average.
/// The `get_load` function should return the load (the number of queued tasks) of the system.
/// See `sched::stats::scheduler_stats::set_stats_from_scheduler()` for an example.
pub fn update_loadavg<F>(get_load: F)
where
    F: Fn() -> u32,
{
    let jiffies = timer::Jiffies::elapsed().as_u64();

    // Return if the load average was updated less than 5 seconds ago.
    if jiffies < LOAD_AVG_NEXT_UPDATE.load(Relaxed) {
        return;
    }

    // Update the next time the load average will be updated (now + 5sec)
    LOAD_AVG_NEXT_UPDATE.store(jiffies + LOAD_FREQ, Relaxed);

    // Get the fixed-point representation of the load
    let new_load = LoadAvgFixed::saturating_from_num(get_load());

    let mut load = LOAD_AVG.write();

    // Calculate the new load average
    load[0] = calc_loadavg(load[0], EXP_1, new_load);
    load[1] = calc_loadavg(load[1], EXP_5, new_load);
    load[2] = calc_loadavg(load[2], EXP_15, new_load);
}

fn calc_loadavg(old_load: LoadAvgFixed, exp: LoadAvgFixed, new_load: LoadAvgFixed) -> LoadAvgFixed {
    old_load * exp + new_load * (LoadAvgFixed::ONE - exp)
}
