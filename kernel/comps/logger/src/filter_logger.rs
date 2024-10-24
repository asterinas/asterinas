// SPDX-License-Identifier: MPL-2.0

use alloc::vec::Vec;

use log::{LevelFilter, Metadata, Record};
use ostd::timer::Jiffies;
use spin::Once;

use crate::filter::{get_filter_list, FilterEntry};

/// A logger with filtering capabilities.
///
/// This logger turns off and produces no output when the max log level is set to "off".
/// In other cases, in addition to the basic logging capabilities, it also has an additional
/// filtering function. This function can utilize the assistance of the `filter` module to
/// change the max log level for specific modules, files, or specified code, allowing users
/// to more flexibly control the output content.
///
/// This logger incurs no additional performance overhead when the level filter is set to "off".
/// In other modes, due to the presence of additional filtering logic, there is a very small
/// performance overhead.
pub(super) struct FilterLogger {
    filter: Vec<FilterEntry>,
    global_max_level: LevelFilter,
    filter_max_level: LevelFilter,
}

impl FilterLogger {
    fn new() -> Self {
        let (filter, filter_max_level) = get_filter_list();

        Self {
            filter,
            global_max_level: log::max_level(),
            filter_max_level,
        }
    }

    fn filter(&self, record: &Record) -> bool {
        for entry in &self.filter {
            match entry {
                FilterEntry::Module { module, log_level } => {
                    if record
                        .module_path()
                        .is_some_and(|path| path.contains(module))
                    {
                        return record.metadata().level() <= *log_level;
                    }
                }
                FilterEntry::File { file, log_level } => {
                    if record.file().is_some_and(|path| path == file) {
                        return record.metadata().level() <= *log_level;
                    }
                }
                FilterEntry::Line {
                    file,
                    line_number,
                    log_level,
                } => {
                    if record.line().is_some_and(|line| line == *line_number)
                        && record.file().is_some_and(|path| path == file)
                    {
                        return record.metadata().level() <= *log_level;
                    }
                }
            }
        }

        record.metadata().level() <= self.global_max_level
    }

    pub(crate) fn set_global(&'static self) {
        ostd::logger::set_logger(self);
        if log::max_level() != LevelFilter::Off && log::max_level() < self.filter_max_level {
            // Relax the global `max_level` to allow for fine-grained control
            // over the output since users want to change the max log level of
            // some scopes to `self.filter_max_level`.
            log::set_max_level(self.filter_max_level);
        }
    }
}

pub(super) static LOGGER: Once<FilterLogger> = Once::new();

impl log::Log for FilterLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record) {
        if !self.filter(record) {
            return;
        }

        let timestamp = Jiffies::elapsed().as_duration().as_secs_f64();
        let level = record.level();

        // Use a global lock to prevent interleaving of log messages.
        use ostd::sync::SpinLock;
        static RECORD_LOCK: SpinLock<()> = SpinLock::new(());
        let _lock = RECORD_LOCK.disable_irq().lock();

        cfg_if::cfg_if! {
            if #[cfg(feature = "log_color")]{
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

                super::_print(
                    format_args!("{} {:<5}: {}\n",
                    timestamp_style.style(format_args!("[{:>10.3}]", timestamp)),
                    level_style.style(level),
                    record_style.style(record.args()))
                );
            }else{
                super::_print(
                    format_args!("{} {:<5}: {}\n",
                    format_args!("[{:>10.3}]", timestamp),
                    level,
                    record.args())
                );
            }
        }
    }

    fn flush(&self) {}
}

pub(super) fn init() {
    let logger = FilterLogger::new();
    LOGGER.call_once(|| logger);
    FilterLogger::set_global(LOGGER.get().unwrap());
}
