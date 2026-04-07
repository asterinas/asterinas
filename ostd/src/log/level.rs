// SPDX-License-Identifier: MPL-2.0

//! Log level and level filter types.

use core::fmt;

/// Kernel log level, matching the severity levels described in `syslog(2)`.
///
/// # Ordering
///
/// Levels are ordered by severity: `Emerg < Alert < ... < Debug`.
/// `a < b` means `a` is more severe than `b`.
///
/// Higher severity is assigned lower numeric value.
/// For example,
/// `Level::Emerg` has the smallest value of 0 and means the highest severity.
/// On the end of the spectrum,
/// `Level::Debug` has the largest value of 7 and means the lowest severity.
///
/// # Mapping from Linux's logging levels
///
/// The numeric values of `Level::Xxx` are assigned to those of their Linux counterparts.
///
/// ```text
/// LOGLEVEL_EMERG      0   /* system is unusable */
/// LOGLEVEL_ALERT      1   /* action must be taken immediately */
/// LOGLEVEL_CRIT       2   /* critical conditions */
/// LOGLEVEL_ERR        3   /* error conditions */
/// LOGLEVEL_WARNING    4   /* warning conditions */
/// LOGLEVEL_NOTICE     5   /* normal but significant condition */
/// LOGLEVEL_INFO       6   /* informational */
/// LOGLEVEL_DEBUG      7   /* debug-level messages */
/// ```
///
/// # Mapping from the `log` crate
///
/// | `log::Level` | `ostd::log::Level` | Notes |
/// |--------------|--------------------| ------|
/// | Error        | Error (3)          | |
/// | Warn         | Warning (4)        | |
/// | Info         | Info (6)           | |
/// | Debug        | Debug (7)          | |
/// | Trace        | Debug (7)          | Bridge only; no `trace!` macro in OSTD |
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, PartialOrd, Ord)]
#[repr(u8)]
pub enum Level {
    /// System is unusable.
    Emerg = 0,
    /// Action must be taken immediately.
    Alert = 1,
    /// Critical conditions.
    Crit = 2,
    /// Error conditions.
    Error = 3,
    /// Warning conditions.
    Warning = 4,
    /// Normal but significant condition.
    Notice = 5,
    /// Informational.
    Info = 6,
    /// Debug-level messages.
    Debug = 7,
}

impl Level {
    /// Creates a `Level` from a numeric value (0--7).
    ///
    /// Values > 7 are clamped to `Debug`.
    pub const fn from_u8(val: u8) -> Self {
        match val {
            0 => Self::Emerg,
            1 => Self::Alert,
            2 => Self::Crit,
            3 => Self::Error,
            4 => Self::Warning,
            5 => Self::Notice,
            6 => Self::Info,
            _ => Self::Debug,
        }
    }
}

impl fmt::Display for Level {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(match self {
            Level::Emerg => "EMERG",
            Level::Alert => "ALERT",
            Level::Crit => "CRIT",
            Level::Error => "ERROR",
            Level::Warning => "WARN",
            Level::Notice => "NOTICE",
            Level::Info => "INFO",
            Level::Debug => "DEBUG",
        })
    }
}

/// A filter for log levels.
///
/// `LevelFilter::from_level(level)` includes that level and all more-severe levels.
/// `LevelFilter::Off` disables all logging.
///
/// The filtering rule: a level passes when `(filter as u8) > (level as u8)`.
///
/// ```text
/// LevelFilter::Off(0)     -> nothing passes
/// LevelFilter::Error(4)   -> Emerg(0), Alert(1), Crit(2), Error(3)
/// LevelFilter::Debug(8)   -> everything
/// ```
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, PartialOrd, Ord)]
#[repr(u8)]
pub enum LevelFilter {
    /// All logging disabled.
    Off = 0,
    /// Enable Emerg only.
    Emerg = 1,
    /// Enable Emerg and Alert.
    Alert = 2,
    /// Enable Emerg through Crit.
    Crit = 3,
    /// Enable Emerg through Error.
    Error = 4,
    /// Enable Emerg through Warning.
    Warning = 5,
    /// Enable Emerg through Notice.
    Notice = 6,
    /// Enable Emerg through Info.
    Info = 7,
    /// Enable all levels.
    Debug = 8,
}

impl LevelFilter {
    /// Returns `true` if `level` passes this filter.
    #[inline]
    pub const fn is_enabled(self, level: Level) -> bool {
        (self as u8) > (level as u8)
    }

    /// Constructs a filter that enables `level` and all more-severe levels.
    pub const fn from_level(level: Level) -> Self {
        match level {
            Level::Emerg => Self::Emerg,
            Level::Alert => Self::Alert,
            Level::Crit => Self::Crit,
            Level::Error => Self::Error,
            Level::Warning => Self::Warning,
            Level::Notice => Self::Notice,
            Level::Info => Self::Info,
            Level::Debug => Self::Debug,
        }
    }

    /// Creates a `LevelFilter` from a numeric value (0--8).
    ///
    /// Values > 8 are clamped to `Debug`.
    pub const fn from_u8(val: u8) -> Self {
        match val {
            0 => Self::Off,
            1 => Self::Emerg,
            2 => Self::Alert,
            3 => Self::Crit,
            4 => Self::Error,
            5 => Self::Warning,
            6 => Self::Notice,
            7 => Self::Info,
            _ => Self::Debug,
        }
    }
}

#[cfg(ktest)]
mod test {
    use super::*;
    use crate::prelude::*;

    #[ktest]
    fn level_ordering() {
        assert!(Level::Emerg < Level::Alert);
        assert!(Level::Alert < Level::Crit);
        assert!(Level::Crit < Level::Error);
        assert!(Level::Error < Level::Warning);
        assert!(Level::Warning < Level::Notice);
        assert!(Level::Notice < Level::Info);
        assert!(Level::Info < Level::Debug);
    }

    #[ktest]
    fn level_filter_enabled() {
        assert!(!LevelFilter::Off.is_enabled(Level::Emerg));
        assert!(LevelFilter::Emerg.is_enabled(Level::Emerg));
        assert!(!LevelFilter::Emerg.is_enabled(Level::Alert));
        assert!(LevelFilter::Error.is_enabled(Level::Error));
        assert!(LevelFilter::Error.is_enabled(Level::Crit));
        assert!(!LevelFilter::Error.is_enabled(Level::Warning));
        assert!(LevelFilter::Debug.is_enabled(Level::Debug));
    }

    #[ktest]
    fn level_from_u8_clamping() {
        assert_eq!(Level::from_u8(0), Level::Emerg);
        assert_eq!(Level::from_u8(7), Level::Debug);
        assert_eq!(Level::from_u8(255), Level::Debug);
    }

    #[ktest]
    fn level_filter_from_level() {
        assert_eq!(LevelFilter::from_level(Level::Error), LevelFilter::Error);
        assert_eq!(LevelFilter::from_level(Level::Debug), LevelFilter::Debug);
    }
}
