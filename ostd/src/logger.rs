// SPDX-License-Identifier: MPL-2.0

//! Logging support.

use alloc::format;

use log::{LevelFilter, Metadata, Record};

use crate::{
    arch::timer::Jiffies,
    boot::{kcmdline::ModuleArg, kernel_cmdline},
    early_println,
};

const LOGGER: Logger = Logger {};

struct Logger {}

impl log::Log for Logger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }

        let timestamp = format!("[{:>10?}]", Jiffies::elapsed().as_duration().as_secs_f64());
        let level = format!("{:<5}", record.level());
        let record_str = format!("{}", record.args());

        #[cfg(feature = "log_color")]
        let (timestamp, level, record_str) = {
            use alloc::string::ToString;

            use owo_colors::OwoColorize;

            let timestamp = timestamp.green();
            let level = match record.level() {
                log::Level::Error => level.red().to_string(),
                log::Level::Warn => level.bright_yellow().to_string(),
                log::Level::Info => level.blue().to_string(),
                log::Level::Debug => level.bright_green().to_string(),
                log::Level::Trace => level.bright_black().to_string(),
            };
            let record_str = record_str.default_color();
            (timestamp, level, record_str)
        };

        early_println!("{} {}: {}", timestamp, level, record_str);
    }

    fn flush(&self) {}
}

/// Initialize the logger. Users should avoid using the log macros before this function is called.
pub(crate) fn init() {
    let level = get_log_level().unwrap_or(LevelFilter::Off);

    log::set_max_level(level);
    log::set_logger(&LOGGER).unwrap();
}

fn get_log_level() -> Option<LevelFilter> {
    let module_args = kernel_cmdline().get_module_args("ostd")?;

    let arg = module_args.iter().find(|arg| match arg {
        ModuleArg::Arg(_) => false,
        ModuleArg::KeyVal(name, _) => name.as_bytes() == "log_level".as_bytes(),
    })?;

    let ModuleArg::KeyVal(_, value) = arg else {
        unreachable!()
    };
    let value = value.as_c_str().to_str().unwrap_or("off");
    Some(match value {
        "error" => LevelFilter::Error,
        "warn" => LevelFilter::Warn,
        "info" => LevelFilter::Info,
        "debug" => LevelFilter::Debug,
        "trace" => LevelFilter::Trace,
        // Otherwise, OFF
        _ => LevelFilter::Off,
    })
}
