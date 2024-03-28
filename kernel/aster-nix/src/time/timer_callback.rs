// SPDX-License-Identifier: MPL-2.0

use core::{sync::atomic::AtomicBool, time::Duration};

use atomic::Ordering;

use crate::prelude::*;

/// A TimerCallback can be used to execute a timer callback function.
pub struct TimerCallback {
    expire_time: Duration,
    callback: Box<dyn Fn() + Send + Sync>,
    is_cancelled: AtomicBool,
}

impl TimerCallback {
    /// Create an instance of `TimerCallback`.
    pub fn new(timeout: Duration, callback: Box<dyn Fn() + Send + Sync>) -> Self {
        Self {
            expire_time: timeout,
            callback,
            is_cancelled: AtomicBool::new(false),
        }
    }

    /// Return the expire time of the `TimerCallback`.
    pub fn expire_time(&self) -> Duration {
        self.expire_time
    }

    /// Whether the set timeout is reached
    pub fn is_expired(&self, now: Duration) -> bool {
        self.expire_time <= now
    }

    /// Cancel a timer callback. If the callback function has not been called,
    /// it will never be called again.
    pub fn cancel(&self) {
        self.is_cancelled.store(true, Ordering::Release);
    }

    // Whether the timer callback is cancelled.
    pub(super) fn is_cancelled(&self) -> bool {
        self.is_cancelled.load(Ordering::Acquire)
    }

    /// Execute the callback function.
    pub(super) fn callback(&self) {
        (self.callback)()
    }
}

impl PartialEq for TimerCallback {
    fn eq(&self, other: &Self) -> bool {
        self.expire_time == other.expire_time
    }
}

impl Eq for TimerCallback {}

impl PartialOrd for TimerCallback {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for TimerCallback {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.expire_time.cmp(&other.expire_time).reverse()
    }
}
