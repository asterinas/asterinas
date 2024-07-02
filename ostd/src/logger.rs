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
        #[cfg(feature = "log_color")]
        use alloc::string::ToString;

        #[cfg(feature = "log_color")]
        use owo_colors::OwoColorize;

        if self.enabled(record.metadata()) {
            let timestamp = format!("[{:>10?}]", Jiffies::elapsed().as_duration().as_secs_f64());
            let level = format!("{:<5}", record.level());
            let record_str = format!("{}", record.args());

            #[cfg(feature = "log_color")]
            let (timestamp, level, record_str) = {
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
    }

    fn flush(&self) {}
}

/// Initialize the logger. Users should avoid using the log macros before this function is called.
pub(crate) fn init() {
    let module_args = kernel_cmdline().get_module_args("ostd");
    let mut level = LevelFilter::Off;
    if let Some(module_args) = module_args {
        for arg in module_args.iter() {
            match arg {
                ModuleArg::Arg(_) => {}
                ModuleArg::KeyVal(name, value) => {
                    if name.as_bytes() == "log_level".as_bytes() {
                        let value = value.as_c_str().to_str().unwrap();
                        level = match value {
                            "error" => LevelFilter::Error,
                            "warn" => LevelFilter::Warn,
                            "info" => LevelFilter::Info,
                            "debug" => LevelFilter::Debug,
                            "trace" => LevelFilter::Trace,
                            // Otherwise, OFF
                            _ => LevelFilter::Off,
                        }
                    }
                }
            }
        }
    }
    log::set_max_level(level);
    log::set_logger(&LOGGER).unwrap();
}
