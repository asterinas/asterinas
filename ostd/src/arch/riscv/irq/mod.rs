// SPDX-License-Identifier: MPL-2.0

//! Interrupts.

mod ipi;
mod ops;
mod remapping;

pub(crate) use ipi::{send_ipi, HwCpuId};
pub(crate) use ops::{disable_local, enable_local, enable_local_and_halt, is_local_enabled};
pub(crate) use remapping::IrqRemapping;

pub(crate) const IRQ_NUM_MIN: u8 = 0;
pub(crate) const IRQ_NUM_MAX: u8 = 255;
