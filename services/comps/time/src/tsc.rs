// SPDX-License-Identifier: MPL-2.0

//! This module provide a instance of `ClockSource` based on TSC.
//!
//! Use `init` to initialize this module.
use alloc::sync::Arc;
use aster_frame::arch::{read_tsc, x86::tsc_freq};
use aster_frame::timer::Timer;
use core::time::Duration;
use spin::Once;

use crate::clocksource::{ClockSource, Instant};
use crate::{START_TIME, VDSO_DATA_UPDATE};

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
pub(super) fn read_instant() -> Instant {
    let clock = CLOCK.get().unwrap();
    clock.read_instant()
}

fn update_clocksource(timer: Arc<Timer>) {
    let clock = CLOCK.get().unwrap();
    clock.update();

    // Update vdso data.
    if VDSO_DATA_UPDATE.is_completed() {
        VDSO_DATA_UPDATE.get().unwrap()(clock.last_instant(), clock.last_cycles());
    }
    // Setting the timer as `clock.max_delay_secs() - 1` is to avoid
    // the actual delay time is greater than the maximum delay seconds due to the latency of execution.
    timer.set(Duration::from_secs(clock.max_delay_secs() - 1));
}

fn init_timer() {
    let timer = Timer::new(update_clocksource).unwrap();
    // The initial timer should be set as `clock.max_delay_secs() >> 1` or something much smaller than `max_delay_secs`.
    // This is because the initialization of this timer occurs during system startup,
    // and the system will also undergo other initialization processes, during which time interrupts are disabled.
    // This results in the actual trigger time of the timer being delayed by about 5 seconds compared to the set time.
    // If without KVM, the delayed time will be larger.
    // TODO: This is a temporary solution, and should be modified in the future.
    timer.set(Duration::from_secs(
        CLOCK.get().unwrap().max_delay_secs() >> 1,
    ));
}
