// SPDX-License-Identifier: MPL-2.0

//! This module provides abstractions for hardware-assisted timing mechanisms, encapsulated by the `ClockSource` struct.
//! A `ClockSource` can be constructed from any counter with a stable frequency, enabling precise time measurements to be taken
//! by retrieving instances of `Instant`.
//!
//! The `ClockSource` module is a fundamental building block for timing in systems that require high precision and accuracy.
//! It can be integrated into larger systems to provide timing capabilities, or used standalone for time tracking and elapsed time measurements.

use alloc::sync::Arc;
use aster_frame::sync::SpinLock;
use aster_util::coeff::Coeff;
use core::{cmp::max, ops::Add, time::Duration};

use crate::NANOS_PER_SECOND;

/// `ClockSource` is an abstraction for hardware-assisted timing mechanisms.
/// A `ClockSource` can be created based on any counter that operates at a stable frequency.
/// Users are able to measure time by retrieving `Instant` from this source.
///
/// # Implementation
/// The `ClockSource` relies on obtaining the frequency of the counter and the method for reading the cycles in order to measure time.
/// The **cycles** here refer the counts of the base time counter.
/// Additionally, the `ClockSource` also holds a last recorded instant, which acts as a reference point for subsequent time retrieval.
/// To prevent numerical overflow during the calculation of `Instant`, this last recorded instant **must be periodically refreshed**.
/// The maximum interval for these updates must be determined at the time of the `ClockSource` initialization.
///
/// # Examples
/// Suppose we have a counter called `counter` which have the frequency `counter.freq`, and the method to read its cycles called `read_counter()`.
/// We can create a corresponding `ClockSource` and use it as follows:
///
/// ```rust
/// // here we set the max_delay_secs = 10
/// let max_delay_secs = 10;
/// // create a clocksource named counter_clock
/// let counter_clock = ClockSource::new(counter.freq, max_delay_secs, Arc::new(read_counter));
/// // read an instant.
/// let instant = counter_clock.read_instant();
/// ```
///
/// If using this `ClockSource`, you must ensure its internal instant will be updated
/// at least once within a time interval of not more than `max_delay_secs.
pub struct ClockSource {
    read_cycles: Arc<dyn Fn() -> u64 + Sync + Send>,
    base: ClockSourceBase,
    coeff: Coeff,
    last_instant: SpinLock<Instant>,
    last_cycles: SpinLock<u64>,
}

impl ClockSource {
    /// Create a new `ClockSource` instance.
    /// Require basic information of based time counter, including the function for reading cycles, the frequency
    /// and the maximum delay seconds to update this `ClockSource`.
    /// The `ClockSource` also calculates a reliable `Coeff` based on the counter's frequency and the maximum delay seconds.
    /// This `Coeff` is used to convert the number of cycles into the duration of time that has passed for those cycles.
    pub fn new(
        freq: u64,
        max_delay_secs: u64,
        read_cycles: Arc<dyn Fn() -> u64 + Sync + Send>,
    ) -> Self {
        let base = ClockSourceBase::new(freq, max_delay_secs);
        // Too big `max_delay_secs` will lead to a low resolution Coeff.
        debug_assert!(max_delay_secs < 600);
        let coeff = Coeff::new(NANOS_PER_SECOND as u64, freq, max_delay_secs * freq);
        Self {
            read_cycles,
            base,
            coeff,
            last_instant: SpinLock::new(Instant::zero()),
            last_cycles: SpinLock::new(0),
        }
    }

    fn cycles_to_nanos(&self, cycles: u64) -> u64 {
        self.coeff * cycles
    }

    /// Use the instant cycles to calculate the instant.
    /// It first calculates the difference between the instant cycles and the last recorded cycles stored in the clocksource.
    /// Then `ClockSource` will convert the passed cycles into passed time and calculate the current instant.
    fn calculate_instant(&self, instant_cycles: u64) -> Instant {
        let delta_nanos = {
            let delta_cycles = instant_cycles - self.last_cycles();
            self.cycles_to_nanos(delta_cycles)
        };
        let duration = Duration::from_nanos(delta_nanos);
        self.last_instant() + duration
    }

