use int_to_c_enum::TryFromInt;
use log::info;

use crate::{
    io_mem::IoMem,
    trap::IrqLine,
    vm::{paddr_to_vaddr, Paddr, VmIo},
};

use super::VIRTIO_MMIO_MAGIC;

/// MMIO Common device.
/// TODO: Implement universal access to MMIO devices since we are temporarily
/// using specific virtio device as implementation of CommonDevice.
#[derive(Debug)]
pub struct MmioCommonDevice {
    io_mem: IoMem,
    irq: IrqLine,
}

impl MmioCommonDevice {
    pub(super) fn new(paddr: Paddr, handle: IrqLine) -> Self {
        // Read magic value
        // Safety: It only read the value and judge if the magic value fit 0x74726976
        unsafe {
            debug_assert_eq!(*(paddr_to_vaddr(paddr) as *const u32), VIRTIO_MMIO_MAGIC);
        }
        // Safety: This range is virtio-mmio device space.
        let io_mem = unsafe { IoMem::new(paddr..paddr + 0x200) };
        let res = Self {
            io_mem,
            irq: handle,
        };
        info!(
            "[Virtio]: Found Virtio mmio device, device id:{:?}, irq number:{:?}",
            res.device_id(),
            res.irq.num()
        );
        res
    }

    pub fn address(&self) -> Paddr {
        self.io_mem.paddr()
    }

    pub fn io_mem(&self) -> &IoMem {
        &self.io_mem
    }

    pub fn device_id(&self) -> u32 {
        self.io_mem.read_val::<u32>(8).unwrap()
    }

    pub fn version(&self) -> VirtioMmioVersion {
        VirtioMmioVersion::try_from(self.io_mem.read_val::<u32>(4).unwrap()).unwrap()
    }

    pub fn irq(&self) -> &IrqLine {
        &self.irq
    }

    pub fn irq_mut(&mut self) -> &mut IrqLine {
        &mut self.irq
    }
}

#[derive(Debug, Clone, Copy, TryFromInt, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u32)]
pub enum VirtioMmioVersion {
    Legacy = 1,
    Modern = 2,
}
