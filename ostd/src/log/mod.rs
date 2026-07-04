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
use crate::boot::EarlyCmdline;

/// Initializes the OSTD logging subsystem.
///
/// Sets the runtime max level from [`EarlyCmdline::log_level`] and
/// registers the `log` crate bridge.
pub(crate) fn init(early_cmdline: &EarlyCmdline) {
    set_max_level(early_cmdline.log_level);
    static BRIDGE: LogCrateBridge = LogCrateBridge;
    let _ = ::log::set_logger(&BRIDGE);
}
