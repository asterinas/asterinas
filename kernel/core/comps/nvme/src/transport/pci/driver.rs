// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use aster_device::{
    bus::Driver,
    common::{Attr, Error, Result},
};
use aster_pci::bus::{PciBus, PciDevice, PciMatch};

use super::transport::NvmePciTransport;

pub(crate) struct NvmePciDriver;

impl Driver<PciBus> for NvmePciDriver {
    fn name(&self) -> &str {
        "nvme"
    }

    fn match_data(&self) -> &[PciMatch] {
        const NVME_DEVICE_CLASS: u8 = 0x01;
        const NVME_DEVICE_SUBCLASS: u8 = 0x08;
        const NVME_DEVICE_PROG_IF: u8 = 0x02;

        &[PciMatch::Class {
            class: NVME_DEVICE_CLASS,
            subclass: NVME_DEVICE_SUBCLASS,
            prog_if: NVME_DEVICE_PROG_IF,
        }]
    }

    fn on_probe(&self, device: &Arc<PciDevice>) -> Result<()> {
        let transport = NvmePciTransport::new(device.clone()).map_err(|_| Error::ProbeFailed)?;
        super::push_device_transport(transport);
        Ok(())
    }

    fn on_release(&self, _device: &Arc<PciDevice>) {
        // FIXME: Stop I/O and unregister NVMe devices before releasing controller resources.
        panic!("nvme driver removal is not supported");
    }

    fn dev_attrs(&self) -> &'static [Attr<PciDevice>] {
        &[]
    }

    fn suppress_bind_attrs(&self) -> bool {
        true
    }
}
