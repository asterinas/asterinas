// SPDX-License-Identifier: MPL-2.0

//! This module contains utilities for manipulating common Unix command-line arguments.

use indexmap::{IndexMap, IndexSet};

use regex::Regex;
use std::sync::OnceLock;

/// Split a string of Unix arguments into an array of key-value strings or switches.
/// Positional arguments are not supported.
pub fn split_to_kv_array(args: &str) -> Vec<String> {
    let target = split_preserving_quotes(args);

    // Join the key value arguments as a single element
    let mut joined = Vec::<String>::new();
    let mut last_has_value = false;
    for elem in target {
        if !elem.starts_with('-')
            && !last_has_value
            && let Some(last) = joined.last_mut()
        {
            last.push(' ');
            last.push_str(&elem);
            last_has_value = true;
            continue;
        }

        joined.push(elem);
        last_has_value = false;
    }

    joined
}

/// Apply key-value pairs to an array of strings.
///
/// The provided arguments will be appended to the array if the key is not already present or if the key is a multi-value key.
/// Otherwise, the value will be updated.
pub fn apply_kv_array(
    array: &mut Vec<String>,
    args: &Vec<String>,
    separator: &str,
    multi_value_keys: &[&str],
) {
    let multi_value_keys = {
        let mut inferred_keys = infer_multi_value_keys(array, separator);
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
        // 1. Separated by separator: key value / key=value
        if let Some(key) = get_key(&item, separator) {
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
        if let Some(key) = get_key(arg, separator) {
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

fn infer_multi_value_keys(array: &Vec<String>, separator: &str) -> IndexSet<String> {
    let mut multi_val_keys = IndexSet::new();

    let mut occurred_keys = IndexSet::new();
    for item in array {
        let Some(key) = get_key(item, separator) else {
            continue;
        };

        if occurred_keys.contains(&key) {
            multi_val_keys.insert(key);
        } else {
            occurred_keys.insert(key);
        }
    }

    multi_val_keys
}

pub fn get_key(item: &str, separator: &str) -> Option<String> {
    item.split_once(separator)
        .map(|(key, _value)| key.to_string())
}

fn split_preserving_quotes(input: &str) -> Vec<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    // A Unix shell-like argument splitter that preserves quoted substrings.
    let re = RE.get_or_init(|| Regex::new(r#"'[^']*'|"(?:[^"\\]|\\.)*"|\S+"#).unwrap());

    // Process line by line, removing comments and splitting
    input
        .lines()
        .flat_map(|line| {
            // Remove comment from this line
            let line = if let Some(pos) = line.find('#') {
                &line[..pos]
            } else {
                line
            };

            // Split the line into tokens
            re.find_iter(line)
                .map(|m| m.as_str().to_string())
                .collect::<Vec<_>>()
        })
        .collect()
}

#[cfg(test)]
pub mod test {
    use super::*;

    #[test]
    fn test_get_key() {
        let string1 = "init=/bin/init";
        let key = get_key(string1, "=").unwrap();
        assert_eq!(key.as_str(), "init");

        let string2 = "-m 8G";
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
            "-m 8G",
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
