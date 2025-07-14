// SPDX-License-Identifier: MPL-2.0

use alloc::string::ToString;
use log::{Metadata, Record};
use ostd::{sync::SpinLock, timer::Jiffies};

/// Callback function type for syslog integration
pub type SyslogCallback = fn(level: log::Level, message: &str);

/// Global syslog callback protected by spinlock
static SYSLOG_CALLBACK: SpinLock<Option<SyslogCallback>> = SpinLock::new(None);

/// Console log level (default is 7 - show all messages)
static CONSOLE_LOG_LEVEL: SpinLock<i32> = SpinLock::new(7);

/// Register a callback for syslog integration
pub fn register_syslog_callback(callback: SyslogCallback) {
    *SYSLOG_CALLBACK.lock() = Some(callback);
}

/// Update the console log level from external components
pub fn set_console_log_level(level: i32) {
    *CONSOLE_LOG_LEVEL.lock() = level;
}

/// Get current console log level
pub fn get_console_log_level() -> i32 {
    *CONSOLE_LOG_LEVEL.lock()
}

/// Convert log level to console priority level (matching Linux kernel levels)
fn log_level_to_console_level(level: log::Level) -> i32 {
    match level {
        log::Level::Error => 3,   // KERN_ERR
        log::Level::Warn => 4,    // KERN_WARNING  
        log::Level::Info => 6,    // KERN_INFO
        log::Level::Debug => 7,   // KERN_DEBUG
        log::Level::Trace => 8,   // KERN_DEBUG (highest verbosity)
    }
}

/// The logger used for Asterinas.
struct AsterLogger;

static LOGGER: AsterLogger = AsterLogger;

impl log::Log for AsterLogger {
    fn enabled(&self, _metadata: &Metadata) -> bool {
        true
    }

    fn log(&self, record: &Record) {
        let timestamp = Jiffies::elapsed().as_duration().as_secs_f64();
        
        // Add to syslog buffer if callback is registered
        if let Some(callback) = *SYSLOG_CALLBACK.lock() {
            callback(record.level(), &record.args().to_string());
        }
        
        // Check console log level - only print to console if message level is high enough
        // (lower numeric values = higher priority)
        let message_level = log_level_to_console_level(record.level());
        let console_level = *CONSOLE_LOG_LEVEL.lock();
        
        // Only output to console if message priority is high enough (level <= console_level)
        if message_level <= console_level {
            print_logs(record, timestamp);
        }
    }

    fn flush(&self) {}
}

#[cfg(feature = "log_color")]
fn print_logs(record: &Record, timestamp: f64) {
    use owo_colors::Style;

    let timestamp_style = Style::new().green();
    let record_style = Style::new().default_color();
    let level_style = match record.level() {
        log::Level::Error => Style::new().red(),
        log::Level::Warn => Style::new().bright_yellow(),
        log::Level::Info => Style::new().blue(),
        log::Level::Debug => Style::new().bright_green(),
        log::Level::Trace => Style::new().bright_black(),
    };

    super::_print(format_args!(
        "{} {:<5}: {}\n",
        timestamp_style.style(format_args!("[{:>10.3}]", timestamp)),
        level_style.style(record.level()),
        record_style.style(record.args())
    ));
}

#[cfg(not(feature = "log_color"))]
fn print_logs(record: &Record, timestamp: f64) {
    super::_print(format_args!(
        "{} {:<5}: {}\n",
        format_args!("[{:>10.3}]", timestamp),
        record.level(),
        record.args()
    ));
}

pub(super) fn init() {
    ostd::logger::inject_logger(&LOGGER);
}
