// SPDX-License-Identifier: MPL-2.0

#![no_std]
#![forbid(unsafe_code)]

use aster_frame::prelude::*;

#[aster_main]
fn kernel_main() {
    let avail_mem_as_mb = mymodule::available_memory() / 1_000_000;
    println!("The available memory is {} MB", avail_mem_as_mb);
}
