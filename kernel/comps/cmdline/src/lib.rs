// SPDX-License-Identifier: MPL-2.0

//! The module to define and parse kernel command-line arguments.
//!
//! The format of the Asterinas command line string conforms
//! to the Linux kernel command line rules:
//!
//! <https://docs.kernel.org/admin-guide/kernel-parameters.html>
//!
#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::{
    collections::BTreeMap,
    ffi::CString,
    string::{String, ToString},
    vec::Vec,
};
use core::{
    str::FromStr,
    sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering},
};

use component::{ComponentInitError, init_component};
pub use inventory::submit;
use spin::Once;

/// Defines a **single-value** `key=value` kernel command-line parameter.
///
/// During command-line parsing, all occurrences of `$name` are grouped and then
/// handled with **last-wins** semantics.
///
/// # Arguments
/// - `$name`: Parameter name (e.g. `"log_level"`).
/// - `$storage`: Storage location for the parsed value. Its type must implement
///   [`ParamStorage`] (e.g. [`spin::Once<T>`], [`AtomicU8`], [`AtomicU32`], [`AtomicBool`]).
///
/// # Parsing
/// The stored value type `S::Value` must implement [`ParseParamValue`].
/// By default, any type implementing [`core::str::FromStr`] automatically
/// implements [`ParseParamValue`].
///
/// # Example
/// ```ignore
/// static LOG_LEVEL: AtomicU8 = AtomicU8::new(0);
/// define_kv_param!("log_level", LOG_LEVEL);
/// ```
#[macro_export]
macro_rules! define_kv_param {
    ($name:expr, $storage:expr) => {
        $crate::__define_param!(@late, $name, $storage, $crate::setup_kv_param);
    };
}

/// Defines an **early** `key=value` kernel command-line parameter.
///
/// Almost same as [`define_kv_param!`], but it is processed earlier in the boot sequence.
#[macro_export]
macro_rules! define_kv_param_early {
    ($name:expr, $storage:expr) => {
        $crate::__define_param!(@early, $name, $storage, $crate::setup_kv_param);
    };
}

/// Defines a **repeatable** `key=value` kernel command-line parameter.
///
/// Unlike [`define_kv_param!`], this parameter may appear multiple times; the
/// framework collects **all** values and passes them to the parser in one shot.
///
/// # Arguments
/// - `$name`: Parameter name (e.g. `"console"`).
/// - `$storage`: Storage location for the parsed value. Its type must implement
///   [`ParamStorage`].
///
/// # Parsing
/// The stored value type `S::Value` must implement [`ParseRepeatableParamValue`].
/// This crate provides a default implementation for `Vec<T>` where `T: FromStr`.
///
/// # Example
/// ```ignore
/// static CONSOLES: Once<Vec<String>> = Once::new();
/// define_repeatable_kv_param!("console", CONSOLES);
/// ```
#[macro_export]
macro_rules! define_repeatable_kv_param {
    ($name:expr, $storage:expr) => {
        $crate::__define_param!(@late, $name, $storage, $crate::setup_repeatable_kv_param);
    };
}

