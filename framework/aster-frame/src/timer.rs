// SPDX-License-Identifier: MPL-2.0

//! Timer.

#[cfg(target_arch = "x86_64")]
use crate::arch::x86::timer::{add_timeout_list, TimerCallback, TICK};
use crate::sync::SpinLock;
use crate::{config::TIMER_FREQ, prelude::*};
use core::{sync::atomic::Ordering, time::Duration};

#[cfg(target_arch = "x86_64")]
pub use crate::arch::x86::timer::read_monotonic_milli_seconds;

/// A timer invokes a callback function after a specified span of time elapsed.
///
/// A new timer is initially inactive. Only after a timeout value is set with
/// the `set` method can the timer become active and the callback function
/// be triggered.
///
/// Timers are one-shot. If the time is out, one has to set the timer again
/// in order to trigger the callback again.
pub struct Timer {
    function: Arc<dyn Fn(Arc<Self>) + Send + Sync>,
    inner: SpinLock<TimerInner>,
}
#[derive(Default)]
struct TimerInner {
    start_tick: u64,
    timeout_tick: u64,
    timer_callback: Option<Arc<TimerCallback>>,
}

fn timer_callback(callback: &TimerCallback) {
    let data = callback.data();
    if data.is::<Arc<Timer>>() {
        let timer = data.downcast_ref::<Arc<Timer>>().unwrap();
        timer.function.call((timer.clone(),));
    } else {
        panic!("the timer callback is not Timer structure");
    }
}

const NANOS_DIVIDE: u64 = 1_000_000_000 / TIMER_FREQ;

impl Timer {
    /// Creates a new instance, given a callback function.
    pub fn new<F>(f: F) -> Result<Arc<Self>>
    where
        F: Fn(Arc<Timer>) + Send + Sync + 'static,
    {
        Ok(Arc::new(Self {
            function: Arc::new(f),
            inner: SpinLock::new(TimerInner::default()),
        }))
    }

    /// Set a timeout value.
    ///
    /// If a timeout value is already set, the timeout value will be refreshed.
    ///
    pub fn set(self: Arc<Self>, timeout: Duration) {
        let mut lock = self.inner.lock_irq_disabled();
        match &lock.timer_callback {
            Some(callback) => {
                callback.cancel();
            }
            None => {}
        }
        let tick_count =
            timeout.as_secs() * TIMER_FREQ + timeout.subsec_nanos() as u64 / NANOS_DIVIDE;
        let tick = TICK.load(Ordering::SeqCst);
        lock.start_tick = tick;
        lock.timeout_tick = tick + tick_count;
        lock.timer_callback = Some(add_timeout_list(tick_count, self.clone(), timer_callback));
    }

    /// Returns the remaining timeout value.
    ///
    /// If the timer is not set, then the remaining timeout value is zero.
    pub fn remain(&self) -> Duration {
        let lock = self.inner.lock_irq_disabled();
        let tick_remain = {
            let tick = TICK.load(Ordering::SeqCst) as i64;
            lock.timeout_tick as i64 - tick
        };
        if tick_remain <= 0 {
            Duration::new(0, 0)
        } else {
            let second_count = tick_remain as u64 / TIMER_FREQ;
            let remain_count = tick_remain as u64 % TIMER_FREQ;
            Duration::new(second_count, (remain_count * NANOS_DIVIDE) as u32)
        }
    }

    /// Clear the timeout value.
    pub fn clear(&self) {
        let mut lock = self.inner.lock_irq_disabled();
        if let Some(callback) = &lock.timer_callback {
            callback.cancel();
        }
        lock.timeout_tick = 0;
        lock.start_tick = 0;
        lock.timer_callback = None;
    }
}
