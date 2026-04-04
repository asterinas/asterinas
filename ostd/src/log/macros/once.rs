// SPDX-License-Identifier: MPL-2.0

//! Print-once logging macros.

/// Logs at the [`Emerg`] level, but only the first time this call site is reached.
///
/// [`Emerg`]: crate::log::Level::Emerg
#[macro_export]
macro_rules! emerg_once {
    ($($arg:tt)+) => {{
        static __ONCE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
        if !__ONCE.load(core::sync::atomic::Ordering::Relaxed) {
            __ONCE.store(true, core::sync::atomic::Ordering::Relaxed);
            $crate::emerg!($($arg)+);
        }
    }};
}

/// Logs at the [`Alert`] level, but only the first time this call site is reached.
///
/// [`Alert`]: crate::log::Level::Alert
#[macro_export]
macro_rules! alert_once {
    ($($arg:tt)+) => {{
        static __ONCE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
        if !__ONCE.load(core::sync::atomic::Ordering::Relaxed) {
            __ONCE.store(true, core::sync::atomic::Ordering::Relaxed);
            $crate::alert!($($arg)+);
        }
    }};
}

/// Logs at the [`Crit`] level, but only the first time this call site is reached.
///
/// [`Crit`]: crate::log::Level::Crit
#[macro_export]
macro_rules! crit_once {
    ($($arg:tt)+) => {{
        static __ONCE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
        if !__ONCE.load(core::sync::atomic::Ordering::Relaxed) {
            __ONCE.store(true, core::sync::atomic::Ordering::Relaxed);
            $crate::crit!($($arg)+);
        }
    }};
}

/// Logs at the [`Error`] level, but only the first time this call site is reached.
///
/// [`Error`]: crate::log::Level::Error
#[macro_export]
macro_rules! error_once {
    ($($arg:tt)+) => {{
        static __ONCE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
        if !__ONCE.load(core::sync::atomic::Ordering::Relaxed) {
            __ONCE.store(true, core::sync::atomic::Ordering::Relaxed);
            $crate::error!($($arg)+);
        }
    }};
}

/// Logs at the [`Warning`] level, but only the first time this call site is reached.
///
/// [`Warning`]: crate::log::Level::Warning
#[macro_export]
macro_rules! warn_once {
    ($($arg:tt)+) => {{
        static __ONCE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
        if !__ONCE.load(core::sync::atomic::Ordering::Relaxed) {
            __ONCE.store(true, core::sync::atomic::Ordering::Relaxed);
            $crate::warn!($($arg)+);
        }
    }};
}

/// Logs at the [`Notice`] level, but only the first time this call site is reached.
///
/// [`Notice`]: crate::log::Level::Notice
#[macro_export]
macro_rules! notice_once {
    ($($arg:tt)+) => {{
        static __ONCE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
        if !__ONCE.load(core::sync::atomic::Ordering::Relaxed) {
            __ONCE.store(true, core::sync::atomic::Ordering::Relaxed);
            $crate::notice!($($arg)+);
        }
    }};
}

/// Logs at the [`Info`] level, but only the first time this call site is reached.
///
/// [`Info`]: crate::log::Level::Info
#[macro_export]
macro_rules! info_once {
    ($($arg:tt)+) => {{
        static __ONCE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
        if !__ONCE.load(core::sync::atomic::Ordering::Relaxed) {
            __ONCE.store(true, core::sync::atomic::Ordering::Relaxed);
            $crate::info!($($arg)+);
        }
    }};
}

/// Logs at the [`Debug`] level, but only the first time this call site is reached.
///
/// [`Debug`]: crate::log::Level::Debug
#[macro_export]
macro_rules! debug_once {
    ($($arg:tt)+) => {{
        static __ONCE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
        if !__ONCE.load(core::sync::atomic::Ordering::Relaxed) {
            __ONCE.store(true, core::sync::atomic::Ordering::Relaxed);
            $crate::debug!($($arg)+);
        }
    }};
}
