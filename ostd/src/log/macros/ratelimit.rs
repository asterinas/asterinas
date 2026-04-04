// SPDX-License-Identifier: MPL-2.0

//! Rate-limited logging macros and rate limiter state.

use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// Per-call-site rate limiter for the `*_ratelimited!` macros.
///
/// Allows up to [`DEFAULT_RATELIMIT_BURST`] messages per
/// [`DEFAULT_RATELIMIT_INTERVAL_MS`]-millisecond window.
///
/// # Limitations
///
/// Rate limiting requires a working timer (`Jiffies::elapsed()`). Before the
/// timer subsystem is initialized during early boot, rate limiting does not
/// function correctly. This is not a practical concern -- early boot code
/// rarely loops.
pub struct RateLimitState {
    window_start: AtomicU64,
    count: AtomicU32,
}

/// Default rate limit burst: 10 messages per interval.
pub const DEFAULT_RATELIMIT_BURST: u32 = 10;

/// Default rate limit interval: 5 seconds.
pub const DEFAULT_RATELIMIT_INTERVAL_MS: u64 = 5_000;

impl Default for RateLimitState {
    fn default() -> Self {
        Self::new()
    }
}

impl RateLimitState {
    /// Creates a new rate limiter. `const` for use in `static` items.
    pub const fn new() -> Self {
        Self {
            window_start: AtomicU64::new(0),
            count: AtomicU32::new(0),
        }
    }

    /// Returns `true` if the caller should emit the log message.
    pub fn try_acquire(&self) -> bool {
        let now_ms = crate::timer::Jiffies::elapsed().as_duration().as_millis() as u64;
        let start = self.window_start.load(Ordering::Relaxed);

        if now_ms.wrapping_sub(start) >= DEFAULT_RATELIMIT_INTERVAL_MS
            && self
                .window_start
                .compare_exchange(start, now_ms, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
        {
            self.count.store(1, Ordering::Relaxed);
            return true;
        }
        self.count.fetch_add(1, Ordering::Relaxed) < DEFAULT_RATELIMIT_BURST
    }
}

/// Logs at the [`Emerg`] level, rate-limited per call site
/// by [`RateLimitState`](crate::log::RateLimitState)
/// (default: 10 messages per 5-second window).
///
/// [`Emerg`]: crate::log::Level::Emerg
#[macro_export]
macro_rules! emerg_ratelimited {
    ($($arg:tt)+) => {{
        static __RATE_LIMIT: $crate::log::RateLimitState = $crate::log::RateLimitState::new();
        if __RATE_LIMIT.try_acquire() {
            $crate::emerg!($($arg)+);
        }
    }};
}

/// Logs at the [`Alert`] level, rate-limited per call site
/// by [`RateLimitState`](crate::log::RateLimitState)
/// (default: 10 messages per 5-second window).
///
/// [`Alert`]: crate::log::Level::Alert
#[macro_export]
macro_rules! alert_ratelimited {
    ($($arg:tt)+) => {{
        static __RATE_LIMIT: $crate::log::RateLimitState = $crate::log::RateLimitState::new();
        if __RATE_LIMIT.try_acquire() {
            $crate::alert!($($arg)+);
        }
    }};
}

/// Logs at the [`Crit`] level, rate-limited per call site
/// by [`RateLimitState`](crate::log::RateLimitState)
/// (default: 10 messages per 5-second window).
///
/// [`Crit`]: crate::log::Level::Crit
#[macro_export]
macro_rules! crit_ratelimited {
    ($($arg:tt)+) => {{
        static __RATE_LIMIT: $crate::log::RateLimitState = $crate::log::RateLimitState::new();
        if __RATE_LIMIT.try_acquire() {
            $crate::crit!($($arg)+);
        }
    }};
}

/// Logs at the [`Error`] level, rate-limited per call site
/// by [`RateLimitState`](crate::log::RateLimitState)
/// (default: 10 messages per 5-second window).
///
/// [`Error`]: crate::log::Level::Error
#[macro_export]
macro_rules! error_ratelimited {
    ($($arg:tt)+) => {{
        static __RATE_LIMIT: $crate::log::RateLimitState = $crate::log::RateLimitState::new();
        if __RATE_LIMIT.try_acquire() {
            $crate::error!($($arg)+);
        }
    }};
}

/// Logs at the [`Warning`] level, rate-limited per call site
/// by [`RateLimitState`](crate::log::RateLimitState)
/// (default: 10 messages per 5-second window).
///
/// [`Warning`]: crate::log::Level::Warning
#[macro_export]
macro_rules! warn_ratelimited {
    ($($arg:tt)+) => {{
        static __RATE_LIMIT: $crate::log::RateLimitState = $crate::log::RateLimitState::new();
        if __RATE_LIMIT.try_acquire() {
            $crate::warn!($($arg)+);
        }
    }};
}

/// Logs at the [`Notice`] level, rate-limited per call site
/// by [`RateLimitState`](crate::log::RateLimitState)
/// (default: 10 messages per 5-second window).
///
/// [`Notice`]: crate::log::Level::Notice
#[macro_export]
macro_rules! notice_ratelimited {
    ($($arg:tt)+) => {{
        static __RATE_LIMIT: $crate::log::RateLimitState = $crate::log::RateLimitState::new();
        if __RATE_LIMIT.try_acquire() {
            $crate::notice!($($arg)+);
        }
    }};
}

/// Logs at the [`Info`] level, rate-limited per call site
/// by [`RateLimitState`](crate::log::RateLimitState)
/// (default: 10 messages per 5-second window).
///
/// [`Info`]: crate::log::Level::Info
#[macro_export]
macro_rules! info_ratelimited {
    ($($arg:tt)+) => {{
        static __RATE_LIMIT: $crate::log::RateLimitState = $crate::log::RateLimitState::new();
        if __RATE_LIMIT.try_acquire() {
            $crate::info!($($arg)+);
        }
    }};
}

/// Logs at the [`Debug`] level, rate-limited per call site
/// by [`RateLimitState`](crate::log::RateLimitState)
/// (default: 10 messages per 5-second window).
///
/// [`Debug`]: crate::log::Level::Debug
#[macro_export]
macro_rules! debug_ratelimited {
    ($($arg:tt)+) => {{
        static __RATE_LIMIT: $crate::log::RateLimitState = $crate::log::RateLimitState::new();
        if __RATE_LIMIT.try_acquire() {
            $crate::debug!($($arg)+);
        }
    }};
}

#[cfg(ktest)]
mod test {
    use super::*;
    use crate::prelude::*;

    #[ktest]
    fn ratelimit_first_call_passes() {
        let state = RateLimitState::new();
        assert!(state.try_acquire());
    }

    #[ktest]
    fn ratelimit_burst_exhaustion() {
        let state = RateLimitState::new();
        for _ in 0..DEFAULT_RATELIMIT_BURST {
            assert!(state.try_acquire());
        }
        // The (BURST+1)-th call should be rejected.
        assert!(!state.try_acquire());
    }
}
