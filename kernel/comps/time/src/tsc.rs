// SPDX-License-Identifier: MPL-2.0

//! This module provide a instance of `ClockSource` based on TSC.
//!
//! Use `init` to initialize this module.
use alloc::sync::Arc;
use core::sync::atomic::{AtomicU64, Ordering};

use ostd::{
    arch::{read_tsc, timer::TIMER_FREQ, tsc_freq},
    timer,
};
use spin::Once;

use crate::{
    clocksource::{ClockSource, Instant},
    START_TIME, VDSO_DATA_HIGH_RES_UPDATE_FN,
};

/// A instance of TSC clocksource.
pub static CLOCK: Once<Arc<ClockSource>> = Once::new();

const MAX_DELAY_SECS: u64 = 100;

/// Init tsc clocksource module.
pub(super) fn init() {
    init_clock();
    calibrate();
    init_timer();
}

fn init_clock() {
    CLOCK.call_once(|| {
        Arc::new(ClockSource::new(
            tsc_freq(),
            MAX_DELAY_SECS,
            Arc::new(read_tsc),
        ))
    });
}

/// Calibrate the TSC and system time based on the RTC time.
fn calibrate() {
    let clock = CLOCK.get().unwrap();
    let cycles = clock.read_cycles();
    clock.calibrate(cycles);
    START_TIME.call_once(crate::read);
}

/// Read an `Instant` of tsc clocksource.
pub fn read_instant() -> Instant {
    let clock = CLOCK.get().unwrap();
    clock.read_instant()
}

fn update_clocksource() {
    let clock = CLOCK.get().unwrap();
    clock.update();

    // Update vdso data.
    if let Some(update_fn) = VDSO_DATA_HIGH_RES_UPDATE_FN.get() {
        let (last_instant, last_cycles) = clock.last_record();
        update_fn(last_instant, last_cycles);
    }
}

static TSC_UPDATE_COUNTER: AtomicU64 = AtomicU64::new(1);

fn init_timer() {
    // The `max_delay_secs` should be set as `clock.max_delay_secs() >> 1` or something much smaller than `max_delay_secs`.
    // This is because the initialization of this timer occurs during system startup,
    // and the system will also undergo other initialization processes, during which time interrupts are disabled.
    // This results in the actual trigger time of the timer being delayed by about 5 seconds compared to the set time.
    // If without KVM, the delayed time will be larger.
    // TODO: This is a temporary solution, and should be modified in the future.
    let max_delay_secs = CLOCK.get().unwrap().max_delay_secs() >> 1;
    let delay_counts = TIMER_FREQ * max_delay_secs;

    let update = move || {
        let counter = TSC_UPDATE_COUNTER.fetch_add(1, Ordering::Relaxed);

        if counter % delay_counts == 0 {
            update_clocksource();
        }
    };

    // TODO: re-organize the code structure and use the `Timer` to achieve the updating.
    timer::register_callback(update);
}
