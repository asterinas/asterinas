// SPDX-License-Identifier: MPL-2.0

//! Registry initialization tests.

use ostd::prelude::ktest;

use super::utils;

#[ktest]
fn roots_exist() {
    crate::init_for_ktest();

    for path in [
        "/devices",
        "/devices/virtual",
        "/bus",
        "/class",
        "/dev/char",
        "/dev/block",
    ] {
        assert!(utils::lookup(path).is_some(), "{path} missing");
    }
}
