// SPDX-License-Identifier: MPL-2.0

use aster_pci::{
    capability::vendor::CapabilityVndrData, cfg_space::BarAccess, common_device::BarManager,
};
use log::warn;
use ostd::io::IoMem;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
#[expect(clippy::enum_variant_names)]
pub enum VirtioPciCpabilityType {
    CommonCfg = 1,
    NotifyCfg = 2,
    IsrCfg = 3,
    DeviceCfg = 4,
    PciCfg = 5,
}

#[derive(Debug, Clone)]
pub struct VirtioPciCapabilityData {
    cfg_type: VirtioPciCpabilityType,
    offset: u32,
    length: u32,
    option: Option<u32>,
    memory_bar: Option<IoMem>,
}

impl VirtioPciCapabilityData {
    pub fn memory_bar(&self) -> Option<&IoMem> {
        self.memory_bar.as_ref()
    }

    pub fn offset(&self) -> u32 {
        self.offset
    }

    pub fn length(&self) -> u32 {
        self.length
    }

    pub fn typ(&self) -> VirtioPciCpabilityType {
        self.cfg_type.clone()
    }

    pub fn option_value(&self) -> Option<u32> {
        self.option
    }

    pub(super) fn new(bar_manager: &mut BarManager, vendor_cap: CapabilityVndrData) -> Self {
        let cfg_type = vendor_cap.read8(3).unwrap();
        let cfg_type = match cfg_type {
            1 => VirtioPciCpabilityType::CommonCfg,
            2 => VirtioPciCpabilityType::NotifyCfg,
            3 => VirtioPciCpabilityType::IsrCfg,
            4 => VirtioPciCpabilityType::DeviceCfg,
            5 => VirtioPciCpabilityType::PciCfg,
            _ => panic!("Unsupported virtio capability type:{:?}", cfg_type),
        };

        let offset = vendor_cap.read32(8).unwrap();
        let length = vendor_cap.read32(12).unwrap();

        let capability_length = vendor_cap.read8(2).unwrap();
        let option = if capability_length > 0x10 {
            Some(vendor_cap.read32(16).unwrap())
        } else {
            None
        };

        let bar = vendor_cap.read8(4).unwrap();
        let memory_bar = if let Some(bar) = bar_manager.bar_mut(bar) {
            match bar.acquire() {
                Ok(BarAccess::Memory(io_mem)) => Some(io_mem),
                Ok(BarAccess::Io) => {
                    warn!("I/O BAR is not supported");
                    None
                }
                Err(err) => {
                    warn!("BAR is not available: {:?}", err);
                    None
                }
            }
        } else {
            None
        };

        Self {
            cfg_type,
            offset,
            length,
            option,
            memory_bar,
        }
    }
}
