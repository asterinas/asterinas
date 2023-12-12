// SPDX-License-Identifier: MPL-2.0

use core::time::Duration;

use crate::{
    prelude::*,
    thread::work_queue::{submit_work_item, work_item::WorkItem, WorkPriority},
    time::SystemTime,
};

/// A timer that counts down in real (wall clock) time to run delayed callbacks in process context.
///
/// Unlike the `Timer` in `aster-frame`, the callbacks of this `RealTimer` will be executed in process
/// context instead of interrupt context. This leads to two differences:
///
/// 1. The callbacks of this `RealTimer` can sleep, whereas the callbacks of `Timer` in `aster-frame` cannot.
/// 2. The callbacks of this `RealTimer` may be delayed by an arbitrary amount of time due to scheduler strategy.
///
/// Note that the callbacks may not be executed in the process context of the timer's creator, so macros such
/// as `current` and `current_thread` should **NOT** be used in the callback.
///
/// # Example
/// ```rust
/// let current_tid = current_thread!().tid();
/// let timer = RealTimer::new(move || {
///     let current_thread = thread_table::get_thread(current_tid);
///     let posix_thread = current_thread.as_posix_thread().unwrap();
///     let process = posix_thread.process();
///     println!("Task executed for PID: {}", process.pid());
/// }, Duration::from_secs(1));
pub struct RealTimer {
    timer: Arc<aster_frame::timer::Timer>,
    expired_time: Option<SystemTime>,
}

impl RealTimer {
    /// Creates a new `RealTimer`. The `callback` parameter will be called once the timeout is reached.
    pub fn new(callback: impl Fn() + Send + Sync + Copy + 'static) -> Result<Self> {
        let timer = {
            aster_frame::timer::Timer::new(move |timer| {
                let work_func = Box::new(callback);
                let work_item = { Arc::new(WorkItem::new(work_func)) };
                // FIXME: set a higher priority like `WorkPriority::Alarm`.
                submit_work_item(work_item, WorkPriority::High);
            })?
        };

        Ok(Self {
            timer,
            expired_time: None,
        })
    }

    /// Sets a new timeout value. If the old timeout is already set, the timeout will be refreshed.
    pub fn set(&mut self, timeout: Duration) -> Result<()> {
        assert_ne!(timeout, ZERO_DURATION);

        let new_expired_time = {
            let now = SystemTime::now();
            now.checked_add(timeout)
                .ok_or_else(|| Error::with_message(Errno::EINVAL, "Invalid duration"))?
        };

        self.expired_time = Some(new_expired_time);

        self.timer.set(timeout);

        Ok(())
    }

    /// Returns the remaining time until the task is executed. If the `timer` is expired or cleared,
    /// this method will return zero.
    pub fn remain(&self) -> Duration {
        let Some(expired_time) = &self.expired_time else {
            return ZERO_DURATION;
        };

        let now = SystemTime::now();
        match expired_time.duration_since(&now) {
            Ok(duration) => duration,
            Err(_) => ZERO_DURATION,
        }
    }

    /// Returns whether the timer has expired.
    pub fn is_expired(&self) -> bool {
        self.remain() == ZERO_DURATION
    }

    /// Clears the timer.
    pub fn clear(&mut self) {
        self.timer.clear();
        self.expired_time = None;
    }
}

impl Drop for RealTimer {
    fn drop(&mut self) {
        self.timer.clear();
    }
}

impl PartialEq for RealTimer {
    fn eq(&self, other: &Self) -> bool {
        self.expired_time == other.expired_time
    }
}

impl Eq for RealTimer {}

const ZERO_DURATION: Duration = Duration::new(0, 0);
