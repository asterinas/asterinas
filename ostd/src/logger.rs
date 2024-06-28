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
