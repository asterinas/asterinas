// SPDX-License-Identifier: MPL-2.0

use alloc::{
    boxed::Box,
    collections::BinaryHeap,
    sync::{Arc, Weak},
    vec::Vec,
};
use core::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use ostd::sync::SpinLock;

use super::Clock;

/// A timeout, represented in one of the two ways.
#[derive(Debug, Clone)]
pub enum Timeout {
    /// The timeout is reached _after_ the `Duration` time is elapsed.
    After(Duration),
    /// The timeout is reached _when_ the clock's time is equal to `Duration`.
    When(Duration),
}

/// A timer with periodic functionality.
///
/// Setting the timer will trigger a callback function upon expiration of
/// the set time. To enable its periodic functionality, users should set
/// its `interval` field with [`Timer::set_interval`]. By doing this,
/// the timer will use the interval time to configure a new timing after expiration.
pub struct Timer {
    interval: SpinLock<Duration>,
    timer_manager: Arc<TimerManager>,
    registered_callback: Box<dyn Fn() + Send + Sync>,
    timer_callback: SpinLock<Weak<TimerCallback>>,
}

impl Timer {
    /// Create a `Timer` instance from a [`TimerManager`].
    /// This timer will be managed by the `TimerManager`.
    ///
    /// Note that if the callback instructions involves sleep, users should put these instructions
    /// into something like `WorkQueue` to avoid sleeping during system timer interruptions.
    fn new<F>(registered_callback: F, timer_manager: Arc<TimerManager>) -> Arc<Self>
    where
        F: Fn() + Send + Sync + 'static,
    {
        Arc::new(Self {
            interval: SpinLock::new(Duration::ZERO),
            timer_manager,
            registered_callback: Box::new(registered_callback),
            timer_callback: SpinLock::new(Weak::default()),
        })
    }

    /// Set the interval time for this timer.
    /// The timer will be reset with the interval time upon expiration.
    pub fn set_interval(&self, interval: Duration) {
        *self.interval.disable_irq().lock() = interval;
    }

    /// Cancel the current timer's set timeout callback.
    pub fn cancel(&self) {
        let timer_callback = self.timer_callback.disable_irq().lock();
        if let Some(timer_callback) = timer_callback.upgrade() {
            timer_callback.cancel();
        }
    }

    /// Set the timer with a timeout.
    ///
    /// The registered callback function of this timer will be invoked
    /// when reaching timeout. If the timer has a valid interval, this timer
    /// will be set again with the interval when reaching timeout.
    pub fn set_timeout(self: &Arc<Self>, timeout: Timeout) {
        let expired_time = match timeout {
            Timeout::After(timeout) => {
                let now = self.timer_manager.clock.read_time();
                now + timeout
            }
            Timeout::When(timeout) => timeout,
        };

        let timer_weak = Arc::downgrade(self);
        let new_timer_callback = Arc::new(TimerCallback::new(
            expired_time,
            Box::new(move || interval_timer_callback(&timer_weak)),
        ));

        let mut timer_callback = self.timer_callback.disable_irq().lock();
        if let Some(timer_callback) = timer_callback.upgrade() {
            timer_callback.cancel();
        }
        *timer_callback = Arc::downgrade(&new_timer_callback);
        self.timer_manager.insert(new_timer_callback);
    }

    /// Return the current expired time of this timer.
    pub fn expired_time(&self) -> Duration {
        let timer_callback = self.timer_callback.disable_irq().lock().upgrade();
        timer_callback.map_or(Duration::ZERO, |timer_callback| timer_callback.expired_time)
    }

    /// Return the remain time to expiration of this timer.
    ///
    /// If the timer has not been set, this method
    /// will return `Duration::ZERO`.
    pub fn remain(&self) -> Duration {
        let now = self.timer_manager.clock.read_time();
        let expired_time = self.expired_time();
        if expired_time > now {
            expired_time - now
        } else {
            Duration::ZERO
        }
    }

