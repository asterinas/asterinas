// SPDX-License-Identifier: MPL-2.0

#![no_std]
#![no_main]
#![forbid(unsafe_code)]
extern crate aster_frame;

use aster_frame::prelude::*;

#[aster_main]
pub fn main() {
    println!("[kernel] finish init aster_frame");
    component::init_all(component::parse_metadata!()).unwrap();
    aster_nix::init();
    aster_nix::run_first_process();
}
