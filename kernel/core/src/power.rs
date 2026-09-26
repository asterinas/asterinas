// SPDX-License-Identifier: MPL-2.0

//! System power provider registration and policy support.
//!
//! Each architecture supplies the restart and poweroff policies injected into OSTD. A policy may
//! invoke OSTD's architecture-specific mechanism and registered kernel providers in the order
//! required by that platform.
//!
//! A [`Priority::Normal`] provider is a regular mechanism expected to operate on the complete
//! system. A [`Priority::Fallback`] provider is a legacy or otherwise last-resort mechanism that
//! is attempted only after all normal providers return. The current provider set requires only
//! this distinction. Providers in the same class have no defined order.
//!
//! The x86 poweroff policy and both RISC-V and AArch64 policies try the OSTD mechanism before
//! registered providers. The x86 restart and LoongArch policies omit their corresponding OSTD
//! mechanisms because those mechanisms are currently no-ops.
//!
//! On x86, ACPI reset is [`Priority::Normal`] and i8042 reset is [`Priority::Fallback`]. On
//! LoongArch, syscon-poweroff is [`Priority::Normal`]. No other architecture and operation
//! combination currently registers a kernel provider.
//!
//! The policies are installed before architecture initialization completes, so providers may be
//! invoked before their runtime state is available. Providers must return when that state is
//! unavailable and must not panic.

use ostd::power::{self, ExitCode};

/// The invocation priority of a power handler.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Priority {
    /// A regular mechanism expected to operate on the complete system.
    Normal,
    /// A legacy or last-resort mechanism attempted after all normal handlers.
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
    power::inject_restart_handler(crate::arch::power::restart_policy);
    power::inject_poweroff_handler(crate::arch::power::poweroff_policy);
}

pub(crate) fn invoke_restart_providers(code: ExitCode) {
    invoke_restart_handlers(code, Priority::Normal);
    invoke_restart_handlers(code, Priority::Fallback);
}

pub(crate) fn invoke_poweroff_providers(code: ExitCode) {
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
