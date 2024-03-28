// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, sync::Arc};
use core::time::Duration;

use aster_frame::{
    arch::{jiffies_as_duration, register_interrupt_callback, TIMER_FREQ},
    sync::SpinLock,
};
use aster_time::{read_monotonic_time, NANOS_PER_SECOND};
use int_to_c_enum::TryFromInt;
use paste::paste;
use spin::Once;

use super::{system_time::START_TIME_AS_DURATION, timer::TimerManager, SystemTime};

type Nanos = u64;

#[derive(Debug, Copy, Clone, TryFromInt, PartialEq)]
#[repr(i32)]
pub enum ClockID {
    CLOCK_REALTIME = 0,
    CLOCK_MONOTONIC = 1,
    CLOCK_PROCESS_CPUTIME_ID = 2,
    CLOCK_THREAD_CPUTIME_ID = 3,
    CLOCK_MONOTONIC_RAW = 4,
    CLOCK_REALTIME_COARSE = 5,
    CLOCK_MONOTONIC_COARSE = 6,
    CLOCK_BOOTTIME = 7,
}

/// A trait that can abstract clocks which have the ability to read time,
/// and has a fixed resolution.
pub trait Clock: Send + Sync {
    /// Read the current time of this clock.
    fn read_time(&self) -> Duration;

    /// The resolution of this clock.
    /// Set to the resolution of system time interrupt by default.
    fn resolution(&self) -> Nanos
    where
        Self: Sized,
    {
        NANOS_PER_SECOND as u64 / TIMER_FREQ
    }
}

/// The Clock that reads the jiffies, and turn the counter into `Duration`.
#[derive(Default)]
pub struct JiffiesClock;

/// `RealTimeClock` represents a clock that provides the current real time.
#[derive(Default)]
pub struct RealTimeClock;

/// `MonotonicClock` represents a clock that measures time in a way that is
/// monotonically increasing since the system was booted.
#[derive(Default)]
pub struct MonotonicClock;

/// `RealTimeCoarseClock` is a coarse-grained version of a real-time clock.
#[derive(Default)]
pub struct RealTimeCoarseClock;

/// `MonotonicCoarseClock` is a coarse-grained version of the monotonic clock.
#[derive(Default)]
pub struct MonotonicCoarseClock;

/// `MonotonicRawClock` provides raw monotonic time that is not influenced by
/// NTP corrections.
///
/// Note: Currently we have not implement NTP corrections so we treat this clock
/// as the `MonotonicClock`.
#[derive(Default)]
pub struct MonotonicRawClock;

/// `BootTimeClock` measures the time elapsed since the system was booted,
/// including time when the system was suspended.
///
/// Note: Currently the system will not be suspended so we treat this clock
/// as the `MonotonicClock`.
#[derive(Default)]
pub struct BootTimeClock;

impl Clock for JiffiesClock {
    fn read_time(&self) -> Duration {
        jiffies_as_duration()
    }
}

impl Clock for RealTimeClock {
    fn read_time(&self) -> Duration {
        SystemTime::now()
            .duration_since(&SystemTime::UNIX_EPOCH)
            .unwrap()
    }
}

impl Clock for MonotonicClock {
    fn read_time(&self) -> Duration {
        read_monotonic_time()
    }
}

impl Clock for RealTimeCoarseClock {
    fn read_time(&self) -> Duration {
        read_xtime()
    }
}

impl Clock for MonotonicCoarseClock {
    fn read_time(&self) -> Duration {
        read_xtime() - *START_TIME_AS_DURATION.get().unwrap()
    }
}

impl Clock for MonotonicRawClock {
    fn read_time(&self) -> Duration {
        read_xtime() - *START_TIME_AS_DURATION.get().unwrap()
    }
}

impl Clock for BootTimeClock {
    fn read_time(&self) -> Duration {
        read_monotonic_time()
    }
}

/// Generate code for the initialization of the global timer manager in the specified position.
macro_rules! init_global_timer_manager {
    ($clock_id:ident, false, function) => {
        None
    };
    ($clock_id:ident, false, $position:ident, $clock:ident) => {};
    ($clock_id:ident, false, $position:ident) => {};
    ($clock_id:ident, true, declare) => {
        paste! {
            pub static [<$clock_id _MANAGER>]: Once<TimerManager> = Once::new();
        }
    };
    ($clock_id:ident, true, init, $clock:ident) => {
        let clock_manager = TimerManager::new($clock);
        clock_manager.set_global();
        paste! {
            [<$clock_id _MANAGER>].call_once(|| clock_manager);
        }
    };
    ($clock_id:ident, true, function) => {
        paste! {Some(&[<$clock_id _MANAGER>].get().unwrap())}
    };
}

