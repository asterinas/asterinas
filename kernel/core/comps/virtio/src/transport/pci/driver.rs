// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, sync::Arc};

use aster_device::{
    bus::Driver,
    common::{Attr, Error, Result},
};
use aster_pci::bus::{PciBus, PciDevice, PciMatch};

use super::{device::VirtioPciModernTransport, legacy::VirtioPciLegacyTransport};
use crate::transport::VirtioTransport;

// PCI device discovery identifiers (Virtio 1.2, section 4.1.2).
// Reference: <https://docs.oasis-open.org/virtio/virtio/v1.2/virtio-v1.2.html>
const VIRTIO_PCI_VENDOR_ID: u16 = 0x1af4;
const TRANSITIONAL_DEVICE_ID_MIN: u16 = 0x1000;
const TRANSITIONAL_DEVICE_ID_MAX: u16 = 0x103f;
const MODERN_DEVICE_ID_MIN: u16 = 0x1040;
const MODERN_DEVICE_ID_MAX: u16 = 0x107f;

pub(crate) struct VirtioPciDriver;

impl Driver<PciBus> for VirtioPciDriver {
    fn name(&self) -> &str {
        "virtio-pci"
    }

    fn match_data(&self) -> &[PciMatch] {
        &[PciMatch::Device {
            vendor: VIRTIO_PCI_VENDOR_ID,
            first: TRANSITIONAL_DEVICE_ID_MIN,
            last: MODERN_DEVICE_ID_MAX,
        }]
    }

    fn on_probe(&self, device: &Arc<PciDevice>) -> Result<()> {
        let has_vendor_cap = device.iter_vndr_capability().next().is_some();
        let device_id = device.device_id();
        let transport: Box<dyn VirtioTransport> = match device_id.device_id {
            TRANSITIONAL_DEVICE_ID_MIN..=TRANSITIONAL_DEVICE_ID_MAX
                if device_id.revision_id == 0 =>
            {
                if has_vendor_cap {
                    let modern = VirtioPciModernTransport::new(device.clone())
                        .map_err(|_| Error::ProbeFailed)?;
                    Box::new(modern)
                } else {
                    let legacy = VirtioPciLegacyTransport::new(device.clone())
                        .map_err(|_| Error::ProbeFailed)?;
                    Box::new(legacy)
                }
            }
            MODERN_DEVICE_ID_MIN..=MODERN_DEVICE_ID_MAX => {
                if !has_vendor_cap {
                    return Err(Error::NoDriver);
                }
                let modern = VirtioPciModernTransport::new(device.clone())
                    .map_err(|_| Error::ProbeFailed)?;
                Box::new(modern)
            }
            _ => return Err(Error::NoDriver),
        };
        super::push_device_transport(transport);
        Ok(())
    }

    fn on_release(&self, _device: &Arc<PciDevice>) {
        // FIXME: Stop the virtio device and release its transport resources before removal.
        panic!("virtio-pci driver removal is not supported");
    }

    fn dev_attrs(&self) -> &'static [Attr<PciDevice>] {
        &[]
    }

    fn suppress_bind_attrs(&self) -> bool {
        true
    }
}
