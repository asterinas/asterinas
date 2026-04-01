// SPDX-License-Identifier: MPL-2.0

//! Kernel command-line parameter parsing and storage.
//!
//! This module defines traits for parsing different types of parameters and
//! provides implementations for common cases (e.g., `FromStr` types, flags).

use alloc::vec::Vec;
use core::{
    str::FromStr,
    sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering},
};

use spin::Once;

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

/// Parses a single-value key-value parameter (e.g., log_level=3).
///
/// If the parameter appears multiple times, the framework applies last-wins
/// semantics (matching Linux behavior) and passes only the final value.
pub trait ParseParamValue: Sized {
    fn parse_param(value: &str) -> Result<Self, ParamError>;
}

/// Parses a repeatable key-value parameter (e.g., "console=ttyS0 console=ttyS1").
///
/// The framework collects all occurrences and passes the full slice.
pub trait ParseRepeatableParamValue: Sized {
    fn parse_all(values: &[&str]) -> Result<Self, ParamError>;
}

/// Parses a flag parameter (e.g., "ro", "debug", "nokaslr").
///
/// Flags may optionally accept a value (e.g., "debug=1" in Linux).
pub trait ParseFlag: Sized {
    /// Prases the flag with an optional value.
    ///
    /// If the flag is specified as "flag=v", then `value` is `Some(v)`;
    /// if the flag is bare, then `value` is `None`.
    fn parse_flag(value: Option<&str>) -> Result<Self, ParamError>;
}

/// Any `FromStr` type can be a single-value parameter.
impl<T: FromStr> ParseParamValue for T {
    fn parse_param(value: &str) -> Result<Self, ParamError> {
        value.parse().map_err(|_| ParamError::InvalidValue)
    }
}

/// A `Vec<T>` where `T: FromStr` can be a repeatable parameter.
impl<T: FromStr> ParseRepeatableParamValue for Vec<T> {
    fn parse_all(values: &[&str]) -> Result<Self, ParamError> {
        values
            .iter()
            .map(|v| v.parse().map_err(|_| ParamError::InvalidValue))
            .collect()
    }
}

/// `bool` as a flag: a bare flag means `true`;
/// flags with typical values such as `1`/`0`, `on`/`off` are also accepted.
impl ParseFlag for bool {
    fn parse_flag(value: Option<&str>) -> Result<Self, ParamError> {
        match value {
            None | Some("1") | Some("on") | Some("yes") | Some("true") => Ok(true),
            Some("0") | Some("off") | Some("no") | Some("false") => Ok(false),
            _ => Err(ParamError::InvalidValue),
        }
    }
}

#[doc(hidden)]
pub fn setup_kv_param<S: ParamStorage>(storage: &S, occurrences: &[Option<&str>], name: &str)
where
    S::Value: ParseParamValue,
{
    let Some(last) = occurrences.last() else {
        return;
    };
    match last {
        Some(value) => match S::Value::parse_param(value) {
            Ok(v) => storage.store_param(v),
            Err(_) => ostd::warn!("invalid value for kernel parameter '{}'", name),
        },
        None => ostd::warn!("kernel parameter '{}' requires a value", name),
    }
}

#[doc(hidden)]
pub fn setup_repeatable_kv_param<S: ParamStorage>(
    storage: &S,
    occurrences: &[Option<&str>],
    name: &str,
) where
    S::Value: ParseRepeatableParamValue,
{
    let values: Vec<&str> = occurrences.iter().filter_map(|val| *val).collect();
    if values.is_empty() {
        ostd::warn!("repeatable parameter '{}' requires values", name);
        return;
    }
    match S::Value::parse_all(&values) {
        Ok(v) => storage.store_param(v),
        Err(_) => ostd::warn!("invalid value for kernel parameter '{}'", name),
    }
}

#[doc(hidden)]
pub fn setup_flag_param<S: ParamStorage>(storage: &S, occurrences: &[Option<&str>], name: &str)
where
    S::Value: ParseFlag,
{
    let last = match occurrences.last() {
        Some(val) => *val,
        None => return,
    };
    match S::Value::parse_flag(last) {
        Ok(v) => storage.store_param(v),
        Err(_) => ostd::warn!("invalid value for flag '{}'", name),
    }
}
