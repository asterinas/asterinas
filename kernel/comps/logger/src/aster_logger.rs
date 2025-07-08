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
        let millis = Jiffies::elapsed().as_duration().as_millis();
        let secs = (millis / 1000) as u64;
        let millis = (millis % 1000) as u64;
        print_logs(record, secs, millis);
    }

    fn flush(&self) {}
}

#[cfg(feature = "log_color")]
fn print_logs(record: &Record, secs: u64, millis: u64) {
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
        timestamp_style.style(format_args!("[{:>6}.{:03}]", secs, millis)),
        level_style.style(record.level()),
        record_style.style(record.args())
    ));
}

#[cfg(not(feature = "log_color"))]
fn print_logs(record: &Record, secs: u64, millis: u64) {
    super::_print(format_args!(
        "{} {:<5}: {}\n",
        format_args!("[{:>6}.{:03}]", secs, millis),
        record.level(),
        record.args()
    ));
}

pub(super) fn init() {
    ostd::logger::inject_logger(&LOGGER);
}
