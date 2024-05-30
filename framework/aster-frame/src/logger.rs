// SPDX-License-Identifier: MPL-2.0

//! Logging support.

use log::{Level, Metadata, Record};

use crate::early_println;

const LOGGER: Logger = Logger {};

/// The log level.
///
/// FIXME: The logs should be able to be read from files in the userspace,
/// and the log level should be configurable.
pub const INIT_LOG_LEVEL: Level = Level::Error;

struct Logger {}

impl log::Log for Logger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= INIT_LOG_LEVEL
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) {
            early_println!("[{}]: {}", record.level(), record.args());
        }
    }

    fn flush(&self) {}
}

pub(crate) fn init() {
    log::set_logger(&LOGGER)
        .map(|()| log::set_max_level(INIT_LOG_LEVEL.to_level_filter()))
        .unwrap();
}
