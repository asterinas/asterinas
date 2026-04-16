// SPDX-License-Identifier: MPL-2.0

//! Kernel logging API.
//!
//! This module provides the logging facade for OSTD and all OSTD-based crates.
//! It uses eight log levels matching the severity levels described in `syslog(2)`.
//!
//! # Setup: defining `__log_prefix`
//!
//! Every crate that uses the logging macros
//! must define a `__log_prefix` macro at its crate root (`lib.rs`),
//! before any `mod` declarations.
//! This prefix is prepended to every log message from the crate.
//!
//! ```rust,ignore
//! // Set this crate's log prefix for `ostd::log`.
//! macro_rules! __log_prefix {
//!     () => {
//!         "virtio: "
//!     };
//! }
//!
//! mod device;   // all modules inherit the "virtio: " prefix
//! ```
//!
//! It is recommended is to follow Linux's convention for log prefixes,
//! which uses the lowercase module name, followed by `: `.
//! For example: `"virtio: "`, `"pci: "`, `"uart: "`.
//!
//! # Quick start
//!
//! After defining `__log_prefix`, import the macros and use them:
//!
//! ```rust,ignore
//! use ostd::prelude::*;
//!
//! info!("boot complete");
//! warn!("feature X is not supported");
//! ```
//!
//! # Log levels
//!
//! Eight severity levels are provided, matching `syslog(2)`:
//!
//! | Level   | Value | Meaning                      |
//! |---------|-------|------------------------------|
//! | Emerg   | 0     | System is unusable           |
//! | Alert   | 1     | Action must be taken         |
//! | Crit    | 2     | Critical conditions          |
//! | Error   | 3     | Error conditions             |
//! | Warning | 4     | Warning conditions           |
//! | Notice  | 5     | Normal but significant       |
//! | Info    | 6     | Informational                |
//! | Debug   | 7     | Debug-level messages         |
//!
//! ```rust,ignore
//! use ostd::prelude::*;
//!
//! emerg!("system is going down");
//! alert!("action required immediately");
//! crit!("critical failure in subsystem");
//! error!("operation failed: {}", err);
//! warn!("deprecated feature used");
//! notice!("configuration change applied");
//! info!("boot complete");
//! debug!("variable x = {:?}", x);
//! ```
//!
//! # `log` crate bridge
//!
//! A bridge forwards messages from third-party crates
//! that use the [`log`](https://docs.rs/log) crate (e.g., `smoltcp`)
//! to the OSTD logger.
//! First-party code should use OSTD's macros directly.
//!
//! # Per-module prefix overrides
//!
//! A subsystem module can override the crate-level prefix
//! by defining its own `__log_prefix` at the top of its `mod.rs`,
//! before any `mod child;` declarations.
//! Child modules inherit the override via textual scoping:
//!
//! ```rust,ignore
//! // Set this module's log prefix for `ostd::log`.
//! macro_rules! __log_prefix {
//!     () => {
//!         "iommu: "
//!     };
//! }
//!
//! mod fault;      // inherits "iommu: " prefix
//! mod registers;  // inherits "iommu: " prefix
//! ```
//!
//! # Limitations
//!
//! ## No attributes on `__log_prefix` definitions
//!
//! Do not put `#[rustfmt::skip]` or any other attribute
//! on `__log_prefix` definitions.
//! Rust treats attributed `macro_rules!` items as "macro-expanded,"
//! which triggers E0659 ambiguity with definitions at other scopes.
//! See the design doc in `log/macros.rs` for the full explanation.
//!
//! # Backend
//!
//! An OSTD-based kernel can register a custom [`Log`] implementation via [`inject_logger`].
//! Before a backend is registered, messages are printed through the early-boot console.

mod bridge;
mod level;
mod logger;
mod macros;

use self::bridge::LogCrateBridge;
pub use self::{
    level::{Level, LevelFilter},
    logger::{
        __write_log_record, Log, Record, STATIC_MAX_LEVEL, inject_logger, max_level, set_max_level,
    },
};

/// Initializes the OSTD logging subsystem.
///
/// Parses the `ostd.log_level` kernel command line parameter, sets the
/// runtime max level, and registers the `log` crate bridge.
pub(crate) fn init() {
    let filter = parse_log_level_from_cmdline().unwrap_or(LevelFilter::Off);
    set_max_level(filter);

    static BRIDGE: LogCrateBridge = LogCrateBridge;
    let _ = ::log::set_logger(&BRIDGE);
}

fn parse_log_level_from_cmdline() -> Option<LevelFilter> {
    let kcmdline = crate::boot::EARLY_INFO.get()?.kernel_cmdline;

    let value = kcmdline
        .split(' ')
        .find(|arg| arg.starts_with("ostd.log_level="))
        .map(|arg| arg.split('=').next_back().unwrap_or_default())?;

    parse_level_str(value)
}

/// Parses a log level string into a [`LevelFilter`].
///
/// Accepts: `"off"`, `"emerg"`, `"alert"`, `"crit"`, `"error"`,
/// `"warn"` / `"warning"`, `"notice"`, `"info"`, `"debug"`.
/// Returns `None` for unrecognized strings.
fn parse_level_str(s: &str) -> Option<LevelFilter> {
    match s {
        "off" => Some(LevelFilter::Off),
        "emerg" => Some(LevelFilter::Emerg),
        "alert" => Some(LevelFilter::Alert),
        "crit" => Some(LevelFilter::Crit),
        "error" => Some(LevelFilter::Error),
        "warn" | "warning" => Some(LevelFilter::Warning),
        "notice" => Some(LevelFilter::Notice),
        "info" => Some(LevelFilter::Info),
        "debug" => Some(LevelFilter::Debug),
        _ => None,
    }
}

#[cfg(ktest)]
mod test {
    use super::*;
    use crate::prelude::*;

    #[ktest]
    fn parse_level_str_valid() {
        assert_eq!(parse_level_str("off"), Some(LevelFilter::Off));
        assert_eq!(parse_level_str("emerg"), Some(LevelFilter::Emerg));
        assert_eq!(parse_level_str("alert"), Some(LevelFilter::Alert));
        assert_eq!(parse_level_str("crit"), Some(LevelFilter::Crit));
        assert_eq!(parse_level_str("error"), Some(LevelFilter::Error));
        assert_eq!(parse_level_str("warn"), Some(LevelFilter::Warning));
        assert_eq!(parse_level_str("warning"), Some(LevelFilter::Warning));
        assert_eq!(parse_level_str("notice"), Some(LevelFilter::Notice));
        assert_eq!(parse_level_str("info"), Some(LevelFilter::Info));
        assert_eq!(parse_level_str("debug"), Some(LevelFilter::Debug));
    }

    #[ktest]
    fn parse_level_str_invalid() {
        assert_eq!(parse_level_str("trace"), None);
        assert_eq!(parse_level_str(""), None);
        assert_eq!(parse_level_str("INFO"), None);
        assert_eq!(parse_level_str("garbage"), None);
    }
}
