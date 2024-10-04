// SPDX-License-Identifier: MPL-2.0

#![no_std]
#![deny(unsafe_code)]

use ostd::prelude::*;

#[ostd::main]
fn kernel_main() {
    let avail_mem_as_mb = mylib::available_memory() / 1_000_000;
    println!("The available memory is {} MB", avail_mem_as_mb);
}
