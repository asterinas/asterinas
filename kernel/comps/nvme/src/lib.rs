// SPDX-License-Identifier: MPL-2.0

//! NVMe (Non-Volatile Memory Express) driver for Asterinas.
//!
//! This driver implements support for NVMe storage devices following the
//! NVM Express Base Specification Revision 2.0.
//!
//! Reference: NVM Express Base Specification Revision 2.0

#![no_std]
extern crate alloc;
#[macro_use]
extern crate ostd_pod;

use aster_block::MajorIdOwner;
use component::{ComponentInitError, init_component};
use device::block_device::NvmeBlockDevice;
use log::error;
use spin::Once;
use transport::pci::{
    NVME_PCI_DRIVER,
    device::{NvmePciTransport, NvmePciTransportLock},
};

use crate::nvme_regs::{NvmeRegs32, NvmeRegs64};

pub mod device;
mod msix;
mod nvme_cmd;
mod nvme_queue;
mod nvme_regs;
mod transport;

pub(crate) static NVME_BLOCK_MAJOR_ID: Once<MajorIdOwner> = Once::new();

#[init_component]
fn nvme_init() -> Result<(), ComponentInitError> {
    NVME_BLOCK_MAJOR_ID.call_once(|| aster_block::allocate_major().unwrap());
    transport::init();

    while let Some(transport) = pop_device_transport() {
        let res = NvmeBlockDevice::init(transport);
        if res.is_err() {
            error!("[NVMe]: Device initialization error:{:?}", res);
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
