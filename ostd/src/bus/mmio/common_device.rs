// SPDX-License-Identifier: MPL-2.0

//! MMIO device common definitions or functions.

use int_to_c_enum::TryFromInt;
use log::info;

use super::VIRTIO_MMIO_MAGIC;
use crate::{
    io_mem::IoMem,
    mm::{
        paddr_to_vaddr,
        page_prop::{CachePolicy, PageFlags},
        Paddr, VmIoOnce,
    },
    trap::IrqLine,
    Error, Result,
};

/// MMIO Common device.
#[derive(Debug)]
pub struct MmioCommonDevice {
    io_mem: IoMem,
    irq: IrqLine,
}

impl MmioCommonDevice {
    pub(super) fn new(paddr: Paddr, handle: IrqLine) -> Self {
        // TODO: Implement universal access to MMIO devices since we are temporarily
        // using specific virtio device as implementation of CommonDevice.

        // Read magic value
        // SAFETY: It only read the value and judge if the magic value fit 0x74726976
        unsafe {
            debug_assert_eq!(*(paddr_to_vaddr(paddr) as *const u32), VIRTIO_MMIO_MAGIC);
        }
        // SAFETY: This range is virtio-mmio device space.
        let io_mem = unsafe {
            IoMem::new(
                paddr..paddr + 0x200,
                PageFlags::RW,
                CachePolicy::Uncacheable,
            )
        };
        let res = Self {
            io_mem,
            irq: handle,
        };
        info!(
            "[Virtio]: Found Virtio mmio device, device id:{:?}, irq number:{:?}",
            res.read_device_id().unwrap(),
            res.irq.num()
        );
        res
    }

    /// Base address
    pub fn address(&self) -> Paddr {
        self.io_mem.paddr()
    }

    /// Grants access to the MMIO
    pub fn io_mem(&self) -> &IoMem {
        &self.io_mem
    }

    /// Device ID
    pub fn read_device_id(&self) -> Result<u32> {
        self.io_mem.read_once::<u32>(8)
    }

    /// Version of the MMIO device.
    pub fn read_version(&self) -> Result<VirtioMmioVersion> {
        VirtioMmioVersion::try_from(self.io_mem.read_once::<u32>(4)?)
            .map_err(|_| Error::InvalidArgs)
    }

    /// Interrupt line
    pub fn irq(&self) -> &IrqLine {
        &self.irq
    }

    /// Mutable Interrupt line
    pub fn irq_mut(&mut self) -> &mut IrqLine {
        &mut self.irq
    }
}

/// Virtio MMIO version
#[derive(Debug, Clone, Copy, TryFromInt, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u32)]
pub enum VirtioMmioVersion {
    /// Legacy
    Legacy = 1,
    /// Modern
    Modern = 2,
}
