use core::sync::atomic::{AtomicBool, Ordering};
use core::time::Duration;

use crate::prelude::*;
use crate::thread::work_queue::work_item::WorkItem;
use crate::thread::work_queue::{submit_work_item, WorkPriority};
use crate::thread::Tid;
use crate::time::SystemTime;

/// An timer to run delayed task in process context.
///
/// Unlike `Timer` in `jinux-frame`, the task of this `Timer` will be executed in process
/// context, instead of interrupt context. This brings two differences:
///
/// 1. The `task` of this `Timer` can sleep, while the task of `Timer` in `jinux-frame` cannot.
/// 2. The `task` of this `Timer` may be delayed by an arbitrary amount of time due to scheduler strategy.
///
/// Note that the `task` may not be executed in the process context who creates the timer, so
/// the `current` and `current_thread` macro should not be used in `task`.
///
/// TODO: implement the interval timer
///
/// # Example
/// ```rust norun
/// let timer = Timer::new(|current_tid| {
///     let current_thread = thread_table::get_thread(current_tid);
///     let posix_thread = current_thread.as_posix_thread().unwrap();
///     let process = posix_thread.process();
///     println!("Task executed for PID: {}", process.pid());
/// }, Duration::from_secs(0));
/// ```
pub struct Timer {
    timer: Arc<jinux_frame::timer::Timer>,
    expired_time: SystemTime,
    is_cancelled: AtomicBool,
}

impl Timer {
    /// Creates a new `Timer`. The parameter of `task` fn represents
    /// the ID of the thread that creates the timer.
    pub fn new(
        task: impl Fn(Tid) + Send + Sync + Copy + 'static,
        timeout: Duration,
    ) -> Result<Self> {
        let timer = {
            let current_tid = current_thread!().tid();
            jinux_frame::timer::Timer::new(move |timer| {
                let work_func = Box::new(move || task(current_tid));
                let work_item = { Arc::new(WorkItem::new(work_func)) };
                // FIXME: set a higher priority like `WorkPriority::Alarm`.
                submit_work_item(work_item, WorkPriority::High);
            })?
        };
        timer.clone().set(timeout);

        let expired_time = {
            let now = SystemTime::now();
            now.checked_add(timeout)
                .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid timeout"))?
        };

        Ok(Self {
            timer,
            expired_time,
            is_cancelled: AtomicBool::new(false),
        })
    }

    /// Returns the remaining time until the `task` to be executed. If the timer is
    /// cancelled or expired, this method will return zero.
    pub fn remain(&self) -> Duration {
        if self.is_cancelled() {
            return ZERO_DURATION;
        }

        let now = SystemTime::now();
        match self.expired_time.duration_since(&now) {
            Ok(duration) => duration,
            Err(_) => ZERO_DURATION,
        }
    }

    /// Returns whether the timer is expired.
    pub fn is_expired(&self) -> bool {
        self.remain() == ZERO_DURATION
    }

    /// Cancels the timer if the timer has not expired.
    pub fn cancel(&self) {
        self.timer.clear();
        self.is_cancelled.store(true, Ordering::Release);
    }

    /// Returns whether the timer is cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.is_cancelled.load(Ordering::Acquire)
    }
}

impl PartialEq for Timer {
    fn eq(&self, other: &Self) -> bool {
        self.expired_time == other.expired_time
    }
}

impl Eq for Timer {}

impl PartialOrd for Timer {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Timer {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        // Note: this order is reversed.
        // This is because the `timers` in `posix_thread` is a max heap, while we want
        // to get the timer with minimal expired time.
        other.expired_time.cmp(&self.expired_time)
    }
}

const ZERO_DURATION: Duration = Duration::new(0, 0);