/// Defines an **early** `key=value` kernel command-line parameter.
///
/// Almost same as [`define_repeatable_kv_param!`], but it is processed earlier in the boot sequence.
#[macro_export]
macro_rules! define_repeatable_kv_param_early {
    ($name:expr, $storage:expr) => {
        $crate::__define_param!(@early, $name, $storage, $crate::setup_repeatable_kv_param);
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
///   [`ParamStorage`].
///
/// # Parsing
/// The stored value type `S::Value` must implement [`ParseFlag`]. This crate
/// provides a default `bool` implementation:
/// - `flag` / `flag=1` / `flag=on|yes|true` => `true`
/// - `flag=0` / `flag=off|no|false` => `false`
///
/// # Example
/// ```ignore
/// static DEBUG: AtomicBool = AtomicBool::new(false);
/// define_flag_param!("debug", DEBUG);
/// ```
#[macro_export]
macro_rules! define_flag_param {
    ($name:expr, $storage:expr) => {
        $crate::__define_param!(@late, $name, $storage, $crate::setup_flag_param);
    };
}

/// Defines an **early** `key=value` kernel command-line parameter.
///
/// Almost same as [`define_flag_param!`], but it is processed earlier in the boot sequence.
#[macro_export]
macro_rules! define_flag_param_early {
    ($name:expr, $storage:expr) => {
        $crate::__define_param!(@early, $name, $storage, $crate::setup_flag_param);
    };
}

/// Helper for defining kernel command-line parameter.
#[macro_export]
macro_rules! __define_param {
    (@late, $name:expr, $storage:expr, $setup:path) => {
        const _: () = {
            fn __kparam_setup(occurrences: &[(&str, Option<&str>)]) {
                $setup(&$storage, occurrences, $name);
            }
            $crate::submit! {
                $crate::KernelParam { name: $name, setup: __kparam_setup, early: false }
            }
        };
    };

    (@early, $name:expr, $storage:expr, $setup:path) => {
        const _: () = {
            fn __kparam_setup(occurrences: &[(&str, Option<&str>)]) {
                $setup(&$storage, occurrences, $name);
            }
            $crate::submit! {
                $crate::KernelParam { name: $name, setup: __kparam_setup, early: true }
            }
        };
    };
}

/// The arguments passed to the init process, extracted from the kernel command line.
#[derive(PartialEq, Debug)]
pub struct InitprocArgs {
    argv: Vec<CString>,
    envp: Vec<CString>,
}

impl InitprocArgs {
    /// Gets the argument vector (`argv`) of the init process.
    pub fn argv(&self) -> &Vec<CString> {
        &self.argv
    }

    /// Gets the environment vector (`envp`) of the init process.
    pub fn envp(&self) -> &Vec<CString> {
        &self.envp
    }
}

/// Trait for types that can store a parsed parameter value.
pub trait ParamStorage: Sync + 'static {
    type Value;
    fn store_param(&self, value: Self::Value);
}

impl<T: Send + Sync + 'static> ParamStorage for Once<T> {
    type Value = T;
    fn store_param(&self, value: T) {
        self.call_once(|| value);
    }
}

impl ParamStorage for AtomicU8 {
    type Value = u8;
    fn store_param(&self, value: u8) {
        self.store(value, Ordering::Relaxed);
    }
}

impl ParamStorage for AtomicU32 {
    type Value = u32;
    fn store_param(&self, value: u32) {
        self.store(value, Ordering::Relaxed);
    }
}

impl ParamStorage for AtomicBool {
    type Value = bool;
    fn store_param(&self, value: bool) {
        self.store(value, Ordering::Relaxed);
    }
}

/// Errors while parsing kernel command line parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamError {
    InvalidValue,
}

/// Parse a single-value key-value parameter (e.g., log_level=3).
///
/// If the parameter appears multiple times, the framework applies last-wins
/// semantics (matching Linux behavior) and passes only the final value.
pub trait ParseParamValue: Sized {
    fn parse_param(value: &str) -> Result<Self, ParamError>;
}

/// Parse a repeatable key-value parameter (e.g., "console=ttyS0 console=ttyS1").
///
/// The framework collects all occurrences and passes the full slice.
pub trait ParseRepeatableParamValue: Sized {
    fn parse_all(values: &[&str]) -> Result<Self, ParamError>;
}

/// Parse a flag parameter (e.g., "ro", "debug", "nokaslr").
///
/// Flags may optionally accept a value (e.g., "debug=1" in Linux).
pub trait ParseFlag: Sized {
    /// Called when the flag is present on the command line.
    /// `value` is `Some(v)` if written as "flag=v", `None` if bare.
    fn parse_flag(value: Option<&str>) -> Result<Self, ParamError>;
}

