// SPDX-License-Identifier: MPL-2.0

//! The assembler crate for the Asterinas kernel.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

// High-level components as external crates.
//
// Keep this list in sync with the high-level component dependencies in `Cargo.toml`.
// Each high-level component crate must have an explicit `extern crate` declaration
// to ensure that its component registration and initialization code are linked into
// the kernel, because the assembler does not otherwise reference symbols from that crate.
extern crate aster_uart as _;

#[ostd::main]
fn main() {
    aster_core::boot();
}
