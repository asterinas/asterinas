// SPDX-License-Identifier: MPL-2.0

// TODO: Add `MappedIrqLine` support for LoongArch.
pub(super) use ostd::irq::IrqLine as MappedIrqLine;

pub(super) fn probe_for_device() {
    // TODO: Probe virtio devices on the MMIO bus in LoongArch.
    // Then, register them by calling `super::try_register_mmio_device`.
}
