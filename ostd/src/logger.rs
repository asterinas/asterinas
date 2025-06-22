// SPDX-License-Identifier: MPL-2.0

//! Logging support.
//!
//! This module provides a default log implementation while allowing users to inject
//! their own logger at a higher level.
//!
//! Generally IRQs are disabled while printing. So do not print long log messages.

use core::str::FromStr;

use log::{LevelFilter, Metadata, Record};
use spin::Once;

use crate::boot::EARLY_INFO;

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

/// Injects a logger as the global logger backend.
///
/// This method allows upper-level users to inject their own implemented loggers,
/// but only allows injecting once. Subsequent injection will have no effect.
///
/// **Caution**: The implementation of log operation in the injected logger should ideally be
/// heap-free and not involve sleep operations. Otherwise, users should refrain from calling `log`
/// in sensitive locations, such as during heap allocations, as this may cause the system to block.
pub fn inject_logger(new_logger: &'static dyn log::Log) {
    LOGGER.backend.call_once(|| new_logger);
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

/// Initialize the logger. Users should avoid using the log macros before this function is called.
pub(crate) fn init() {
    let level = get_log_level().unwrap_or(LevelFilter::Off);
    log::set_max_level(level);
    log::set_logger(&LOGGER).unwrap();
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
