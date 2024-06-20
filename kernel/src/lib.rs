// SPDX-License-Identifier: MPL-2.0

#![no_std]
#![no_main]
#![deny(unsafe_code)]
extern crate ostd;

use ostd::prelude::*;

#[ostd::main]
pub fn main() {
    println!("[kernel] finish init ostd");
    component::init_all(component::parse_metadata!()).unwrap();
    aster_nix::init();
    aster_nix::run_first_process();
}
