// SPDX-License-Identifier: MPL-2.0

//! Universal asynchronous receiver-transmitter (UART).

#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

use component::{ComponentInitError, init_component};

#[cfg_attr(target_arch = "x86_64", path = "arch/x86/mod.rs")]
#[cfg_attr(target_arch = "riscv64", path = "arch/riscv/mod.rs")]
#[cfg_attr(target_arch = "loongarch64", path = "arch/loongarch/mod.rs")]
mod arch;

mod console;

pub const CONSOLE_NAME: &str = "Uart-Console";

#[init_component]
fn init() -> Result<(), ComponentInitError> {
    arch::init();
    Ok(())
}