    /// Use an input instant to update the internal instant in the `ClockSource`.
    fn update_last_instant(&self, instant: &Instant) {
        *self.last_instant.lock() = *instant;
    }

    /// Use an input cycles to update the internal instant in the `ClockSource`.
    fn update_last_cycles(&self, cycles: u64) {
        *self.last_cycles.lock() = cycles;
    }

    /// read current cycles of the `ClockSource`.
    pub fn read_cycles(&self) -> u64 {
        (self.read_cycles)()
    }

    /// Return the last instant recorded in the `ClockSource`.
    pub fn last_instant(&self) -> Instant {
        return *self.last_instant.lock();
    }

    /// Return the last cycles recorded in the `ClockSource`.
    pub fn last_cycles(&self) -> u64 {
        return *self.last_cycles.lock();
    }

    /// Return the maximum delay seconds for updating of the `ClockSource`.
    pub fn max_delay_secs(&self) -> u64 {
        self.base.max_delay_secs()
    }

    /// Return the reference to the generated cycles coeff of the `ClockSource`.
    pub fn coeff(&self) -> &Coeff {
        &self.coeff
    }

    /// Return the frequency of the counter used in the `ClockSource`.
    pub fn freq(&self) -> u64 {
        self.base.freq()
    }

    /// Calibrate the recorded `Instant` to zero, and record the instant cycles.
    pub(crate) fn calibrate(&self, instant_cycles: u64) {
        self.update_last_cycles(instant_cycles);
        self.update_last_instant(&Instant::zero());
    }

    /// Get the instant to update the internal instant in the `ClockSource`.
    pub(crate) fn update(&self) {
        let instant_cycles = self.read_cycles();
        let instant = self.calculate_instant(instant_cycles);
        self.update_last_cycles(instant_cycles);
        self.update_last_instant(&instant);
    }

    /// Read the instant corresponding to the current time.
    /// When trying to read an instant from the clocksource, it will use the reading method to read instant cycles.
    /// Then leverage it to calculate the instant.
    pub(crate) fn read_instant(&self) -> Instant {
        let instant_cycles = self.read_cycles();
        self.calculate_instant(instant_cycles)
    }
}

/// `Instant` captures a specific moment, storing the duration of time
/// elapsed since a reference point (typically the system boot time).
/// The `Instant` is expressed in seconds and the fractional part is expressed in nanoseconds.
#[derive(Debug, Default, Copy, Clone)]
pub struct Instant {
    secs: u64,
    nanos: u32,
}

impl Instant {
    pub const fn zero() -> Self {
        Self { secs: 0, nanos: 0 }
    }

    pub fn new(secs: u64, nanos: u32) -> Self {
        Self { secs, nanos }
    }

    /// Return the seconds recorded in the Instant.
    pub fn secs(&self) -> u64 {
        self.secs
    }

    /// Return the nanoseconds recorded in the Instant.
    pub fn nanos(&self) -> u32 {
        self.nanos
    }
}

impl Add<Duration> for Instant {
    type Output = Instant;

    fn add(self, other: Duration) -> Self::Output {
        let mut secs = self.secs + other.as_secs();
        let mut nanos = self.nanos + other.subsec_nanos();
        if nanos >= NANOS_PER_SECOND {
            secs += 1;
            nanos -= NANOS_PER_SECOND;
        }
        Instant::new(secs, nanos)
    }
}

/// The basic properties of `ClockSource`.
#[derive(Debug, Copy, Clone)]
struct ClockSourceBase {
    freq: u64,
    max_delay_secs: u64,
}

impl ClockSourceBase {
    fn new(freq: u64, max_delay_secs: u64) -> Self {
        let max_delay_secs = max(2, max_delay_secs);
        ClockSourceBase {
            freq,
            max_delay_secs,
        }
    }

    fn max_delay_secs(&self) -> u64 {
        self.max_delay_secs
    }

    fn freq(&self) -> u64 {
        self.freq
    }
}
