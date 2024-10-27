// SPDX-License-Identifier: MPL-2.0

use alloc::{sync::Arc, vec::Vec};

use ostd::{
    bus::{
        pci::{
            bus::{PciDevice, PciDriver},
            common_device::PciCommonDevice,
        },
        BusProbeError,
    },
    sync::SpinLock,
};

use super::device::VirtioPciTransport;

#[derive(Debug)]
pub struct VirtioPciDriver {
    devices: SpinLock<Vec<VirtioPciTransport>>,
}

impl VirtioPciDriver {
    pub fn num_devices(&self) -> usize {
        self.devices
            .lock_with(|devices: &mut Vec<VirtioPciTransport>| devices.len())
    }

    pub fn pop_device_transport(&self) -> Option<VirtioPciTransport> {
        self.devices.lock_with(|devices| devices.pop())
    }

    pub(super) fn new() -> Self {
        VirtioPciDriver {
            devices: SpinLock::new(Vec::new()),
        }
    }
}

impl PciDriver for VirtioPciDriver {
    fn probe(
        &self,
        device: PciCommonDevice,
    ) -> Result<Arc<dyn PciDevice>, (BusProbeError, PciCommonDevice)> {
        const VIRTIO_DEVICE_VENDOR_ID: u16 = 0x1af4;
        if device.device_id().vendor_id != VIRTIO_DEVICE_VENDOR_ID {
            return Err((BusProbeError::DeviceNotMatch, device));
        }
        let transport = VirtioPciTransport::new(device)?;
        let device = transport.pci_device().clone();
        self.devices.lock_with(|devices| devices.push(transport));
        Ok(device)
    }
}
