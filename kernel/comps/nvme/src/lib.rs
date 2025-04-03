// SPDX-License-Identifier: MPL-2.0

#![no_std]
#![deny(unsafe_code)]
#![allow(dead_code)]
extern crate alloc;

use component::{ComponentInitError, init_component};
use device::block_device::NvmeBlockDevice;
use log::{error, info};
use transport::pci::{NVME_PCI_DRIVER, device::NvmePciTransport};

use crate::nvme_regs::{NvmeRegs32, NvmeRegs64};

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
            "[Nvme]: Capabilities 0x{:X}",
            transport.read_reg64(NvmeRegs64::Cap)
        );
        info!(
            "[Nvme]: Version 0x{:X}",
            transport.read_reg32(NvmeRegs32::Vs)
        );
        info!(
            "[Nvme]: Controller Configuration 0x{:X}",
            transport.read_reg32(NvmeRegs32::Cc)
        );
        info!(
            "[Nvme]: Controller Status 0x{:X}",
            transport.read_reg32(NvmeRegs32::Csts)
        );

        let res = NvmeBlockDevice::init(transport);
        if res.is_err() {
            error!("[Nvme]: Device initialization error:{:?}", res);
        }
    }

    Ok(())
}

fn pop_device_transport() -> Option<NvmePciTransport> {
    if let Some(device) = NVME_PCI_DRIVER.get().unwrap().pop_device_transport() {
        return Some(device);
    }
    None
}
