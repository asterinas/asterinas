// SPDX-License-Identifier: MPL-2.0

#![no_std]
#![no_main]
#![deny(unsafe_code)]
extern crate ostd;

#[ostd::main]
pub fn main() {
    component::init_all(component::parse_metadata!()).unwrap();
    aster_nix::init();
    aster_nix::run_first_process();
}
