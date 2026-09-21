// SPDX-License-Identifier: MPL-2.0

//! System restart and poweroff handlers.
//!
//! This module allows kernel components to register restart and poweroff handlers at link time.
//!
//! # Handler Registration and Priorities
//!
//! During kernel initialization, this module injects one restart dispatcher and one poweroff
//! dispatcher into OSTD. Kernel components register zero or more handlers at link time with
//! [`register_restart_handler!`](crate::register_restart_handler) and
//! [`register_poweroff_handler!`](crate::register_poweroff_handler). The dispatchers invoke these
//! handlers when a restart or poweroff is requested.
//!
//! Each registered handler has an integer [`Priority`]. The named priorities provide standard
//! reference values; handlers may use intermediate values when finer ordering is required.
//!
//! # Invocation Order
//!
//! Registered handlers are tried in descending priority order. Handlers with the same priority
//! have no defined order. A handler that completes the requested restart or poweroff does not
//! return. If it returns, the next handler is tried.
//!
//! # Handler Requirements
//!
//! Handlers may be invoked before the state required by their mechanism is initialized. They must
//! return when that state is unavailable and must not panic.

use ostd::power::{self, ExitCode};

/// The invocation priority of a restart or poweroff handler.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Priority(i16);

impl Priority {
    /// A priority for handlers of last resort.
    pub const LOW: Self = Self(-128);

    /// A priority for normal handlers.
    pub const DEFAULT: Self = Self(0);

    /// A priority for handlers that should run before default-priority handlers.
    pub const HIGH: Self = Self(192);

    /// A priority for handlers that invoke platform firmware.
    pub const FIRMWARE: Self = Self(224);

    /// Creates a priority from the given integer value.
    ///
    /// Handlers with higher values are invoked first.
    pub const fn new(value: i16) -> Self {
        Self(value)
    }
}

/// A restart handler collected at link time.
#[doc(hidden)]
pub struct RestartHandler {
    callback: fn(ExitCode),
    priority: Priority,
}

impl RestartHandler {
    /// Creates a restart handler descriptor.
    #[doc(hidden)]
    pub const fn new(callback: fn(ExitCode), priority: Priority) -> Self {
        Self { callback, priority }
    }
}

inventory::collect!(RestartHandler);

/// A poweroff handler collected at link time.
#[doc(hidden)]
pub struct PoweroffHandler {
    callback: fn(ExitCode),
    priority: Priority,
}

impl PoweroffHandler {
    /// Creates a poweroff handler descriptor.
    #[doc(hidden)]
    pub const fn new(callback: fn(ExitCode), priority: Priority) -> Self {
        Self { callback, priority }
    }
}

inventory::collect!(PoweroffHandler);

#[doc(hidden)]
pub use inventory::submit;

/// Registers a restart handler.
#[macro_export]
macro_rules! register_restart_handler {
    ($handler:path, $priority:expr) => {
        $crate::power::submit! {
            $crate::power::RestartHandler::new($handler, $priority)
        }
    };
}

/// Registers a poweroff handler.
#[macro_export]
macro_rules! register_poweroff_handler {
    ($handler:path, $priority:expr) => {
        $crate::power::submit! {
            $crate::power::PoweroffHandler::new($handler, $priority)
        }
    };
}

pub(crate) fn init() {
    power::inject_restart_handler(restart_handler);
    power::inject_poweroff_handler(poweroff_handler);
}

fn restart_handler(code: ExitCode) {
    invoke_restart_handlers(code);
}

fn poweroff_handler(code: ExitCode) {
    invoke_poweroff_handlers(code);
}

fn invoke_restart_handlers(code: ExitCode) {
    let mut upper_bound = None;
    loop {
        let next_priority = inventory::iter::<RestartHandler>
            .into_iter()
            .filter(|handler| upper_bound.is_none_or(|bound| handler.priority < bound))
            .map(|handler| handler.priority)
            .max();
        let Some(priority) = next_priority else {
            break;
        };

        for handler in inventory::iter::<RestartHandler> {
            if handler.priority == priority {
                (handler.callback)(code);
            }
        }

        upper_bound = Some(priority);
    }
}

fn invoke_poweroff_handlers(code: ExitCode) {
    let mut upper_bound = None;
    loop {
        let next_priority = inventory::iter::<PoweroffHandler>
            .into_iter()
            .filter(|handler| upper_bound.is_none_or(|bound| handler.priority < bound))
            .map(|handler| handler.priority)
            .max();
        let Some(priority) = next_priority else {
            break;
        };

        for handler in inventory::iter::<PoweroffHandler> {
            if handler.priority == priority {
                (handler.callback)(code);
            }
        }

        upper_bound = Some(priority);
    }
}