/// Any `FromStr` type can be a single-value parameter.
impl<T: FromStr> ParseParamValue for T {
    fn parse_param(value: &str) -> Result<Self, ParamError> {
        value.parse().map_err(|_| ParamError::InvalidValue)
    }
}

/// A `Vec<T>` where `T: FromStr` can be a repeatable parameter.
/// (No coherence conflict: `Vec<T>` does not implement `FromStr`.)
impl<T: FromStr> ParseRepeatableParamValue for Vec<T> {
    fn parse_all(values: &[&str]) -> Result<Self, ParamError> {
        values
            .iter()
            .map(|v| v.parse().map_err(|_| ParamError::InvalidValue))
            .collect()
    }
}

/// `bool` as a flag: bare flag means `true`; "flag=1"/"flag=on" also accepted.
impl ParseFlag for bool {
    fn parse_flag(value: Option<&str>) -> Result<Self, ParamError> {
        match value {
            None | Some("1") | Some("on") | Some("yes") | Some("true") => Ok(true),
            Some("0") | Some("off") | Some("no") | Some("false") => Ok(false),
            _ => Err(ParamError::InvalidValue),
        }
    }
}

#[derive(Debug)]
pub struct KernelParam {
    pub name: &'static str,
    pub setup: fn(occurrences: &[(&str, Option<&str>)]),
    pub early: bool,
}

inventory::collect!(KernelParam);

/// Helper for single-value key-value parameters (last-wins).
pub fn setup_kv_param<S: ParamStorage>(
    storage: &S,
    occurrences: &[(&str, Option<&str>)],
    name: &str,
) where
    S::Value: ParseParamValue,
{
    let last = match occurrences.last() {
        Some((_, val)) => *val, // Option<&str> is Copy
        None => return,
    };
    match last {
        Some(value) => match S::Value::parse_param(value) {
            Ok(v) => storage.store_param(v),
            Err(_) => log::warn!("invalid value for kernel parameter '{}'", name),
        },
        None => log::warn!("kernel parameter '{}' requires a value", name),
    }
}

/// Helper for repeatable key-value parameters (all occurrences).
pub fn setup_repeatable_kv_param<S: ParamStorage>(
    storage: &S,
    occurrences: &[(&str, Option<&str>)],
    name: &str,
) where
    S::Value: ParseRepeatableParamValue,
{
    let values: Vec<&str> = occurrences.iter().filter_map(|(_, val)| *val).collect();
    if values.is_empty() {
        log::warn!("repeatable parameter '{}' requires values", name);
        return;
    }
    match S::Value::parse_all(&values) {
        Ok(v) => storage.store_param(v),
        Err(_) => log::warn!("invalid value for kernel parameter '{}'", name),
    }
}

/// Helper for flag parameters (last-wins, optional value).
pub fn setup_flag_param<S: ParamStorage>(
    storage: &S,
    occurrences: &[(&str, Option<&str>)],
    name: &str,
) where
    S::Value: ParseFlag,
{
    let last = match occurrences.last() {
        Some((_, val)) => *val,
        None => return,
    };
    match S::Value::parse_flag(last) {
        Ok(v) => storage.store_param(v),
        Err(_) => log::warn!("invalid value for flag '{}'", name),
    }
}

pub static INIT_PROC_ARGS: Once<InitprocArgs> = Once::new();

#[init_component]
fn init() -> Result<(), ComponentInitError> {
    INIT_PROC_ARGS.call_once(|| dispatch_params(ostd::boot::boot_info().kernel_cmdline.as_str()));

    Ok(())
}

// Splits the command line string by spaces but preserve
// ones that are protected by double quotes(`"`).
fn split_arg(input: &str) -> impl Iterator<Item = &str> {
    let mut inside_quotes = false;

    input.split(move |c: char| {
        if c == '"' {
            inside_quotes = !inside_quotes;
        }

        !inside_quotes && c.is_whitespace()
    })
}

