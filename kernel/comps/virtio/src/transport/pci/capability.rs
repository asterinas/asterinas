// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use ostd::bus::pci::{
    capability::vendor::CapabilityVndrData,
    cfg_space::{Bar, IoBar, MemoryBar},
    common_device::BarManager,
};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
#[allow(clippy::enum_variant_names)]
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
    memory_bar: Option<Arc<MemoryBar>>,
    io_bar: Option<Arc<IoBar>>,
}

impl VirtioPciCapabilityData {
    pub fn memory_bar(&self) -> &Option<Arc<MemoryBar>> {
        &self.memory_bar
    }

    pub fn io_bar(&self) -> &Option<Arc<IoBar>> {
        &self.io_bar
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

    pub(super) fn new(bar_manager: &BarManager, vendor_cap: CapabilityVndrData) -> Self {
        let cfg_type = vendor_cap.read8(3).unwrap();
        let cfg_type = match cfg_type {
            1 => VirtioPciCpabilityType::CommonCfg,
            2 => VirtioPciCpabilityType::NotifyCfg,
            3 => VirtioPciCpabilityType::IsrCfg,
            4 => VirtioPciCpabilityType::DeviceCfg,
            5 => VirtioPciCpabilityType::PciCfg,
            _ => panic!("Unsupported virtio capability type:{:?}", cfg_type),
        };
        let bar = vendor_cap.read8(4).unwrap();
        let capability_length = vendor_cap.read8(2).unwrap();
        let offset = vendor_cap.read32(8).unwrap();
        let length = vendor_cap.read32(12).unwrap();
        let option = if capability_length > 0x10 {
            Some(vendor_cap.read32(16).unwrap())
        } else {
            None
        };

        let mut io_bar = None;
        let mut memory_bar = None;
        if let Some(bar) = bar_manager.bar(bar) {
            match bar {
                Bar::Memory(memory) => {
                    memory_bar = Some(memory);
                }
                Bar::Io(io) => {
                    io_bar = Some(io);
                }
            }
        };
        Self {
            cfg_type,
            offset,
            length,
            option,
            memory_bar,
            io_bar,
        }
    }
}
