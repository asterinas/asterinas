// SPDX-License-Identifier: MPL-2.0

//! System power provider orchestration.
//!
//! Architecture mechanisms are attempted first, followed by normal and fallback providers.
//! Ordering between providers with the same priority is unspecified.
//! Providers must tolerate early calls, return after failed attempts, and not panic.

use ostd::power::{self, ExitCode};

/// The invocation priority of a power handler.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Priority {
    /// A regular power handler.
    Normal,
    /// A handler attempted after all regular handlers.
    Fallback,
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
    power::inject_restart_handler(try_restart);
    power::inject_poweroff_handler(try_poweroff);
}

fn try_restart(code: ExitCode) {
    ostd::arch::power::try_restart(code);
    invoke_restart_handlers(code, Priority::Normal);
    invoke_restart_handlers(code, Priority::Fallback);
}

fn try_poweroff(code: ExitCode) {
    ostd::arch::power::try_poweroff(code);
    invoke_poweroff_handlers(code, Priority::Normal);
    invoke_poweroff_handlers(code, Priority::Fallback);
}

fn invoke_restart_handlers(code: ExitCode, priority: Priority) {
    for handler in inventory::iter::<RestartHandler> {
        if handler.priority == priority {
            (handler.callback)(code);
        }
    }
}

fn invoke_poweroff_handlers(code: ExitCode, priority: Priority) {
    for handler in inventory::iter::<PoweroffHandler> {
        if handler.priority == priority {
            (handler.callback)(code);
        }
    }
}
