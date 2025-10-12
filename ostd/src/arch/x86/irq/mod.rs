// SPDX-License-Identifier: MPL-2.0

//! Interrupts.

pub(super) mod chip;
mod ipi;
mod ops;
mod remapping;

pub use chip::{IrqChip, MappedIrqLine, IRQ_CHIP};
pub(crate) use ipi::{send_ipi, HwCpuId};
pub(crate) use ops::{disable_local, enable_local, enable_local_and_halt, is_local_enabled};
pub(crate) use remapping::IrqRemapping;

// Intel(R) 64 and IA-32 rchitectures Software Developer's Manual,
// Volume 3A, Section 6.2 says "Vector numbers in the range 32 to 255
// are designated as user-defined interrupts and are not reserved by
// the Intel 64 and IA-32 architecture."
pub(crate) const IRQ_NUM_MIN: u8 = 32;
pub(crate) const IRQ_NUM_MAX: u8 = 255;
