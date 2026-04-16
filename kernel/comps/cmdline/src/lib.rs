// SPDX-License-Identifier: MPL-2.0

//! Kernel command-line interface for Asterinas.
//!
//! This crate provides a macro-based registration model to declare kernel
//! parameters which are collected at compile time and parsed during early boot.
//!
//! Registration model
//! - Macros: `define_kv_param!`, `define_repeatable_kv_param!`,
//!   `define_flag_param!` are used by components to register handlers/storage
//!   for parameters. These macros submit `KernelParam` entries into the
//!   `inventory` registry which the `dispatch` module consumes at boot.
//!
//! Key traits
//! - `ParseParamValue`, `ParseRepeatableParamValue`, `ParseFlag`: parsing
//!   traits that convert raw `&str` tokens into typed values.
//! - `ParamStorage`: storage abstraction used by components to receive parsed
//!   values (examples: `spin::Once<T>`, atomic types, `Once<Vec<T>>`).
//!
//! How it works
//! - Each registration macro submits a `KernelParam` (name + setup function)
//!   into the `inventory` registry.
//! - At early boot the `dispatch` module builds a lookup table from the
//!   registry, tokenizes the kernel command line, groups recognized
//!   occurrences and calls the corresponding setup functions. Unrecognized
//!   tokens are forwarded to the init process as `argv` (bare tokens) or
//!   `envp` (`key=value`).
//!
//! Relationship to components
//! - This crate integrates with the component initialization system. The cmdline
//!   component registers an initialization step that parses kernel parameters
//!   during early boot. Any component that depends on `aster-cmdline` will be
//!   initialized after the cmdline component has run, so parameters registered
//!   via the macros are parsed and stored before the dependent component's
//!   `init_component` executes. This guarantees that other components can rely
//!   on kernel parameters being initialized when they start.
//!
//! Usage example
//! ```ignore
//! // storage for a single key=value parameter
//! static LOG_LEVEL: AtomicU8 = AtomicU8::new(DEFAULT_LOG_LEVEL);
//! define_kv_param!("log_level", LOG_LEVEL);
//!
//! // repeatable example
//! static CONSOLES: Once<Vec<String>> = Once::new();
//! define_repeatable_kv_param!("console", CONSOLES);
//! ```
#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

// Set this crate's log prefix for `ostd::log`.
macro_rules! __log_prefix {
    () => {
        "cmdline: "
    };
}

mod dispatch;
pub mod parse;
pub mod types;
mod unimplemented;

pub use dispatch::INIT_PROC_ARGS;
#[doc(hidden)]
pub use dispatch::KernelParam;
#[doc(hidden)]
pub use inventory::submit;
#[doc(hidden)]
pub use unimplemented::setup_unimplemented;

/// Defines a **single-value** `key=value` kernel command-line parameter.
///
/// During command-line parsing, all occurrences of `$name` are grouped and then
/// handled with **last-wins** semantics.
///
/// # Arguments
///
/// - `$name`: Parameter name (e.g. `"log_level"`).
/// - `$storage`: Storage location for the parsed value. Its type must implement
///   [`crate::parse::ParamStorage`].
///
/// # Parsing
///
/// The stored value type `S::Value` must implement [`crate::parse::ParseParamValue`].
/// By default, any type implementing [`core::str::FromStr`] automatically implements
/// [`crate::parse::ParseParamValue`].
///
/// # Examples
///
/// ```ignore
/// static LOG_LEVEL: AtomicU8 = AtomicU8::new(0);
/// define_kv_param!("log_level", LOG_LEVEL);
/// ```
#[macro_export]
macro_rules! define_kv_param {
    ($name:expr, $storage:expr) => {
        $crate::__define_param!(@late, $name, $storage, $crate::parse::setup_kv_param);
    };
}