    /// Return a reference to the [`TimerManager`] which manages
    /// the current timer.
    pub fn timer_manager(&self) -> &Arc<TimerManager> {
        &self.timer_manager
    }

    /// Returns the interval time of the current timer.
    pub fn interval(&self) -> Duration {
        *self.interval.disable_irq().lock()
    }
}

fn interval_timer_callback(timer: &Weak<Timer>) {
    let Some(timer) = timer.upgrade() else {
        return;
    };

    (timer.registered_callback)();
    let interval = timer.interval.disable_irq().lock();
    if *interval != Duration::ZERO {
        timer.set_timeout(Timeout::After(*interval));
    }
}

/// `TimerManager` is used to create timers and manage their expiries. It holds a clock and can
/// create [`Timer`]s based on this clock.
///
/// These created `Timer`s will hold an `Arc` pointer to this manager, hence this manager
/// will be actually dropped after all the created timers have been dropped.
pub struct TimerManager {
    clock: Arc<dyn Clock>,
    timer_callbacks: SpinLock<BinaryHeap<Arc<TimerCallback>>>,
}

impl TimerManager {
    /// Create a `TimerManager` instance from a clock.
    pub fn new(clock: Arc<dyn Clock>) -> Arc<Self> {
        Arc::new(Self {
            clock,
            timer_callbacks: SpinLock::new(BinaryHeap::new()),
        })
    }

    /// Returns whether a given `timeout` is expired.
    pub fn is_expired_timeout(&self, timeout: &Timeout) -> bool {
        match timeout {
            Timeout::After(duration) => *duration == Duration::ZERO,
            Timeout::When(duration) => {
                let now = self.clock.read_time();
                now >= *duration
            }
        }
    }

    fn insert(&self, timer_callback: Arc<TimerCallback>) {
        self.timer_callbacks
            .disable_irq()
            .lock()
            .push(timer_callback);
    }

    /// Check the managed timers, and if any have timed out,
    /// call the corresponding callback functions.
    pub fn process_expired_timers(&self) {
        let callbacks = {
            let mut timeout_list = self.timer_callbacks.disable_irq().lock();
            if timeout_list.len() == 0 {
                return;
            }

            let mut callbacks = Vec::new();
            let current_time = self.clock.read_time();
            while let Some(t) = timeout_list.peek() {
                if t.is_cancelled() {
                    // Just ignore the cancelled callback
                    timeout_list.pop();
                } else if t.expired_time <= current_time {
                    callbacks.push(timeout_list.pop().unwrap());
                } else {
                    break;
                }
            }
            callbacks
        };

        for callback in callbacks {
            (callback.callback)();
        }
    }

    /// Create an [`Timer`], which will be managed by this `TimerManager`.
    pub fn create_timer<F>(self: &Arc<Self>, function: F) -> Arc<Timer>
    where
        F: Fn() + Send + Sync + 'static,
    {
        Timer::new(function, self.clone())
    }
}

/// A `TimerCallback` can be used to execute a timer callback function.
struct TimerCallback {
    expired_time: Duration,
    callback: Box<dyn Fn() + Send + Sync>,
    is_cancelled: AtomicBool,
}

impl TimerCallback {
    /// Create an instance of `TimerCallback`.
    fn new(timeout: Duration, callback: Box<dyn Fn() + Send + Sync>) -> Self {
        Self {
            expired_time: timeout,
            callback,
            is_cancelled: AtomicBool::new(false),
        }
    }

    /// Cancel a `TimerCallback`. If the callback function has not been called,
    /// it will never be called again.
    fn cancel(&self) {
        self.is_cancelled.store(true, Ordering::Release);
    }

    // Return whether the `TimerCallback` is cancelled.
    fn is_cancelled(&self) -> bool {
        self.is_cancelled.load(Ordering::Acquire)
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
