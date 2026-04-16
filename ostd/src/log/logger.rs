// SPDX-License-Identifier: MPL-2.0

//! Logger backend trait and global state.

use core::{
    fmt,
    sync::atomic::{AtomicU8, Ordering},
};

use spin::Once;

use super::{Level, LevelFilter, bridge::sync_log_crate_max_level};

static LOGGER: Once<&'static dyn Log> = Once::new();

/// Registers the global logger backend.
///
/// The function may be called only once; subsequent calls take no effect.
pub fn inject_logger(logger: &'static dyn Log) {
    LOGGER.call_once(|| logger);
}

/// Returns the registered logger, if any.
#[inline]
pub(super) fn __logger() -> Option<&'static dyn Log> {
    LOGGER.get().copied()
}

/// Writes a log record to the registered logger,
/// or falls back to early console output
/// if no logger has been registered yet.
///
/// Called by the `log!` macro. Not intended for direct use.
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
/// - **Console flushing is best-effort.** After recording the message,
///   the implementation may attempt to flush pending messages to console
///   devices synchronously. In contended or non-blockable contexts
///   (e.g., scheduler code), the implementation should skip or defer
///   flushing rather than blocking.
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

// -- Maximum log level --

/// Compile-time maximum log level.
// TODO: Add cargo features (e.g., `log_max_level_info`) to
// set `STATIC_MAX_LEVEL` at compile time
// so that log calls above the chosen level are eliminated entirely.
// The same feature should activate the corresponding `log` crate feature
// (e.g., `log/max_level_info`)
// so that both OSTD macros and third-party `log::info!()` calls are filtered uniformly.
pub const STATIC_MAX_LEVEL: LevelFilter = LevelFilter::Debug;

/// Run-time maximum log level.
static DYNAMIC_MAX_LEVEL: AtomicU8 = {
    // By default, the run-runtime max log level is off,
    // which can be overridden
    AtomicU8::new(LevelFilter::Off as u8)
};

/// Sets the runtime maximum log level.
///
/// If the given `filter` argument is greater than [`STATIC_MAX_LEVEL`],
/// then the runtime maximum log level is set to `STATIC_MAX_LEVEL`.
///
/// This function also updates the `log` crate's max level
/// so that third-party crates using `log::info!()` etc. are filtered consistently.
///
/// # Requirements
///
/// This function is intended to be called sequentially:
/// concurrent calls to this function may cause the maximum levels
/// kept by OSTD and `log` to diverge.
pub fn set_max_level(mut filter: LevelFilter) {
    if filter > STATIC_MAX_LEVEL {
        filter = STATIC_MAX_LEVEL;
    }

    DYNAMIC_MAX_LEVEL.store(filter as u8, Ordering::Relaxed);
    sync_log_crate_max_level(filter);
}

/// Returns the current runtime maximum log level.
#[inline]
pub fn max_level() -> LevelFilter {
    LevelFilter::from_u8(DYNAMIC_MAX_LEVEL.load(Ordering::Relaxed))
}
