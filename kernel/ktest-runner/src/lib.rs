// SPDX-License-Identifier: MPL-2.0

#![no_std]
#![no_main]
#![deny(unsafe_code)]
extern crate ostd;

#[ostd::main]
fn main() {
    ostd::ktest_main();
}