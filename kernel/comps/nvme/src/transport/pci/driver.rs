// SPDX-License-Identifier: MPL-2.0

use alloc::{sync::Arc, vec::Vec};

use ostd::{
    bus::{
        BusProbeError,
        pci::{
            bus::{PciDevice, PciDriver},
            common_device::PciCommonDevice,
        },
    },
    sync::SpinLock,
};

use super::device::NvmePciTransport;
use crate::transport::pci::device::NvmePciDevice;

#[derive(Debug)]
pub struct NvmePciDriver {
    pub devices: SpinLock<Vec<NvmePciTransport>>,
}

impl NvmePciDriver {
    pub fn num_devices(&self) -> usize {
        self.devices.lock().len()
    }

    pub fn pop_device_transport(&self) -> Option<NvmePciTransport> {
        self.devices.lock().pop()
    }

    pub(super) fn new() -> Self {
        NvmePciDriver {
            devices: SpinLock::new(Vec::new()),
        }
    }
}

impl PciDriver for NvmePciDriver {
    fn probe(
        &self,
        device: PciCommonDevice,
    ) -> Result<Arc<dyn PciDevice>, (BusProbeError, PciCommonDevice)> {
        const NVME_DEVICE_CLASS: u8 = 0x01;
        const NVME_DEVICE_SUBCLASS: u8 = 0x08;
        const NVME_DEVICE_PROG_IF: u8 = 0x02;

        if device.device_id().class != NVME_DEVICE_CLASS
            || device.device_id().subclass != NVME_DEVICE_SUBCLASS
            || device.device_id().prog_if != NVME_DEVICE_PROG_IF
        {
            return Err((BusProbeError::DeviceNotMatch, device));
        }

        let device_id = *device.device_id();
        let transport = NvmePciTransport::new(device)?;

        self.devices.lock().push(transport);

        Ok(Arc::new(NvmePciDevice::new(device_id)))
    }
}
