// SPDX-License-Identifier: MPL-2.0

//! Bridge that forwards `log` crate messages to the OSTD logger.

use super::{Level, LevelFilter, Record, logger::__logger};

/// Syncs the `log` crate's max level with the given OSTD level filter.
pub(super) fn sync_log_crate_max_level(filter: LevelFilter) {
    ::log::set_max_level(map_level_filter(filter));
}

pub(super) struct LogCrateBridge;

impl ::log::Log for LogCrateBridge {
    fn enabled(&self, _metadata: &::log::Metadata) -> bool {
        // The `log` crate's macros already check static and runtime filters
        // before calling this method,
        // and we sync the max level via `sync_log_crate_max_level`.
        // No extra filtering is needed here.
        true
    }

    fn log(&self, record: &::log::Record) {
        if let Some(logger) = __logger() {
            let level = map_log_level(record.level());
            logger.log(&Record::new(
                level,
                *record.args(),
                record.module_path_static().unwrap_or(""),
                record.file_static().unwrap_or(""),
                record.line().unwrap_or(0),
            ));
        }
    }

    fn flush(&self) {}
}

fn map_log_level(level: ::log::Level) -> Level {
    match level {
        ::log::Level::Error => Level::Error,
        ::log::Level::Warn => Level::Warning,
        ::log::Level::Info => Level::Info,
        ::log::Level::Debug => Level::Debug,
        ::log::Level::Trace => Level::Debug,
    }
}

fn map_level_filter(filter: LevelFilter) -> ::log::LevelFilter {
    match filter {
        LevelFilter::Off => ::log::LevelFilter::Off,
        LevelFilter::Emerg | LevelFilter::Alert | LevelFilter::Crit | LevelFilter::Error => {
            ::log::LevelFilter::Error
        }
        LevelFilter::Warning => ::log::LevelFilter::Warn,
        LevelFilter::Notice | LevelFilter::Info => ::log::LevelFilter::Info,
        LevelFilter::Debug => ::log::LevelFilter::Trace,
    }
}

#[cfg(ktest)]
mod test {
    use super::*;
    use crate::prelude::*;

    #[ktest]
    fn bridge_map_log_level() {
        assert_eq!(map_log_level(::log::Level::Error), Level::Error);
        assert_eq!(map_log_level(::log::Level::Warn), Level::Warning);
        assert_eq!(map_log_level(::log::Level::Info), Level::Info);
        assert_eq!(map_log_level(::log::Level::Debug), Level::Debug);
        assert_eq!(map_log_level(::log::Level::Trace), Level::Debug);
    }

    #[ktest]
    fn bridge_map_level_filter() {
        assert_eq!(map_level_filter(LevelFilter::Off), ::log::LevelFilter::Off);
        assert_eq!(
            map_level_filter(LevelFilter::Emerg),
            ::log::LevelFilter::Error
        );
        assert_eq!(
            map_level_filter(LevelFilter::Alert),
            ::log::LevelFilter::Error
        );
        assert_eq!(
            map_level_filter(LevelFilter::Crit),
            ::log::LevelFilter::Error
        );
        assert_eq!(
            map_level_filter(LevelFilter::Error),
            ::log::LevelFilter::Error
        );
        assert_eq!(
            map_level_filter(LevelFilter::Warning),
            ::log::LevelFilter::Warn
        );
        assert_eq!(
            map_level_filter(LevelFilter::Notice),
            ::log::LevelFilter::Info
        );
        assert_eq!(
            map_level_filter(LevelFilter::Info),
            ::log::LevelFilter::Info
        );
        assert_eq!(
            map_level_filter(LevelFilter::Debug),
            ::log::LevelFilter::Trace
        );
    }
}
