// SPDX-License-Identifier: MPL-2.0

//! Logger backend trait and global state.

use core::{
    fmt,
    sync::atomic::{AtomicU8, Ordering},
};

use spin::Once;

use super::{Level, LevelFilter, bridge::sync_log_crate_max_level};

/// A single log record carrying level, message, and source location.
///
/// Records are created by the logging macros
/// and passed to the [`Log`] backend.
/// They are transient —
/// the backend must consume all data during the `log()` call.
pub struct Record<'a> {
    level: Level,
    prefix: &'static str,
    args: fmt::Arguments<'a>,
    module_path: &'static str,
    file: &'static str,
    line: u32,
}

impl<'a> Record<'a> {
    /// Creates a new record. Called by the logging macros.
    #[doc(hidden)]
    #[inline]
    pub fn new(
        level: Level,
        prefix: &'static str,
        args: fmt::Arguments<'a>,
        module_path: &'static str,
        file: &'static str,
        line: u32,
    ) -> Self {
        Self {
            level,
            prefix,
            args,
            module_path,
            file,
            line,
        }
    }

    /// Returns the log level.
    pub fn level(&self) -> Level {
        self.level
    }

    /// Returns the per-module log prefix (may be empty).
    pub fn prefix(&self) -> &'static str {
        self.prefix
    }

    /// Returns the formatted message arguments.
    pub fn args(&self) -> &fmt::Arguments<'a> {
        &self.args
    }

    /// Returns the full module path where the log call originated.
    pub fn module_path(&self) -> &'static str {
        self.module_path
    }

    /// Returns the source file path.
    pub fn file(&self) -> &'static str {
        self.file
    }

    /// Returns the source line number.
    pub fn line(&self) -> u32 {
        self.line
    }
}

/// The logger backend trait.
///
/// Implement this trait and register it with [`inject_logger()`] to receive log
/// records from the OSTD logging macros.
///
/// # Implementation guidelines
///
/// The logging macros can be called from **any context**: interrupt handlers,
/// early boot, OOM handlers, or panic handlers. An implementation should
/// be designed to work correctly in all of these contexts. In practice:
///
/// - **The ring buffer write must be heapless and lock-free (or IRQ-safe).**
///   The part of `log()` that records the message must not allocate from the
///   heap and must use either a lock-free data structure or an IRQ-disabled
///   spinlock, so that it is safe from any context.
///
/// - **Console flushing may block.** After recording the message, the
///   implementation may attempt to flush pending messages to console devices
///   synchronously. This can block on a console lock when contended. In
///   contexts where blocking is unsafe (scheduler code), the
///   implementation should defer console flushing.
///
/// - **The implementation should be short.** Long-running work can stall the
///   calling CPU. Implementations should bound the work per `log()` call.
pub trait Log: Sync + Send {
    /// Logs a record.
    ///
    /// The caller (the `log!` macro) has already verified that the record's
    /// level passes both the compile-time and runtime level filters. The
    /// backend does not need to re-check the level.
    fn log(&self, record: &Record);
}

// -- Global state --

static LOGGER: Once<&'static dyn Log> = Once::new();
static MAX_LEVEL: AtomicU8 = AtomicU8::new(0); // LevelFilter::Off

/// Compile-time maximum log level. Log calls above this level are compiled
/// out entirely.
pub const STATIC_MAX_LEVEL: LevelFilter = LevelFilter::Debug;

/// Registers the global logger backend.
///
/// Can be called at most once. Subsequent calls are silently ignored.
pub fn inject_logger(logger: &'static dyn Log) {
    LOGGER.call_once(|| logger);
}

/// Sets the runtime maximum log level.
///
/// Also updates the `log` crate's max level so that third-party crates
/// using `log::info!()` etc. are filtered consistently.
pub fn set_max_level(filter: LevelFilter) {
    MAX_LEVEL.store(filter as u8, Ordering::Relaxed);
    sync_log_crate_max_level(filter);
}

/// Returns the current runtime maximum log level.
#[inline]
pub fn max_level() -> LevelFilter {
    LevelFilter::from_u8(MAX_LEVEL.load(Ordering::Relaxed))
}

/// Returns the registered logger, if any.
#[inline]
pub(super) fn __logger() -> Option<&'static dyn Log> {
    LOGGER.get().copied()
}

/// Writes a log record to the registered logger, or falls back to
/// early console output if no logger has been registered yet.
///
/// This is called by the `log!` macro. It is not intended for direct use.
#[doc(hidden)]
pub fn __write_log_record(record: &Record) {
    if let Some(logger) = __logger() {
        logger.log(record);
    } else {
        crate::console::early_print(format_args!(
            "{}: {}{}\n",
            record.level(),
            record.prefix(),
            record.args()
        ));
    }
}
