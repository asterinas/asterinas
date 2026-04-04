// SPDX-License-Identifier: MPL-2.0

use core::time::Duration;

use ostd::{
    log::{Level, Record},
    timer::Jiffies,
};

/// The logger used for Asterinas.
struct AsterLogger;

static LOGGER: AsterLogger = AsterLogger;

impl ostd::log::Log for AsterLogger {
    fn log(&self, record: &Record) {
        let timestamp = Jiffies::elapsed().as_duration();
        print_logs(record, &timestamp);
    }
}

#[cfg(feature = "log_color")]
fn print_logs(record: &Record, timestamp: &Duration) {
    use owo_colors::Style;

    let secs = timestamp.as_secs();
    let millis = timestamp.subsec_millis();

    let timestamp_style = Style::new().green();
    let record_style = Style::new().default_color();
    let level_style = match record.level() {
        Level::Error => Style::new().red(),
        Level::Warning => Style::new().bright_yellow(),
        Level::Info => Style::new().blue(),
        Level::Debug => Style::new().bright_green(),
        Level::Notice => Style::new().cyan(),
        Level::Emerg | Level::Alert | Level::Crit => Style::new().red().bold(),
    };

    super::_print(format_args!(
        "{} {:<6}: {}{}\n",
        timestamp_style.style(format_args!("[{:>6}.{:03}]", secs, millis)),
        level_style.style(record.level()),
        record_style.style(record.prefix()),
        record_style.style(record.args())
    ));
}

#[cfg(not(feature = "log_color"))]
fn print_logs(record: &Record, timestamp: &Duration) {
    let secs = timestamp.as_secs();
    let millis = timestamp.subsec_millis();

    super::_print(format_args!(
        "[{:>6}.{:03}] {:<6}: {}{}\n",
        secs,
        millis,
        record.level(),
        record.prefix(),
        record.args()
    ));
}

pub(super) fn init() {
    ostd::log::inject_logger(&LOGGER);
}
