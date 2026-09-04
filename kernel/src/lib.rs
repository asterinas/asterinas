// SPDX-License-Identifier: MPL-2.0

//! The assembler crate for the Asterinas kernel.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate aster_drm as _;

#[ostd::main]
fn main() {
    aster_core::boot();
}
