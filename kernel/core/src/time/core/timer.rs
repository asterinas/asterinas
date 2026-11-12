// SPDX-License-Identifier: MPL-2.0

use alloc::{
    boxed::Box,
    collections::BinaryHeap,
    sync::{Arc, Weak},
    vec::Vec,
};
use core::time::Duration;

use ostd::sync::{LocalIrqDisabled, SpinLock, SpinLockGuard};

use super::Clock;

/// A timeout, represented in one of the two ways.
#[derive(Clone, Debug)]
pub(crate) enum Timeout {
    /// The timeout is reached _after_ the `Duration` time is elapsed.
    After(Duration),
    /// The timeout is reached _when_ the clock's time is equal to `Duration`.
    When(Duration),
}

/// A timer with periodic functionality.
///
/// Setting the timer will trigger a callback function upon expiration of
/// the set time. To enable its periodic functionality, users should set
/// its `interval` field with [`TimerGuard::set_interval`]. By doing this,
/// the timer will use the interval time to configure a new timing after expiration.
pub(crate) struct Timer {
    inner: SpinLock<TimerInner>,
    timer_manager: Arc<TimerManager>,
    registered_callback: Box<dyn Fn(TimerGuard) + Send + Sync>,
}

#[derive(Default)]
struct TimerInner {
    // A `None` denotes that the timer has been cancelled.
    expired_time: Option<Duration>,
    // A `Duration::ZERO` denotes that the timer won't repeat.
    interval: Duration,
}

/// A guard that provides exclusive access to a `Timer`.
pub(crate) struct TimerGuard<'a> {
    inner: SpinLockGuard<'a, TimerInner, LocalIrqDisabled>,
    timer: &'a Arc<Timer>,
}

impl TimerGuard<'_> {
    /// Sets the interval time for this timer.
    ///
    /// The timer will be reset with the interval time upon expiration.
    pub(crate) fn set_interval(&mut self, interval: Duration) {
        self.inner.interval = interval;
    }

    /// Sets the timer with a timeout.
    ///
    /// The registered callback function of this timer will be invoked
    /// when reaching timeout. If the timer has a valid interval, this timer
    /// will be set again with the interval when reaching timeout.
    pub(crate) fn set_timeout(&mut self, timeout: Timeout) {
        let expired_time = match timeout {
            Timeout::After(timeout) => {
                let now = self.timer.timer_manager.clock.read_time();
                now + timeout
            }
            Timeout::When(timeout) => timeout,
        };

        self.inner.expired_time = Some(expired_time);

        let timer_callback = TimerCallback {
            expired_time,
            timer: Arc::downgrade(self.timer),
        };
        self.timer.timer_manager.insert(timer_callback);
    }

    /// Cancels the timer.
    ///
    /// Once cancelled, the timer will not be triggered again.
    pub(crate) fn cancel(&mut self) {
        self.inner.expired_time = None;
    }

    /// Returns the current expired time of this timer.
    ///
    /// If the timer has not been set, this method
    /// will return a [`Duration::ZERO`].
    pub(crate) fn expired_time(&self) -> Duration {
        self.inner.expired_time.unwrap_or(Duration::ZERO)
    }

    /// Returns the remain time to expiration of this timer.
    ///
    /// If the timer has not been set, this method
    /// will return a [`Duration::ZERO`].
    pub(crate) fn remain(&self) -> Duration {
        let now = self.timer.timer_manager.clock.read_time();
        let expired_time = self.expired_time();
        expired_time.saturating_sub(now)
    }

    /// Returns the interval time of the current timer.
    pub(crate) fn interval(&self) -> Duration {
        self.inner.interval
    }
}

impl Timer {
    /// Creates a `Timer` instance from a [`TimerManager`].
    /// This timer will be managed by the `TimerManager`.
    ///
    /// Note that if the callback instructions involves sleep, users should put these instructions
    /// into something like `WorkQueue` to avoid sleeping during system timer interruptions.
    fn new<F>(registered_callback: F, timer_manager: Arc<TimerManager>) -> Arc<Self>
    where
        F: Fn(TimerGuard) + Send + Sync + 'static,
    {
        Arc::new(Self {
            inner: SpinLock::new(TimerInner::default()),
            timer_manager,
            registered_callback: Box::new(registered_callback),
        })
    }

