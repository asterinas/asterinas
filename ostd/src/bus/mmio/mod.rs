// SPDX-License-Identifier: MPL-2.0

#![allow(dead_code)]

//! Virtio over MMIO

use crate::{bus::mmio::bus::MmioBus, sync::SpinLock};

pub mod bus;
pub mod common_device;

pub(crate) const VIRTIO_MMIO_MAGIC: u32 = 0x74726976;

/// MMIO bus instance
pub static MMIO_BUS: SpinLock<MmioBus> = SpinLock::new(MmioBus::new());

use crate::arch;

pub(crate) fn init() {
    arch::bus::mmio::init();
}
