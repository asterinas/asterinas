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

// Set this crate's log prefix for `ostd::log`.
macro_rules! __log_prefix {
    () => {
        "nvme: "
    };
}

use aster_block::MajorIdOwner;
use component::{ComponentInitError, init_component};
use spin::Once;
use transport::pci::NVME_PCI_DRIVER;

pub use self::device::block_device::NvmeBlockDevice;

mod device;
mod msix;
mod nvme_cmd;
mod nvme_queue;
mod nvme_regs;
mod nvme_spec;
mod transport;

static NVME_BLOCK_MAJOR_ID: Once<MajorIdOwner> = Once::new();

#[init_component]
fn nvme_init() -> Result<(), ComponentInitError> {
    let major = aster_block::allocate_major().map_err(|_| ComponentInitError::Unknown)?;
    NVME_BLOCK_MAJOR_ID.call_once(|| major);

    transport::init();

    while let Some(transport) = NVME_PCI_DRIVER.get().unwrap().pop_device_transport() {
        let res = NvmeBlockDevice::init(transport);
        if res.is_err() {
            ostd::error!("Device initialization error: {:?}", res);
        }
    }

    Ok(())
}
