// SPDX-License-Identifier: MPL-2.0

//! Logging macros.
//!
//! Contains the core [`log!`](crate::log!) macro and
//! level-specific wrappers (`info!`, `warn!`, etc.).
//! All are `#[macro_export]` so downstream crates can access them
//! via `use ostd::info;` etc.

/// Logs a message at the [`Emerg`] level.
///
/// [`Emerg`]: crate::log::Level::Emerg
#[macro_export]
macro_rules! emerg {
    ($($arg:tt)+) => { $crate::log!($crate::log::Level::Emerg, $($arg)+) };
}

/// Logs a message at the [`Alert`] level.
///
/// [`Alert`]: crate::log::Level::Alert
#[macro_export]
macro_rules! alert {
    ($($arg:tt)+) => { $crate::log!($crate::log::Level::Alert, $($arg)+) };
}

/// Logs a message at the [`Crit`] level.
///
/// [`Crit`]: crate::log::Level::Crit
#[macro_export]
macro_rules! crit {
    ($($arg:tt)+) => { $crate::log!($crate::log::Level::Crit, $($arg)+) };
}

/// Logs a message at the [`Error`] level.
///
/// [`Error`]: crate::log::Level::Error
#[macro_export]
macro_rules! error {
    ($($arg:tt)+) => { $crate::log!($crate::log::Level::Error, $($arg)+) };
}

/// Logs a message at the [`Warning`] level.
///
/// [`Warning`]: crate::log::Level::Warning
#[macro_export]
macro_rules! warn {
    ($($arg:tt)+) => { $crate::log!($crate::log::Level::Warning, $($arg)+) };
}

/// Logs a message at the [`Notice`] level.
///
/// [`Notice`]: crate::log::Level::Notice
#[macro_export]
macro_rules! notice {
    ($($arg:tt)+) => { $crate::log!($crate::log::Level::Notice, $($arg)+) };
}

/// Logs a message at the [`Info`] level.
///
/// [`Info`]: crate::log::Level::Info
#[macro_export]
macro_rules! info {
    ($($arg:tt)+) => { $crate::log!($crate::log::Level::Info, $($arg)+) };
}

/// Logs a message at the [`Debug`] level.
///
/// [`Debug`]: crate::log::Level::Debug
#[macro_export]
macro_rules! debug {
    ($($arg:tt)+) => { $crate::log!($crate::log::Level::Debug, $($arg)+) };
}

/// Returns `true` if a message at the given level would be logged.
///
/// Checks both the compile-time [`STATIC_MAX_LEVEL`] and the runtime
/// [`max_level`] filters.
///
/// [`STATIC_MAX_LEVEL`]: crate::log::STATIC_MAX_LEVEL
/// [`max_level`]: crate::log::max_level
#[macro_export]
macro_rules! log_enabled {
    ($level:expr) => {{
        let __level: $crate::log::Level = $level;
        $crate::log::STATIC_MAX_LEVEL.is_enabled(__level)
            && $crate::log::max_level().is_enabled(__level)
    }};
}

/// Logs a message at the given level.
///
/// This is the core logging macro.
/// All level-specific macros delegate to it.
///
/// # Examples
///
/// ```rust,ignore
/// use ostd::log::Level;
/// ostd::log!(Level::Info, "message");
/// ostd::log!(Level::Warning, "value = {}", x);
/// ```
#[macro_export]
macro_rules! log {
    ($level:expr, $($arg:tt)+) => {{
        // `const` is intentional: it enables compile-time dead code
        // elimination so that log calls above `STATIC_MAX_LEVEL` are
        // removed entirely.
        const __LEVEL: $crate::log::Level = $level;
        if $crate::log_enabled!(__LEVEL) {
            $crate::log::__write_log_record(&$crate::log::Record::new(
                __LEVEL,
                format_args!($($arg)+),
                module_path!(),
                file!(),
                line!(),
            ));
        }
    }};
}
