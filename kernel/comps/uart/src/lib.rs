// SPDX-License-Identifier: MPL-2.0

#![no_std]
#![deny(unsafe_code)]
#![feature(let_chains)]

use component::{init_component, ComponentInitError};

#[cfg(target_arch = "riscv64")]
mod sifive;

#[init_component]
fn uart_init() -> Result<(), ComponentInitError> {
    #[cfg(target_arch = "riscv64")]
    sifive::init();

    Ok(())
}

#[cfg_attr(not(target_arch = "riscv64"), expect(unused))]
trait Uart {
    fn init(&self, clock_hz: u32);

    fn transmit(&self, byte: u8) -> ostd::Result<()>;

    fn receive(&self) -> Option<u8>;
}
