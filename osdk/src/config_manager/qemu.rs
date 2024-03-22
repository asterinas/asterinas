// SPDX-License-Identifier: MPL-2.0

//! A module about QEMU arguments.

use std::process;

use super::unix_args::{apply_kv_array, get_key};

use crate::{error::Errno, error_msg};

pub fn apply_qemu_args_addition(target: &mut Vec<String>, args: &Vec<String>) {
    // check qemu_args
    for arg in target.iter() {
        check_qemu_arg(arg);
    }
    for arg in args.iter() {
        check_qemu_arg(arg);
    }

    apply_kv_array(target, args, " ", MULTI_VALUE_KEYS);
}

// Below are keys in qemu arguments. The key list is not complete.

/// Keys with multiple values
const MULTI_VALUE_KEYS: &[&str] = &[
    "-device", "-chardev", "-object", "-netdev", "-drive", "-cdrom",
];
/// Keys with only single value
const SINGLE_VALUE_KEYS: &[&str] = &["-cpu", "-machine", "-m", "-serial", "-monitor", "-display"];
/// Keys with no value
const NO_VALUE_KEYS: &[&str] = &["--no-reboot", "-nographic", "-enable-kvm"];
/// Keys are not allowed to set in configuration files and command line
const NOT_ALLOWED_TO_SET_KEYS: &[&str] = &["-kernel", "-initrd"];

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
