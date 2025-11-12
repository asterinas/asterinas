// SPDX-License-Identifier: MPL-2.0

//! Flattened Device Tree (FDT) utilities.
//!
//! This crate provides some useful extension traits
//! for `FdtNode` from [the `fdt` crate](https://crates.io/crates/fdt)
//! on supported CPU architectures.

#![no_std]
#![deny(unsafe_code)]
#![cfg(any(
    target_arch = "riscv64",
    target_arch = "loongarch64",
    target_arch = "aarch64"
))]
#![feature(array_try_from_fn)]
#![cfg_attr(
    any(target_arch = "riscv64", target_arch = "aarch64"),
    feature(iter_next_chunk)
)]

// Set this crate's log prefix for `ostd::log`.
macro_rules! __log_prefix {
    () => {
        "fdt: "
    };
}

mod io_mem;
#[cfg(any(target_arch = "riscv64", target_arch = "aarch64"))]
mod irq_line;

pub use self::io_mem::AcquireIoMems;
#[cfg(any(target_arch = "riscv64", target_arch = "aarch64"))]
pub use self::irq_line::AcquireIrqLines;
