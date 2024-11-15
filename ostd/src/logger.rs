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

use crate::boot::{kcmdline::ModuleArg, kernel_cmdline};

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
    let module_args = kernel_cmdline().get_module_args("ostd")?;

    let value = {
        let value = module_args.iter().find_map(|arg| match arg {
            ModuleArg::KeyVal(name, value) if name.as_bytes() == "log_level".as_bytes() => {
                Some(value)
            }
            _ => None,
        })?;
        value.as_c_str().to_str().ok()?
    };
    LevelFilter::from_str(value).ok()
}
