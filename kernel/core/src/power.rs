// SPDX-License-Identifier: MPL-2.0

//! System restart and poweroff handlers.
//!
//! This module allows restart and poweroff handlers to be registered at link time.
//!
//! # Handler Registration and Priorities
//!
//! During kernel initialization, this module injects one restart dispatcher and one poweroff
//! dispatcher into OSTD. Restart and poweroff handlers can be registered with
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
pub struct Priority(u8);

impl Priority {
    /// A priority for handlers of last resort.
    pub const LOW: Self = Self(0);

    /// A priority for normal handlers.
    pub const DEFAULT: Self = Self(128);

    /// A priority for handlers that should run before default-priority handlers.
    pub const HIGH: Self = Self(192);

    /// A priority for handlers that invoke platform firmware.
    pub const FIRMWARE: Self = Self(224);

    /// Creates a priority from the given integer value.
    ///
    /// Handlers with higher values are invoked first.
    pub const fn new(value: u8) -> Self {
        Self(value)
    }
}

trait RegisteredHandler: inventory::Collect {
    fn callback(&self) -> fn(ExitCode);

    fn priority(&self) -> Priority;
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

impl RegisteredHandler for RestartHandler {
    fn callback(&self) -> fn(ExitCode) {
        self.callback
    }

    fn priority(&self) -> Priority {
        self.priority
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

impl RegisteredHandler for PoweroffHandler {
    fn callback(&self) -> fn(ExitCode) {
        self.callback
    }

    fn priority(&self) -> Priority {
        self.priority
    }
}

inventory::collect!(PoweroffHandler);

#[doc(hidden)]
pub use inventory::submit;

/// Registers a restart handler.
///
/// The first argument must be a function path with the signature
/// `fn(ostd::power::ExitCode)`. The second argument must be a
/// [`Priority`].
#[macro_export]
macro_rules! register_restart_handler {
    ($handler:path, $priority:expr) => {
        $crate::power::submit! {
            $crate::power::RestartHandler::new($handler, $priority)
        }
    };
}

/// Registers a poweroff handler.
///
/// The first argument must be a function path with the signature
/// `fn(ostd::power::ExitCode)`. The second argument must be a
/// [`Priority`].
#[macro_export]
macro_rules! register_poweroff_handler {
    ($handler:path, $priority:expr) => {
        $crate::power::submit! {
            $crate::power::PoweroffHandler::new($handler, $priority)
        }
    };
}

pub(crate) fn init() {
    power::inject_restart_handler(invoke_handlers::<RestartHandler>);
    power::inject_poweroff_handler(invoke_handlers::<PoweroffHandler>);
}

fn invoke_handlers<H: RegisteredHandler>(code: ExitCode) {
    let mut upper_bound = None;
    loop {
        let next_priority = inventory::iter::<H>
            .into_iter()
            .filter(|handler| upper_bound.is_none_or(|bound| handler.priority() < bound))
            .map(RegisteredHandler::priority)
            .max();
        let Some(priority) = next_priority else {
            break;
        };

        for handler in inventory::iter::<H> {
            if handler.priority() == priority {
                (handler.callback())(code);
            }
        }

        upper_bound = Some(priority);
    }
}
