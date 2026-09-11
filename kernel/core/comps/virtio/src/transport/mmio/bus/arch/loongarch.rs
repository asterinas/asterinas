// SPDX-License-Identifier: MPL-2.0

// TODO: Add `MappedIrqLine` support for LoongArch.
pub(super) use ostd::irq::IrqLine as MappedIrqLine;

use crate::transport::mmio::bus::{MmioBus, common_device::MmioCommonDevice};

pub(super) fn probe_for_device() {
    // TODO: Probe virtio devices on the MMIO bus in LoongArch.
    let _ = super::validate_mmio_device;
    let _ = MmioCommonDevice::new;
    let _ = MmioBus::register_mmio_device;
}
