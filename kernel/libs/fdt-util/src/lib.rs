// SPDX-License-Identifier: MPL-2.0

//! Utilities for OSTD users to work with device trees.
#![no_std]
#![deny(unsafe_code)]
#![cfg(any(
    target_arch = "riscv64",
    target_arch = "loongarch64",
    target_arch = "aarch64"
))]
#![feature(array_try_from_fn)]
#![cfg_attr(target_arch = "riscv64", feature(iter_next_chunk))]

// Set this crate's log prefix for `ostd::log`.
macro_rules! __log_prefix {
    () => {
        "fdt: "
    };
}

mod io_mem;
#[cfg(target_arch = "riscv64")]
mod irq_line;

pub use self::io_mem::AcquireIoMems;
#[cfg(target_arch = "riscv64")]
pub use self::irq_line::AcquireIrqLines;
