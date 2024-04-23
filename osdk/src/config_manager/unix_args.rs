// SPDX-License-Identifier: MPL-2.0

//! This module contains utilities for manipulating common Unix command-line arguments.

use std::process;

use indexmap::{IndexMap, IndexSet};

use crate::{error::Errno, error_msg};

/// Apply key-value pairs to an array of strings.
///
/// The provided arguments will be appended to the array if the key is not already present or if the key is a multi-value key.
/// Otherwise, the value will be updated.
pub fn apply_kv_array(
    array: &mut Vec<String>,
    args: &Vec<String>,
    seperator: &str,
    multi_value_keys: &[&str],
) {
    let multi_value_keys = {
        let mut inferred_keys = infer_multi_value_keys(array, seperator);
        for key in multi_value_keys {
            inferred_keys.insert(key.to_string());
        }
        inferred_keys
    };

    debug!("multi value keys: {:?}", multi_value_keys);

    // We use IndexMap to keep key orders
    let mut key_strings = IndexMap::new();
    let mut multi_value_key_strings: IndexMap<String, Vec<String>> = IndexMap::new();
    for item in array.drain(..) {
        // Each key-value string has two patterns:
        // 1. Seperated by separator: key value / key=value
        if let Some(key) = get_key(&item, seperator) {
            if multi_value_keys.contains(&key) {
                if let Some(v) = multi_value_key_strings.get_mut(&key) {
                    v.push(item);
                } else {
                    let v = vec![item];
                    multi_value_key_strings.insert(key, v);
                }
                continue;
            }

            key_strings.insert(key, item);
            continue;
        }
        // 2. Only key, no value
        key_strings.insert(item.clone(), item);
    }

    for arg in args {
        if let Some(key) = get_key(arg, seperator) {
            if multi_value_keys.contains(&key) {
                if let Some(v) = multi_value_key_strings.get_mut(&key) {
                    v.push(arg.to_owned());
                } else {
                    let v = vec![arg.to_owned()];
                    multi_value_key_strings.insert(key, v);
                }
                continue;
            }

            key_strings.insert(key, arg.to_owned());
            continue;
        }

        key_strings.insert(arg.to_owned(), arg.to_owned());
    }

    *array = key_strings.into_iter().map(|(_, value)| value).collect();

    for (_, mut values) in multi_value_key_strings {
        array.append(&mut values);
    }
}

fn infer_multi_value_keys(array: &Vec<String>, seperator: &str) -> IndexSet<String> {
    let mut multi_val_keys = IndexSet::new();

    let mut occured_keys = IndexSet::new();
    for item in array {
        let Some(key) = get_key(item, seperator) else {
            continue;
        };

        if occured_keys.contains(&key) {
            multi_val_keys.insert(key);
        } else {
            occured_keys.insert(key);
        }
    }

    multi_val_keys
}

pub fn get_key(item: &str, seperator: &str) -> Option<String> {
    let split = item.split(seperator).collect::<Vec<_>>();
    let len = split.len();
    if len > 2 || len == 0 {
        error_msg!("`{}` is an invalid argument.", item);
        process::exit(Errno::ParseMetadata as _);
    }

    if len == 1 {
        return None;
    }

    let key = split.first().unwrap();

    Some(key.to_string())
}

#[cfg(test)]
pub mod test {
    use super::*;

    #[test]
    fn test_get_key() {
        let string1 = "init=/bin/init";
        let key = get_key(string1, "=").unwrap();
        assert_eq!(key.as_str(), "init");

        let string2 = "-m 2G";
        let key = get_key(string2, " ").unwrap();
        assert_eq!(key.as_str(), "-m");

        let string3 = "-device virtio-keyboard-pci,disable-legacy=on,disable-modern=off";
        let key = get_key(string3, " ").unwrap();
        assert_eq!(key.as_str(), "-device");

        let string4 = "-device";
        assert!(get_key(string4, " ").is_none());
    }

    #[test]
    fn test_apply_kv_array() {
        let qemu_args = &[
            "-enable-kvm",
            "-m 2G",
            "-device virtio-blk-pci,bus=pcie.0,addr=0x6,drive=x0,disable-legacy=on,disable-modern=off",
            "-device virtio-keyboard-pci,disable-legacy=on,disable-modern=off",
        ];

        let args = &["-m 100G", "-device ioh3420,id=pcie.0,chassis=1"];

        let expected = &[
            "-enable-kvm",
            "-m 100G",
            "-device virtio-blk-pci,bus=pcie.0,addr=0x6,drive=x0,disable-legacy=on,disable-modern=off",
            "-device virtio-keyboard-pci,disable-legacy=on,disable-modern=off",
            "-device ioh3420,id=pcie.0,chassis=1",
        ];

        let mut array = qemu_args.iter().map(ToString::to_string).collect();
        let args = args.iter().map(ToString::to_string).collect();
        apply_kv_array(&mut array, &args, " ", &["-device"]);

        let expected: Vec<_> = expected.iter().map(ToString::to_string).collect();
        assert_eq!(expected, array);
    }
}