/// Defines an **early** `key=value` kernel command-line parameter.
///
/// Almost same as [`define_kv_param!`], but it is processed earlier in the boot sequence.
#[macro_export]
macro_rules! define_kv_param_early {
    ($name:expr, $storage:expr) => {
        $crate::__define_param!(@early, $name, $storage, $crate::parse::setup_kv_param);
    };
}

/// Defines a **repeatable** `key=value` kernel command-line parameter.
///
/// Unlike [`define_kv_param!`], this parameter may appear multiple times; the
/// framework collects **all** values and passes them to the parser in one shot.
///
/// # Arguments
///
/// - `$name`: Parameter name (e.g. `"console"`).
/// - `$storage`: Storage location for the parsed value. Its type must implement
///   [`crate::parse::ParamStorage`].
///
/// # Parsing
///
/// The stored value type `S::Value` must implement
/// [`crate::parse::ParseRepeatableParamValue`]. This crate provides a default
/// implementation for `Vec<T>` where `T: FromStr`.
///
/// # Examples
///
/// ```ignore
/// static CONSOLES: Once<Vec<String>> = Once::new();
/// define_repeatable_kv_param!("console", CONSOLES);
/// ```
#[macro_export]
macro_rules! define_repeatable_kv_param {
    ($name:expr, $storage:expr) => {
        $crate::__define_param!(@late, $name, $storage, $crate::parse::setup_repeatable_kv_param);
    };
}

/// Defines an **early repeatable** `key=value` kernel command-line parameter.
///
/// Almost same as [`define_repeatable_kv_param!`], but it is processed earlier in the boot sequence.
#[macro_export]
macro_rules! define_repeatable_kv_param_early {
    ($name:expr, $storage:expr) => {
        $crate::__define_param!(@early, $name, $storage, $crate::parse::setup_repeatable_kv_param);
    };
}

/// Defines a **flag** kernel command-line parameter.
///
/// A flag may appear as a bare token (e.g. `debug`) or with an optional value
/// (e.g. `debug=1`). If the flag appears multiple times, the framework uses
/// **last-wins** semantics.
///
/// # Arguments
/// - `$name`: Flag name (e.g. `"debug"`).
/// - `$storage`: Storage location for the parsed value. Its type must implement
///   [`crate::parse::ParamStorage`].
///
/// # Parsing
///
/// The stored value type `S::Value` must implement [`crate::parse::ParseFlag`].
/// This crate provides a default `bool` implementation:
/// - `flag` / `flag=1` / `flag=on|yes|true` => `true`
/// - `flag=0` / `flag=off|no|false` => `false`
///
/// # Examples
///
/// ```ignore
/// static DEBUG: AtomicBool = AtomicBool::new(false);
/// define_flag_param!("debug", DEBUG);
/// ```
#[macro_export]
macro_rules! define_flag_param {
    ($name:expr, $storage:expr) => {
        $crate::__define_param!(@late, $name, $storage, $crate::parse::setup_flag_param);
    };
}

/// Defines an **early flag** kernel command-line parameter.
///
/// Almost same as [`define_flag_param!`], but it is processed earlier in the boot sequence.
#[macro_export]
macro_rules! define_flag_param_early {
    ($name:expr, $storage:expr) => {
        $crate::__define_param!(@early, $name, $storage, $crate::parse::setup_flag_param);
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __define_param {
    (@late, $name:expr, $storage:expr, $setup:path) => {
        const _: () = {
            fn __kparam_setup(occurrences: &[Option<&str>]) {
                $setup(&$storage, occurrences, $name);
            }
            $crate::submit! {
                $crate::KernelParam::new($name, __kparam_setup, false)
            }
        };
    };

    (@early, $name:expr, $storage:expr, $setup:path) => {
        const _: () = {
            fn __kparam_setup(occurrences: &[Option<&str>]) {
                $setup(&$storage, occurrences, $name);
            }
            $crate::submit! {
                $crate::KernelParam::new($name, __kparam_setup, true)
            }
        };
    };
}