/// Generate code for the declaration and initialization of the supported clock and global timer manager.
///
/// This macro accept a series of triples: `ClockID`, `Clock Name`, `Has Global TimerManager`(true or false).
/// It will declare a list that contains all supported `ClockID`, a instance for each clock,
/// and a `TimerManager` for corresponding clock whose `Has Global TimerManager` is set true.
///
/// Then this macro will generate the corresponding initialization function for clocks and timer managers,
/// and two functions to fetch the clock instances and timer managers from `ClockID` respectively.
macro_rules! init_supported_global_clock_and_timer_manager {
    ($($clock_id:ident, $clock_type:ty, $has_manager:ident;)*) => {
        /// A list of all supported clock IDs for time-related functions.
        const ALL_SUPPORTED_CLOCK_IDS: [ClockID; 6] = [
            $(ClockID:: $clock_id,)*
        ];

        $(
            paste! {
                pub static [<$clock_id _INSTANCE>]: Once<Arc<dyn Clock>> = Once::new();
            }
            init_global_timer_manager!($clock_id, $has_manager, declare);
        )*

        fn _init_global_clock_and_timer_manager() {
            $(
                let clock = Arc::new(<$clock_type>::default());
                paste! {
                    [<$clock_id _INSTANCE>].call_once(|| clock.clone());
                }
                init_global_timer_manager!($clock_id, $has_manager, init, clock);
            )*
        }

        fn _id_to_global_manager(clock_id: &ClockID) -> Option<&TimerManager> {
            match clock_id {
                $(
                    ClockID::$clock_id => init_global_timer_manager!($clock_id, $has_manager, function),
                )*
                _ => None,
            }
        }

        fn _id_to_global_clock(clock_id: &ClockID) -> Option<&Arc<dyn Clock>> {
            match clock_id {
                $(
                    ClockID::$clock_id => paste!{Some(&[<$clock_id _INSTANCE>].get().unwrap())},
                )*
                _ => None,
            }
        }
    };
}

init_supported_global_clock_and_timer_manager! {
    //  ClockID              |    Clock Names       |   Has Global TimerManager
    CLOCK_REALTIME,            RealTimeClock,           true;
    CLOCK_REALTIME_COARSE,     RealTimeCoarseClock,     false;
    CLOCK_MONOTONIC,           MonotonicClock,          true;
    CLOCK_MONOTONIC_COARSE,    MonotonicCoarseClock,    false;
    CLOCK_MONOTONIC_RAW,       MonotonicRawClock,       false;
    CLOCK_BOOTTIME,            BootTimeClock,           false;
}

/// Get a list that contains all supported ClockIDs.
pub fn all_supported_clock_ids() -> &'static [ClockID] {
    &ALL_SUPPORTED_CLOCK_IDS
}

/// Get the instance of the global clock depends on the input `ClockID`.
///
/// If the `ClockID` does not support or does not have a global instance,
/// this function will return `None`.
pub fn id_to_global_clock(clock_id: &ClockID) -> Option<&Arc<dyn Clock>> {
    _id_to_global_clock(clock_id)
}

/// Get the global `TimerManager` depends on the input `ClockID`.
///
/// If the `ClockID` does not have a corresponding global TimerManager,
/// this function will return `None`.
pub fn id_to_global_manager(clock_id: &ClockID) -> Option<&TimerManager> {
    _id_to_global_manager(clock_id)
}

/// Init the global clock and the global `TimerManager`s.
pub(super) fn init_global_clock_and_timer_manager() {
    _init_global_clock_and_timer_manager();
}

/// The global `TimerManager` for the `JiffiesClock`.
pub static JIFFIES_TIMER_MANAGER: Once<TimerManager> = Once::new();

/// Init the Jiffies clock and its global `TimerManager`.
pub(super) fn init_jiffies_clock_manager() {
    let jiffies_clock = JiffiesClock {};
    let jiffies_timer_manager = TimerManager::new(Arc::new(jiffies_clock));
    jiffies_timer_manager.set_global();
    JIFFIES_TIMER_MANAGER.call_once(|| jiffies_timer_manager);
}

/// A record to the RealTimeClock.
///
/// It will be updated during system timer interruptions, and will be used for
/// the coarse-grained clocks.
pub static XTIME: Once<SpinLock<Duration>> = Once::new();

fn update_xtime() {
    let real_time = id_to_global_clock(&ClockID::CLOCK_REALTIME)
        .unwrap()
        .read_time();
    if let Some(xtime) = XTIME.get() {
        *xtime.lock_irq_disabled() = real_time;
    }
}

pub(super) fn init_xtime() {
    let real_time = id_to_global_clock(&ClockID::CLOCK_REALTIME)
        .unwrap()
        .read_time();
    XTIME.call_once(|| SpinLock::new(real_time));
    register_interrupt_callback(Box::new(update_xtime));
}

fn read_xtime() -> Duration {
    *XTIME.get().unwrap().lock_irq_disabled()
}
