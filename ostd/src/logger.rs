// SPDX-License-Identifier: MPL-2.0

//! Logging support.

use log::{LevelFilter, Metadata, Record};

use crate::{
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
        if self.enabled(record.metadata()) {
            early_println!("[{}]: {}", record.level(), record.args());
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
