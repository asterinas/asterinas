// SPDX-License-Identifier: MPL-2.0

use aster_softirq::{softirq_id::LOG_SOFTIRQ_ID, SoftIrqLine};
use ostd::logger::{consume_buffer_with, LogBuffer, LogRecord};

fn log_handler() {
    let f = |buffer: &mut LogBuffer| {
        while let Some(record) = buffer.read_log() {
            print_logs(&record);
        }
    };

    consume_buffer_with(f);
}

#[cfg(feature = "log_color")]
fn print_logs(record: &LogRecord<'_>) {
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
        timestamp_style.style(format_args!("[{:>10.3}]", record.timestamp().as_secs_f64())),
        level_style.style(record.level()),
        record_style.style(record.context())
    ));
}

#[cfg(not(feature = "log_color"))]
fn print_logs(record: &Record) {
    super::_print(format_args!(
        "{} {:<5}: {}\n",
        format_args!("[{:>10.3}]", record.timestamp().as_secs_f64()),
        record.level(),
        record.context()
    ));
}

pub(super) fn init() {
    SoftIrqLine::get(LOG_SOFTIRQ_ID).enable(log_handler);

    ostd::timer::register_callback(|| {
        SoftIrqLine::get(LOG_SOFTIRQ_ID).raise();
    });
}