    /// Locks the timer and returns a [`TimerGuard`] for exclusive access.
    pub(crate) fn lock(self: &Arc<Self>) -> TimerGuard<'_> {
        TimerGuard {
            inner: self.inner.disable_irq().lock(),
            timer: self,
        }
    }

    /// Returns a reference to the [`TimerManager`] which manages
    /// the current timer.
    pub(crate) fn timer_manager(&self) -> &Arc<TimerManager> {
        &self.timer_manager
    }
}

/// `TimerManager` is used to create timers and manage their expiries. It holds a clock and can
/// create [`Timer`]s based on this clock.
///
/// These created `Timer`s will hold an `Arc` pointer to this manager, hence this manager
/// will be actually dropped after all the created timers have been dropped.
pub(crate) struct TimerManager {
    clock: Arc<dyn Clock>,
    timer_callbacks: SpinLock<BinaryHeap<TimerCallback>>,
}

impl TimerManager {
    /// Creates a `TimerManager` instance from a clock.
    pub(crate) fn new(clock: Arc<dyn Clock>) -> Arc<Self> {
        Arc::new(Self {
            clock,
            timer_callbacks: SpinLock::new(BinaryHeap::new()),
        })
    }

    /// Returns the clock associated with this timer manager.
    pub(crate) fn clock(&self) -> &Arc<dyn Clock> {
        &self.clock
    }

    /// Returns whether a given `timeout` is expired.
    pub(crate) fn is_expired_timeout(&self, timeout: &Timeout) -> bool {
        match timeout {
            Timeout::After(duration) => *duration == Duration::ZERO,
            Timeout::When(duration) => {
                let now = self.clock.read_time();
                now >= *duration
            }
        }
    }

    fn insert(&self, timer_callback: TimerCallback) {
        self.timer_callbacks
            .disable_irq()
            .lock()
            .push(timer_callback);
    }

    /// Checks and processes the managed timers.
    ///
    /// If any of the timers have timed out, call the corresponding callback functions.
    pub(crate) fn process_expired_timers(&self) {
        let callbacks = {
            let mut timeout_list = self.timer_callbacks.disable_irq().lock();
            if timeout_list.is_empty() {
                return;
            }

            let mut callbacks = Vec::new();
            let current_time = self.clock.read_time();
            while let Some(t) = timeout_list.peek() {
                if t.expired_time <= current_time {
                    callbacks.push(timeout_list.pop().unwrap());
                } else {
                    break;
                }
            }
            callbacks
        };

        for callback in callbacks {
            callback.call();
        }
    }

    /// Creates an [`Timer`], which will be managed by this `TimerManager`.
    pub(crate) fn create_timer<F>(self: &Arc<Self>, function: F) -> Arc<Timer>
    where
        F: Fn(TimerGuard) + Send + Sync + 'static,
    {
        Timer::new(function, self.clone())
    }
}

/// A `TimerCallback` can be used to execute a timer callback function.
struct TimerCallback {
    expired_time: Duration,
    timer: Weak<Timer>,
}

impl TimerCallback {
    fn call(&self) {
        let Some(timer) = self.timer.upgrade() else {
            return;
        };

        let mut timer_guard = timer.lock();

        if timer_guard.inner.expired_time != Some(self.expired_time) {
            // The callback is cancelled.
            return;
        }

        let interval = timer_guard.interval();
        if interval != Duration::ZERO {
            timer_guard.set_timeout(Timeout::After(interval));
        } else {
            timer_guard.cancel();
        }

        // Pass the `timer_guard` guard to the callback, allowing it to prevent race conditions.
        // The callback may choose to use the guard or drop it if not needed.
        (timer.registered_callback)(timer_guard);
    }
}

impl PartialEq for TimerCallback {
    fn eq(&self, other: &Self) -> bool {
        self.expired_time == other.expired_time
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
        // We want `TimerCallback`s to be processed in ascending order of `expired_time`,
        // and the in-order management of `TimerCallback`s currently relies on a maximum heap,
        // so we need the reverse instruction here.
        self.expired_time.cmp(&other.expired_time).reverse()
    }
}
