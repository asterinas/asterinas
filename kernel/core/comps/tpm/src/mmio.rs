// SPDX-License-Identifier: MPL-2.0

use core::ops::Range;

use ostd::{
    io::IoMem,
    mm::{Paddr, VmIoOnce},
    task::Task,
    timer::Jiffies,
};
use tpm_core::{phy::TisPhy, tis::TisErr};

pub(super) struct TisMmio {
    // Register offsets supplied by tpm-core already include the locality window.
    mmio: IoMem,
}
impl TisMmio {
    /// Requires a register window covering all five localities (0x5000 bytes).
    pub(super) fn acquire(phys: Range<Paddr>) -> Result<Self, TisErr> {
        match IoMem::acquire(phys) {
            Ok(mmio) => Ok(TisMmio { mmio }),
            Err(_) => Err(TisErr::Phy),
        }
    }
}
impl TisPhy for TisMmio {
    fn read8(&mut self, addr: u32) -> Result<u8, TisErr> {
        self.mmio
            .read_once::<u8>(addr as usize)
            .map_err(|_| TisErr::Phy)
    }
    fn read32(&mut self, addr: u32) -> Result<u32, TisErr> {
        self.mmio
            .read_once::<u32>(addr as usize)
            .map_err(|_| TisErr::Phy)
    }
    fn write8(&mut self, addr: u32, value: u8) -> Result<(), TisErr> {
        self.mmio
            .write_once::<u8>(addr as usize, &value)
            .map_err(|_| TisErr::Phy)
    }
    fn write32(&mut self, addr: u32, value: u32) -> Result<(), TisErr> {
        self.mmio
            .write_once::<u32>(addr as usize, &value)
            .map_err(|_| TisErr::Phy)
    }
    fn read_fifo(&mut self, addr: u32, out: &mut [u8], off: usize, n: usize) -> Result<(), TisErr> {
        // Each read of the data-FIFO register consumes one byte, so the same
        // address is read `n` times.
        for slot in &mut out[off..off + n] {
            *slot = self
                .mmio
                .read_once::<u8>(addr as usize)
                .map_err(|_| TisErr::Phy)?;
        }
        Ok(())
    }
    fn write_fifo(&mut self, addr: u32, data: &[u8], off: usize, n: usize) -> Result<(), TisErr> {
        for byte in &data[off..off + n] {
            self.mmio
                .write_once::<u8>(addr as usize, byte)
                .map_err(|_| TisErr::Phy)?;
        }
        Ok(())
    }
    // The transport trait has no error channel for FIFO reset.
    fn reset_fifo(&mut self, addr: u32) {
        let _ = self.mmio.write_once::<u8>(addr as usize, &0x40u8);
    }
    // TIS polling budgets assume millisecond delays, so wait for a timer tick.
    fn delay(&mut self) {
        if Task::current().is_some() {
            let target = Jiffies::elapsed().as_u64().saturating_add(1);
            loop {
                Task::yield_now();
                if Jiffies::elapsed().as_u64() >= target {
                    break;
                }
            }
        } else {
            // Early probing runs before a task context is available.
            core::hint::spin_loop();
        }
    }
}
