// SPDX-License-Identifier: MPL-2.0

//! PCI device common definitions or functions.

#![expect(dead_code)]

use alloc::vec::Vec;

use super::{
    capability::Capability,
    cfg_space::{AddrLen, Bar, Command, Status},
    device_info::PciDeviceId,
};
use crate::{
    cfg_space::{PciBridgeCfgOffset, PciCommonCfgOffset},
    device_info::PciDeviceLocation,
};

/// Represents the type of PCI device, determined by the device's header type.
/// Used to distinguish between general devices, PCI-to-PCI bridges, and PCI-to-Cardbus bridges.
#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum PciDeviceType {
    /// General PCI device (header type 0x00).
    GeneralDevice,
    /// PCI-to-PCI bridge (header type 0x01).
    /// Contains the primary, secondary, and subordinate bus numbers.
    PciToPciBridge(u8, u8, u8),
    /// PCI-to-Cardbus bridge (header type 0x02).
    PciToCardbusBridge,
}

impl PciDeviceType {
    /// Creates a new `PciDeviceType` based on the header type read from the PCI device's configuration space.
    pub fn new(location: &PciDeviceLocation) -> Option<Self> {
        const HEADER_TYPE_MASK: u8 = 0x7F;

        let header_type = location.read8(PciCommonCfgOffset::HeaderType as u16) & HEADER_TYPE_MASK;

        let device_type = match header_type {
            0x00 => PciDeviceType::GeneralDevice,
            0x01 => {
                let primary_bus = location.read8(PciBridgeCfgOffset::PrimaryBusNumber as u16);
                let secondary_bus = location.read8(PciBridgeCfgOffset::SecondaryBusNumber as u16);
                let subordinate_bus =
                    location.read8(PciBridgeCfgOffset::SubordinateBusNumber as u16);
                PciDeviceType::PciToPciBridge(primary_bus, secondary_bus, subordinate_bus)
            }
            0x02 => PciDeviceType::PciToCardbusBridge,
            _ => return None,
        };

        Some(device_type)
    }
}

/// PCI common device, Contains a range of information and functions common to PCI devices.
#[derive(Debug)]
pub struct PciCommonDevice {
    device_id: PciDeviceId,
    location: PciDeviceLocation,
    device_type: PciDeviceType,
    bar_manager: BarManager,
    capabilities: Vec<Capability>,
}

impl PciCommonDevice {
    /// PCI device ID
    pub fn device_id(&self) -> &PciDeviceId {
        &self.device_id
    }

    /// PCI device location
    pub fn location(&self) -> &PciDeviceLocation {
        &self.location
    }

    /// PCI Base Address Register (BAR) manager
    pub fn bar_manager(&self) -> &BarManager {
        &self.bar_manager
    }

    /// PCI capabilities
    pub fn capabilities(&self) -> &Vec<Capability> {
        &self.capabilities
    }

    /// PCI device type
    pub fn device_type(&self) -> PciDeviceType {
        self.device_type
    }

    /// Checks whether the device is a multi-function device
    pub fn has_multi_function(&self) -> bool {
        (self.location.read8(PciCommonCfgOffset::HeaderType as u16) & 0x80) != 0
    }

    /// Gets the PCI Command
    pub fn command(&self) -> Command {
        Command::from_bits_truncate(self.location.read16(PciCommonCfgOffset::Command as u16))
    }

    /// Sets the PCI Command
    pub fn set_command(&self, command: Command) {
        self.location
            .write16(PciCommonCfgOffset::Command as u16, command.bits())
    }

    /// Gets the PCI status
    pub fn status(&self) -> Status {
        Status::from_bits_truncate(self.location.read16(PciCommonCfgOffset::Status as u16))
    }

    pub(super) fn new(location: PciDeviceLocation) -> Option<Self> {
        if location.read16(0) == 0xFFFF {
            // not exists
            return None;
        }

        let capabilities = Vec::new();
        let device_id = PciDeviceId::new(location);
        let bar_manager = BarManager {
            bars: [const { None }; 6],
        };
        let device_type = PciDeviceType::new(&location)?;
        let mut device = Self {
            device_id,
            location,
            device_type,
            bar_manager,
            capabilities,
        };

        device.set_command(
            device.command() | Command::MEMORY_SPACE | Command::BUS_MASTER | Command::IO_SPACE,
        );
        device.bar_manager = BarManager::new(device_type, location);
        device.capabilities = Capability::device_capabilities(&mut device);

        Some(device)
    }

    pub(super) fn bar_manager_mut(&mut self) -> &mut BarManager {
        &mut self.bar_manager
    }

    pub(super) fn capabilities_mut(&mut self) -> &mut Vec<Capability> {
        &mut self.capabilities
    }
}

/// Base Address Registers manager.
#[derive(Debug)]
pub struct BarManager {
    /// There are at most 6 BARs in PCI device.
    bars: [Option<Bar>; 6],
}

impl BarManager {
    /// Gain access to the BAR space and return None if that BAR is absent.
    pub fn bar(&self, idx: u8) -> &Option<Bar> {
        &self.bars[idx as usize]
    }

    /// Parse the BAR space by PCI device location.
    fn new(device_type: PciDeviceType, location: PciDeviceLocation) -> Self {
        let mut bars = [None, None, None, None, None, None];

        // Determine the maximum number of BARs based on the device type.
        let max = match device_type {
            PciDeviceType::GeneralDevice => 6,
            PciDeviceType::PciToPciBridge(_, _, _) => 2,
            PciDeviceType::PciToCardbusBridge => 0,
        };
        let mut idx = 0;
        while idx < max {
            let mut idx_step = 1;
            let Ok(bar) = Bar::new(location, idx) else {
                idx += idx_step;
                continue;
            };

            if let Bar::Memory(memory_bar) = &bar
                && memory_bar.address_length() == AddrLen::Bits64
            {
                // 64-bit BAR occupies two BAR slots.
                idx_step += 1;
            }

            bars[idx as usize] = Some(bar);
            idx += idx_step;
        }

        Self { bars }
    }
}
