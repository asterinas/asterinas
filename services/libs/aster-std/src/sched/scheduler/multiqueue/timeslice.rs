use aster_frame::{config::TIMER_FREQ, task::Priority};
use core::time::Duration;

type Tick = u64;

/// The ticks a task gets during one round of execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TimeSlice(Tick);

pub const DEFAULT_TIME_SLICE: Tick = 100 * TIMER_FREQ / 1000; // 100 ms
const MIN_TIME_SLICE_TMP: Tick = 5 * TIMER_FREQ / 1000; // 5 ms
pub const MIN_TIME_SLICE: Tick = if MIN_TIME_SLICE_TMP > 1 {
    MIN_TIME_SLICE_TMP
} else {
    1
}; // the larger one in 1 tick and 5ms
pub const MAX_TIME_SLICE: Tick = 800 * TIMER_FREQ / 1000; // 800 ms

impl Default for TimeSlice {
    fn default() -> Self {
        Self(DEFAULT_TIME_SLICE)
    }
}

impl TimeSlice {
    pub const fn as_ticks(&self) -> Tick {
        self.0
    }

    pub const fn as_ms(&self) -> u64 {
        self.0 * 1000 / TIMER_FREQ
    }

    pub fn from_ms(ms: Tick) -> Self {
        Self(ms * TIMER_FREQ / 1000)
    }

    pub fn from_duration(duration: &Duration) -> Self {
        Self::from_ms(duration.as_millis() as Tick)
    }

    pub fn as_duration(&self) -> Duration {
        Duration::from_millis(self.as_ms())
    }

    pub const MAX: Self = Self(MAX_TIME_SLICE);
    pub const MIN: Self = Self(MIN_TIME_SLICE);

    pub const fn from(static_prio: Priority) -> Self {
        use static_assertions::const_assert_eq;
        const_assert_eq!(Priority::normal().get(), 100);
        let multiplier = if static_prio.get() <= 120 /* Nice: 0 */ {
            2
        } else {
            0
        };
        let x = DEFAULT_TIME_SLICE << multiplier;

        const_assert_eq!(Priority::lowest().get(), 139);
        const MAX_PRIO: u64 = Priority::lowest().get() as u64 + 1;
        const MAX_USER_PRIO: u64 = MAX_PRIO - Priority::normal().get() as u64;

        const fn limit(x: Tick) -> Tick {
            let ans = if x < MIN_TIME_SLICE {
                MIN_TIME_SLICE
            } else {
                x
            };
            if ans > MAX_TIME_SLICE {
                MAX_TIME_SLICE
            } else {
                ans
            }
        }
        Self(limit(
            x * (MAX_PRIO - static_prio.get() as u64) / (MAX_USER_PRIO >> 1),
        ))
    }
}

impl From<Tick> for TimeSlice {
    fn from(ticks: Tick) -> Self {
        Self(ticks)
    }
}

#[if_cfg_ktest]
mod tests {
    use super::*;

    #[ktest]
    fn default_timeslice() {
        let ts = TimeSlice::default();
        assert_eq!(ts.as_ticks(), DEFAULT_TIME_SLICE);
    }
}
