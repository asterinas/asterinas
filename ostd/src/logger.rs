// SPDX-License-Identifier: MPL-2.0

//! Logging support.
//!
//! Currently the logger prints the logs to the console.
//!
//! This module guarantees _atomicity_ under concurrency: messages are always
//! printed in their entirety without being mixed with messages generated
//! concurrently on other cores.
//!
//! IRQs are disabled while printing. So do not print long log messages.

use core::str::FromStr;

use log::{LevelFilter, Metadata, Record};
use spin::Once;

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

/// Sets the backend for the global logger, allowing upper-level users to register
/// their own implemented loggers.
///
/// This method only allows setting once; subsequent attempts to set it again will have no effect.
pub fn set_logger(new_logger: &'static dyn log::Log) {
    LOGGER.backend.call_once(|| new_logger);
}

impl log::Log for Logger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        if let Some(logger) = self.backend.get() {
            return logger.enabled(metadata);
        };

        // Default instruction.
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record) {
        if let Some(logger) = self.backend.get() {
            return logger.log(record);
        };

        // Default instruction.
        let level = record.level();

        // Use a global lock to prevent interleaving of log messages.
        use crate::sync::SpinLock;
        static RECORD_LOCK: SpinLock<()> = SpinLock::new(());
        let _lock = RECORD_LOCK.disable_irq().lock();

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
    let log_level = option_env!("LOG_LEVEL")?;
    LevelFilter::from_str(log_level).ok()
}
