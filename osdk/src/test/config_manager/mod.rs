// SPDX-License-Identifier: MPL-2.0

use crate::config_manager::{apply_kv_array, get_key};
mod manifest;

#[test]
fn get_key_test() {
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
fn apply_kv_array_test() {
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
