// SPDX-License-Identifier: MPL-2.0

//! A module about QEMU settings and arguments.

use std::{path::PathBuf, process};

use crate::{
    arch::{get_default_arch, Arch},
    config::{
        eval::{eval, Vars},
        unix_args::{apply_kv_array, get_key},
    },
    error::Errno,
    error_msg,
};

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QemuScheme {
    /// The additional arguments for running QEMU, in the form of raw
    /// command line arguments.
    pub args: Option<String>,
    /// The path of qemu
    pub path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Qemu {
    pub args: String,
    pub path: PathBuf,
}

impl Default for Qemu {
    fn default() -> Self {
        Qemu {
            args: String::new(),
            path: PathBuf::from(get_default_arch().system_qemu()),
        }
    }
}

impl Qemu {
    pub fn apply_qemu_args(&mut self, args: &Vec<String>) {
        let target = match shlex::split(&self.args) {
            Some(v) => v,
            None => {
                error_msg!("Failed to parse qemu args: {:#?}", &self.args);
                process::exit(Errno::ParseMetadata as _);
            }
        };

        // Join the key value arguments as a single element
        let mut joined = Vec::new();
        let mut consumed = false;
        for (first, second) in target.iter().zip(target.iter().skip(1)) {
            if consumed {
                consumed = false;
                continue;
            }
            if first.starts_with('-') && !first.starts_with("--") && !second.starts_with('-') {
                joined.push(format!("{} {}", first, second));
                consumed = true;
            } else {
                joined.push(first.clone());
            }
        }
        if !consumed {
            joined.push(target.last().unwrap().clone());
        }

        // Check the soundness of qemu arguments
        for arg in joined.iter() {
            check_qemu_arg(arg);
        }
        for arg in joined.iter() {
            check_qemu_arg(arg);
        }

        apply_kv_array(&mut joined, args, " ", MULTI_VALUE_KEYS);

        self.args = joined.join(" ");
    }
}

impl QemuScheme {
    pub fn inherit(&mut self, from: &Self) {
        if from.args.is_some() {
            self.args = from.args.clone();
        }
        if from.path.is_some() {
            self.path = from.path.clone();
        }
    }

    pub fn finalize(self, vars: &Vars, arch: Arch) -> Qemu {
        Qemu {
            args: self
                .args
                .map(|args| match eval(vars, &args) {
                    Ok(v) => v,
                    Err(e) => {
                        error_msg!("Failed to evaluate qemu args: {:#?}", e);
                        process::exit(Errno::ParseMetadata as _);
                    }
                })
                .unwrap_or_default(),
            path: self.path.unwrap_or(PathBuf::from(arch.system_qemu())),
        }
    }
}

// Below are checked keys in qemu arguments. The key list is non-exhaustive.

/// Keys with multiple values
const MULTI_VALUE_KEYS: &[&str] = &[
    "-device", "-chardev", "-object", "-netdev", "-drive", "-cdrom",
];
/// Keys with only single value
const SINGLE_VALUE_KEYS: &[&str] = &["-cpu", "-machine", "-m", "-serial", "-monitor", "-display"];
/// Keys with no value
const NO_VALUE_KEYS: &[&str] = &["--no-reboot", "-nographic", "-enable-kvm"];
/// Keys are not allowed to set in configuration files and command line
const NOT_ALLOWED_TO_SET_KEYS: &[&str] = &["-kernel", "-append", "-initrd"];

fn check_qemu_arg(arg: &str) {
    let key = if let Some(key) = get_key(arg, " ") {
        key
    } else {
        arg.to_string()
    };

    if NOT_ALLOWED_TO_SET_KEYS.contains(&key.as_str()) {
        error_msg!("`{}` is not allowed to set", arg);
        process::exit(Errno::ParseMetadata as _);
    }

    if NO_VALUE_KEYS.contains(&key.as_str()) && key.as_str() != arg {
        error_msg!("`{}` cannot have value", arg);
        process::exit(Errno::ParseMetadata as _);
    }

    if (SINGLE_VALUE_KEYS.contains(&key.as_str()) || MULTI_VALUE_KEYS.contains(&key.as_str()))
        && key.as_str() == arg
    {
        error_msg!("`{}` should have value", arg);
        process::exit(Errno::ParseMetadata as _);
    }
}