fn dispatch_params(cmdline: &str) -> InitprocArgs {
    let mut result: InitprocArgs = InitprocArgs {
        argv: Vec::new(),
        envp: Vec::new(),
    };

    let mut kcmdline_end = false;

    // Step 1: Tokenize and group by normalized parameter name.
    let mut grouped: BTreeMap<String, Vec<(&str, Option<&str>)>> = BTreeMap::new();

    for arg in split_arg(cmdline) {
        // Everything after "--" goes to init.
        if kcmdline_end {
            result.argv.push(CString::new(arg).unwrap());
            continue;
        }
        if arg == "--" {
            kcmdline_end = true;
            continue;
        }

        let (key, value) = match arg.find('=') {
            Some(pos) => (&arg[..pos], Some(&arg[pos + 1..])),
            None => (arg, None),
        };
        // Normalize hyphens to underscores (Linux compatibility)
        let normalized = key.replace('-', "_");
        // Group by normalized name, but keep the original key and value for forwarding.
        grouped.entry(normalized).or_default().push((key, value));
    }

    // Step 2: Build lookup from registered param name to handler.
    let registry: BTreeMap<&str, &KernelParam> = inventory::iter::<KernelParam>
        .into_iter()
        .map(|p| (p.name, p))
        .collect();

    // Step 3: Dispatch each group to its handler.
    let mut recognized = Vec::new();
    for (name, occurrences) in &grouped {
        if let Some(param) = registry.get(name.as_str()) {
            recognized.push((*param, occurrences));
        } else {
            // Unknown parameter: forward to init
            if name.contains('.') {
                // The entry contains a dot, which is treated as a module argument.
                // Unrecognized module arguments are ignored.
                continue;
            } else if let Some((key, Some(value))) = occurrences.last().copied() {
                // If the entry is not recognized, it is passed to the init process.
                // Pattern 'entry=value' is treated as the init environment.
                let envp_entry = CString::new(key.to_string() + "=" + value).unwrap();
                result.envp.push(envp_entry);
            } else if let Some((key, None)) = occurrences.last().copied() {
                // If the entry is not recognized, it is passed to the init process.
                // Pattern 'entry' without value is treated as the init argument.
                let argv_entry = CString::new(key.to_string()).unwrap();
                result.argv.push(argv_entry);
            } else {
                // This case should not happen since `occurrences` should never be empty.
                debug_assert!(!occurrences.is_empty());
            }
        }
    }

    // Step 4: Call the setup function for each recognized parameter.
    let (early_params, params): (Vec<_>, Vec<_>) =
        recognized.into_iter().partition(|(p, _)| p.early);
    early_params
        .into_iter()
        .chain(params)
        .for_each(|(param, occurrences)| (param.setup)(occurrences));

    result
}

macro_rules! define_unimplemented_param {
    ($($name:expr),+ $(,)?) => {
        $(
            const _: () = {
                fn __kparam_setup(occurrences: &[(&str, Option<&str>)]) {
                    $crate::setup_unimplemented(occurrences, $name);
                }
                $crate::submit! {
                    $crate::KernelParam { name: $name, setup: __kparam_setup, early: false }
                }
            };
        )+
    };
}

fn setup_unimplemented(occurrences: &[(&str, Option<&str>)], name: &str) {
    if !occurrences.is_empty() {
        log::warn!("kernel parameter '{}' is not yet implemented", name);
    }
}

// Placeholders for recognized but unimplemented kernel command-line parameters; these
// consume matching parameters so they are not forwarded to init and avoid unexpected
// boot behavior. A warning is emitted when such parameters appear.
define_unimplemented_param!(
    "tsc",
    "no_timer_check",
    "reboot",
    "pci",
    "debug",
    "panic",
    "nr_cpus",
    "selinux",
    "initrd"
);
