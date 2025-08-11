// SPDX-License-Identifier: MPL-2.0

//! Logger injection.
//!
//! OSTD allows its client to inject a custom implementation of logger.
//! If no such logger is injected,
//! then OSTD falls back to a built-in logger that
//! simply dumps all log records with [`crate::console::early_print`].
//!
//! OSTD's logger facility relies on the [log] crate.
//! Both an OSTD client and OSTD itself use the macros from the `log` crate
//! such as `error`, `info`, and `debug` to emit log records.
//! The injected logger is required to implement the [`log::Log`] trait.
//!
//! [log]: https://docs.rs/log

use core::str::FromStr;

use log::{LevelFilter, Metadata, Record};
use spin::Once;

use crate::boot::EARLY_INFO;

/// Injects a logger.
///
/// This method can be called at most once; calling it more than once has no effect.
///
/// # Requirements
///
/// As the logger may be invoked in stringent situations,
/// such as an interrupt handler, an out-of-memory handler, or a panic handler,
/// a logger should be implemented to be
/// _short_ (simple and non-sleeping) and
/// _heapless_ (not trigger heap allocations).
/// Failing to do so may cause the kernel to panic or deadlock.
pub fn inject_logger(new_logger: &'static dyn log::Log) {
    LOGGER.backend.call_once(|| new_logger);
}

/// Initializes the logger. Users should avoid using the log macros before this function is called.
pub(crate) fn init() {
    let level = get_log_level().unwrap_or(LevelFilter::Off);
    log::set_max_level(level);
    log::set_logger(&LOGGER).unwrap();
}

static LOGGER: Logger = Logger::new();

struct Logger {
    backend: Once<&'static dyn log::Log>,
}

impl Logger {
    const fn new() -> Self {
        Self {
            backend: Once::new(),
        }
    }
}

impl log::Log for Logger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        if let Some(logger) = self.backend.get() {
            return logger.enabled(metadata);
        };

        // Default implementation.
        true
    }

    fn log(&self, record: &Record) {
        if let Some(logger) = self.backend.get() {
            return logger.log(record);
        };

        // Default implementation.
        let level = record.level();
        crate::console::early_print(format_args!("{}: {}\n", level, record.args()));
    }

    fn flush(&self) {
        if let Some(logger) = self.backend.get() {
            logger.flush();
        };
    }
}

fn get_log_level() -> Option<LevelFilter> {
    let kcmdline = EARLY_INFO.get().unwrap().kernel_cmdline;

    // Although OSTD is agnostic of the parsing of the kernel command line,
    // the logger assumes that it follows the Linux kernel command line format.
    // We search for the `ostd.log_level=ARGUMENT` pattern in string.
    let value = kcmdline
        .split(' ')
        .find(|arg| arg.starts_with("ostd.log_level="))
        .map(|arg| arg.split('=').next_back().unwrap_or_default())?;

    LevelFilter::from_str(value).ok()
}
