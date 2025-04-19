// SPDX-License-Identifier: MPL-2.0

#![no_std]
#![deny(unsafe_code)]
#![allow(dead_code)]
extern crate alloc;

use component::{ComponentInitError, init_component};
use device::block_device::NVMeBlockDevice;
use log::{error, info};
use transport::pci::{NVME_PCI_DRIVER, device::NVMePciTransport};

use crate::nvme_regs::{NVMeRegs32, NVMeRegs64};

pub mod device;
mod nvme_cmd;
mod nvme_queue;
mod nvme_regs;
mod transport;

#[init_component]
fn nvme_init() -> Result<(), ComponentInitError> {
    transport::init();

    while let Some(transport) = pop_device_transport() {
        info!(
            "[NVMe]: Capabilities 0x{:X}",
            transport.read_reg64(NVMeRegs64::Cap)
        );
        info!(
            "[NVMe]: Version 0x{:X}",
            transport.read_reg32(NVMeRegs32::Vs)
        );
        info!(
            "[NVMe]: Controller Configuration 0x{:X}",
            transport.read_reg32(NVMeRegs32::Cc)
        );
        info!(
            "[NVMe]: Controller Status 0x{:X}",
            transport.read_reg32(NVMeRegs32::Csts)
        );

        let res = NVMeBlockDevice::init(transport);
        if res.is_err() {
            error!("[NVMe]: Device initialization error:{:?}", res);
        }
    }

    Ok(())
}

fn pop_device_transport() -> Option<NVMePciTransport> {
    if let Some(device) = NVME_PCI_DRIVER.get().unwrap().pop_device_transport() {
        return Some(device);
    }
    None
}
