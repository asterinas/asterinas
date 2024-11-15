// SPDX-License-Identifier: MPL-2.0

use log::{Metadata, Record};
use ostd::timer::Jiffies;

/// The logger used for Asterinas.
struct AsterLogger;

static LOGGER: AsterLogger = AsterLogger;

impl log::Log for AsterLogger {
    fn enabled(&self, _metadata: &Metadata) -> bool {
        true
    }

    fn log(&self, record: &Record) {
        let timestamp = Jiffies::elapsed().as_duration().as_secs_f64();

        // Use a global lock to prevent interleaving of log messages.
        use ostd::sync::SpinLock;
        static RECORD_LOCK: SpinLock<()> = SpinLock::new(());
        let _lock = RECORD_LOCK.disable_irq().lock();

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
