// SPDX-License-Identifier: MPL-2.0

//! PCI bus in Asterinas
#![no_std]
#![deny(unsafe_code)]

use component::{init_component, ComponentInitError};

#[init_component]
fn pci_init() -> Result<(), ComponentInitError> {
    init();
    Ok(())
}
