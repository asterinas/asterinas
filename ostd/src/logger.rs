// SPDX-License-Identifier: MPL-2.0

//! Logging support.
//!
//! Currently the logger prints the logs to the console.
//!
//! This module guarantees _atomicity_ under concurrency: messages are always
//! printed in their entirety without being mixed with messages generated
//! concurrently on other cores.
//!
//! IRQs are disabled while printing. So do not print long log messages.

use log::{LevelFilter, Metadata, Record};

use crate::{
    boot::{kcmdline::ModuleArg, kernel_cmdline},
    timer::Jiffies,
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

        // Use a global lock to prevent interleaving of log messages.
        use crate::sync::SpinLock;
        static RECORD_LOCK: SpinLock<()> = SpinLock::new(());

        RECORD_LOCK.disable_irq().lock_with(|_| {
            self.do_log(record);
        });
    }

    fn flush(&self) {}
}

impl Logger {
    fn do_log(&self, record: &Record) {
        let timestamp = Jiffies::elapsed().as_duration().as_secs_f64();
        let level = record.level();

        cfg_if::cfg_if! {
            if #[cfg(feature = "log_color")] {
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

                crate::console::early_print(
                    format_args!("{} {:<5}: {}\n",
                    timestamp_style.style(format_args!("[{:>10.3}]", timestamp)),
                    level_style.style(level),
                    record_style.style(record.args()))
                );
            } else {
                crate::console::early_print(
                    format_args!("{} {:<5}: {}\n",
                    format_args!("[{:>10.3}]", timestamp),
                    level,
                    record.args())
                );
            }
        }
    }
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
