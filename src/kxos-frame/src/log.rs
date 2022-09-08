use core::fmt::Arguments;

use crate::config::DEFAULT_LOG_LEVEL;

/// Print log message
/// This function should *NOT* be directly called.
/// Instead, print logs with macros.
#[cfg(not(feature = "serial_print"))]
#[doc(hidden)]
pub fn log_print(args: Arguments) {
    use crate::device::framebuffer::WRITER;
    use core::fmt::Write;
    use x86_64::instructions::interrupts;

    interrupts::without_interrupts(|| {
        WRITER.lock().as_mut().unwrap().write_fmt(args).unwrap();
    });
}

/// Print log message
/// This function should *NOT* be directly called.
/// Instead, print logs with macros.
#[cfg(feature = "serial_print")]
#[doc(hidden)]
pub fn log_print(args: Arguments) {
    use crate::device::serial::SERIAL;
    use core::fmt::Write;
    use x86_64::instructions::interrupts;
    interrupts::without_interrupts(|| {
        SERIAL
            .lock()
            .write_fmt(args)
            .expect("Printing to serial failed");
    });
}

/// This macro should not be directly called.
#[macro_export]
macro_rules! log_print {
    ($($arg:tt)*) => {
        $crate::log::log_print(format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! trace {
    ($($arg:tt)*) => {
        if $crate::log::Logger::trace() {
            $crate::log_print!("[trace]:");
            $crate::log_print!($($arg)*);
            $crate::log_print!("\n");
        }
    };
}

#[macro_export]
macro_rules! debug {
    ($($arg:tt)*) => {
        if $crate::log::Logger::debug() {
            $crate::log_print!("[debug]:");
            $crate::log_print!($($arg)*);
            $crate::log_print!("\n");
        }
    };
}

#[macro_export]
macro_rules! info {
    ($($arg:tt)*) => {
        if $crate::log::Logger::info() {
            ($crate::log_print!("[info]:"));
            ($crate::log_print!($($arg)*));
            ($crate::log_print!("\n"));
        }
    };
}

#[macro_export]
macro_rules! warn {
    ($($arg:tt)*) => {
        if $crate::log::Logger::warn() {
            $crate::log_print!("[warn]:");
            $crate::log_print!($($arg)*);
            $crate::log_print!("\n");
        }
    };
}

#[macro_export]
macro_rules! error {
    ($($arg:tt)*) => {
        if $crate::log::Logger::error() {
            $crate::log_print!("[error]:");
            $crate::log_print!($($arg)*);
            $crate::log_print!("\n");
        }
    };
}

pub const LOGGER: Logger = Logger::default_log_level();

pub struct Logger {
    log_level: LogLevel,
}

impl Logger {
    pub const fn default_log_level() -> Logger {
        Logger {
            log_level: DEFAULT_LOG_LEVEL,
        }
    }

    pub fn trace() -> bool {
        LOGGER.log_level <= LogLevel::Trace
    }

    pub fn debug() -> bool {
        LOGGER.log_level <= LogLevel::Debug
    }

    pub fn info() -> bool {
        LOGGER.log_level <= LogLevel::Info
    }

    pub fn warn() -> bool {
        LOGGER.log_level <= LogLevel::Warn
    }

    pub fn error() -> bool {
        LOGGER.log_level <= LogLevel::Error
    }
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}
