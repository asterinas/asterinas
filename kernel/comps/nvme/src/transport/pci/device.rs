// SPDX-License-Identifier: MPL-2.0

use core::{fmt::Debug, hint::spin_loop};

use log::info;
use ostd::bus::{
    BusProbeError,
    pci::{PciDeviceId, bus::PciDevice, cfg_space::Bar, common_device::PciCommonDevice},
};

use crate::{nvme_regs::*, transport::NVMeTransportError};

const NVME_CC_ENABLE: u32 = 0x1;
const NVME_CSTS_RDY: u32 = 0x1;

#[derive(Debug)]
pub struct NVMePciDevice {
    device_id: PciDeviceId,
}

impl NVMePciDevice {
    pub(super) fn new(device_id: PciDeviceId) -> Self {
        Self { device_id }
    }
}

impl PciDevice for NVMePciDevice {
    fn device_id(&self) -> PciDeviceId {
        self.device_id
    }
}

pub struct NVMePciTransport {
    pub common_device: PciCommonDevice,
    pub config_bar: Bar,
}

impl Debug for NVMePciTransport {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("NVMePciTransport")
            .field("common_device", &self.common_device)
            .finish()
    }
}

impl NVMePciTransport {
    #[allow(clippy::result_large_err)]
    pub(super) fn new(
        common_device: PciCommonDevice,
    ) -> Result<Self, (BusProbeError, PciCommonDevice)> {
        let config_bar = common_device.bar_manager().bar(0).clone().unwrap();
        Ok(Self {
            common_device,
            config_bar,
        })
    }

    fn read_u32(&self, offset: u32) -> u32 {
        self.config_bar
            .read_once(offset.try_into().unwrap())
            .unwrap()
    }

    fn write_u32(&self, offset: u32, val: u32) -> Result<(), NVMeTransportError> {
        self.config_bar
            .write_once(offset.try_into().unwrap(), val)
            .unwrap();
        Ok(())
    }

    pub fn read_reg32(&self, reg: NVMeRegs32) -> u32 {
        self.read_u32(reg as u32)
    }

    pub fn read_reg64(&self, reg: NVMeRegs64) -> u64 {
        let reg_low = self.read_u32(reg as u32);
        let reg_high = self.read_u32(reg as u32 + 0x04);
        let reg_high_shift: u64 = (reg_high as u64) << 32;
        reg_high_shift | reg_low as u64
    }

    pub fn write_reg32(&self, reg: NVMeRegs32, val: u32) -> Result<(), NVMeTransportError> {
        self.write_u32(reg as u32, val)
    }

    pub fn write_reg64(&self, reg: NVMeRegs64, val: u64) -> Result<(), NVMeTransportError> {
        let val_low = (val & 0xFFFFFFFF) as u32;
        let val_high = (val >> 32) as u32;
        self.write_u32(reg as u32, val_low)?;
        self.write_u32(reg as u32 + 0x04, val_high)
    }

    pub fn reset_controller(&self) {
        info!("[NVMe]: Resetting...");
        let mut cc = self.read_reg32(NVMeRegs32::Cc);
        cc &= !NVME_CC_ENABLE;
        let _ = self.write_reg32(NVMeRegs32::Cc, cc);

        info!("[NVMe]: Waiting for reset to be acked");
        loop {
            let csts = self.read_reg32(NVMeRegs32::Csts);
            if (csts & NVME_CSTS_RDY) == 0 {
                break;
            }
            spin_loop();
        }
    }

    pub fn enable_controller(&self) {
        info!("[NVMe]: Set enable bit");
        let mut cc = self.read_reg32(NVMeRegs32::Cc);
        cc |= NVME_CC_ENABLE;
        let _ = self.write_reg32(NVMeRegs32::Cc, cc);

        info!("[NVMe]: Waiting for controller to be ready");
        loop {
            let csts = self.read_reg32(NVMeRegs32::Csts);
            if (csts & NVME_CSTS_RDY) == 1 {
                break;
            }
            spin_loop();
        }
    }

    pub fn set_entry_size(&self) {
        const IOSQES_BITS: u32 = 20;
        const IOSQES_VALUE: u32 = 4;
        const IOCQES_BITS: u32 = 16;
        const IOCQES_VALUE: u32 = 6;

        let mut cc = self.read_reg32(NVMeRegs32::Cc);
        cc = cc | (IOSQES_VALUE << IOSQES_BITS) | (IOCQES_VALUE << IOCQES_BITS);
        let _ = self.write_reg32(NVMeRegs32::Cc, cc);
    }
}
