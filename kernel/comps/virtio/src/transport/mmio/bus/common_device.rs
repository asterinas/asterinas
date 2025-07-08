// SPDX-License-Identifier: MPL-2.0

//! MMIO device common definitions or functions.

use int_to_c_enum::TryFromInt;
use log::info;
#[cfg(target_arch = "x86_64")]
use ostd::arch::kernel::MappedIrqLine;
#[cfg(target_arch = "riscv64")] // TODO: Add `MappedIrqLine` support for RISC-V.
use ostd::trap::irq::IrqLine as MappedIrqLine;
#[cfg(target_arch = "loongarch64")] // TODO: Add `MappedIrqLine` support for Loongarch.
use ostd::trap::irq::IrqLine as MappedIrqLine;
use ostd::{io::IoMem, mm::VmIoOnce, trap::irq::IrqLine, Error, Result};

/// A MMIO common device.
#[derive(Debug)]
pub struct MmioCommonDevice {
    io_mem: IoMem,
    irq: MappedIrqLine,
}

impl MmioCommonDevice {
    pub(super) fn new(io_mem: IoMem, irq: MappedIrqLine) -> Self {
        debug_assert!(mmio_check_magic(&io_mem));

        let this = Self { io_mem, irq };
        info!(
            "[Virtio]: Found MMIO device at {:#x}, device ID {}, IRQ number {}",
            this.io_mem.paddr(),
            this.read_device_id().unwrap(),
            this.irq.num(),
        );

        this
    }

    /// Returns a reference to the I/O memory.
    pub fn io_mem(&self) -> &IoMem {
        &self.io_mem
    }

    /// Reads the device ID from the I/O memory.
    pub fn read_device_id(&self) -> Result<u32> {
        mmio_read_device_id(&self.io_mem)
    }

    /// Reads the version number from the I/O memory.
    pub fn read_version(&self) -> Result<VirtioMmioVersion> {
        VirtioMmioVersion::try_from(mmio_read_version(&self.io_mem)?)
            .map_err(|_| Error::InvalidArgs)
    }

    /// Returns an immutable reference to the IRQ line.
    pub fn irq(&self) -> &IrqLine {
        &self.irq
    }
}

/// Virtio MMIO version.
#[derive(Debug, Clone, Copy, TryFromInt, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u32)]
pub enum VirtioMmioVersion {
    /// Legacy
    Legacy = 1,
    /// Modern
    Modern = 2,
}

const OFFSET_TO_MAGIC: usize = 0;
const OFFSET_TO_VERSION: usize = 4;
const OFFSET_TO_DEVICE_ID: usize = 8;

pub(super) fn mmio_check_magic(io_mem: &IoMem) -> bool {
    const MAGIC_VALUE: u32 = 0x74726976;
    io_mem
        .read_once::<u32>(OFFSET_TO_MAGIC)
        .is_ok_and(|val| val == MAGIC_VALUE)
}
fn mmio_read_version(io_mem: &IoMem) -> Result<u32> {
    io_mem.read_once(OFFSET_TO_VERSION)
}
pub(super) fn mmio_read_device_id(io_mem: &IoMem) -> Result<u32> {
    io_mem.read_once(OFFSET_TO_DEVICE_ID)
}
