// SPDX-License-Identifier: MPL-2.0

use alloc::string::ToString;
use log::{Metadata, Record};
use ostd::{sync::SpinLock, timer::Jiffies};

/// Callback function type for syslog integration
pub type SyslogCallback = fn(level: log::Level, message: &str);

/// Global syslog callback protected by spinlock
static SYSLOG_CALLBACK: SpinLock<Option<SyslogCallback>> = SpinLock::new(None);

/// Register a callback for syslog integration
pub fn register_syslog_callback(callback: SyslogCallback) {
    *SYSLOG_CALLBACK.lock() = Some(callback);
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
        
        print_logs(record, timestamp);
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
