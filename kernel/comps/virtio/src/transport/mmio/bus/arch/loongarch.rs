// SPDX-License-Identifier: MPL-2.0

// TODO: Add `MappedIrqLine` support for Loongarch.
pub(super) use ostd::trap::irq::IrqLine as MappedIrqLine;

pub(super) fn probe_for_device() {
    unimplemented!()
}
