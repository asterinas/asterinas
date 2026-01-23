// SPDX-License-Identifier: MPL-2.0

//! PCI device common definitions or functions.

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

/// PCI common device.
///
/// This type contains a range of information and functions common to PCI devices.
#[derive(Debug)]
pub struct PciCommonDevice {
    device_id: PciDeviceId,
    location: PciDeviceLocation,
    header_type: PciHeaderType,
    bar_manager: BarManager,
    capabilities: Vec<Capability>,
}

impl PciCommonDevice {
    /// Returns the PCI device ID.
    pub fn device_id(&self) -> &PciDeviceId {
        &self.device_id
    }

    /// Returns the PCI device location.
    pub fn location(&self) -> &PciDeviceLocation {
        &self.location
    }

    /// Returns the PCI Base Address Register (BAR) manager.
    pub fn bar_manager(&self) -> &BarManager {
        &self.bar_manager
    }

    /// Returns the PCI capabilities.
    pub fn capabilities(&self) -> &Vec<Capability> {
        &self.capabilities
    }

    /// Returns the PCI device type.
    pub fn device_type(&self) -> PciDeviceType {
        self.header_type.device_type()
    }

    /// Checks whether the device is a multi-function device.
    pub fn has_multi_funcs(&self) -> bool {
        self.header_type.has_multi_funcs()
    }

    /// Reads the PCI command.
    pub fn read_command(&self) -> Command {
        Command::from_bits_truncate(self.location.read16(PciCommonCfgOffset::Command as u16))
    }

    /// Writes the PCI command.
    pub fn write_command(&self, command: Command) {
        self.location
            .write16(PciCommonCfgOffset::Command as u16, command.bits())
    }

    /// Reads the PCI status.
    pub fn read_status(&self) -> Status {
        Status::from_bits_truncate(self.location.read16(PciCommonCfgOffset::Status as u16))
    }

    pub(super) fn new(location: PciDeviceLocation) -> Option<Self> {
        if location.read16(0) == 0xFFFF {
            // No device.
            return None;
        }

        let capabilities = Vec::new();
        let device_id = PciDeviceId::new(location);
        let bar_manager = BarManager {
            bars: [const { None }; 6],
        };
        let mut header_type =
            PciHeaderType::try_from_raw(location.read8(PciCommonCfgOffset::HeaderType as u16))?;

        if let PciDeviceType::PciToPciBridge(primary_bus, secondary_bus, subordinate_bus) =
            &mut header_type.device_type
        {
            *primary_bus = location.read8(PciBridgeCfgOffset::PrimaryBusNumber as u16);
            *secondary_bus = location.read8(PciBridgeCfgOffset::SecondaryBusNumber as u16);
            *subordinate_bus = location.read8(PciBridgeCfgOffset::SubordinateBusNumber as u16);
        }

        let mut device = Self {
            device_id,
            location,
            header_type,
            bar_manager,
            capabilities,
        };

        // While setting up the BARs, we need to ensure that
        // "Decode (I/O or memory) of the appropriate address space is disabled via the Command
        // Register before sizing a Base Address register."
        let command_val = device.read_command() | Command::BUS_MASTER;
        device.write_command(command_val - (Command::MEMORY_SPACE | Command::IO_SPACE));
        device.bar_manager = BarManager::new(device.header_type.device_type(), location);
        device.write_command(command_val | (Command::MEMORY_SPACE | Command::IO_SPACE));

        device.capabilities = Capability::device_capabilities(&mut device);

        Some(device)
    }
}

/// The header type field of a PCI device struct in the PCI configuration space.
///
/// A header type is comprised of two pieces of information:
/// 1. The device type ([`PciDeviceType`]);
/// 2. Whether the device has multiple functions (`bool`).
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
struct PciHeaderType {
    device_type: PciDeviceType,
    has_multi_funcs: bool,
}

impl PciHeaderType {
    /// Converts a byte into a header type.
    ///
    /// According to the PCI specification, the encoding of a header type is as follows:
    /// - Bit 0-6 encodes the raw value of `PciDeviceType`;
    /// - Bit 7 indicates whether the PCI device has multiple functions.
    pub fn try_from_raw(raw: u8) -> Option<Self> {
        let device_type = PciDeviceType::try_from_raw(raw & 0x7F)?;
        let has_multi_funcs = (raw & 0x80) != 0;

        Some(Self {
            device_type,
            has_multi_funcs,
        })
    }

    /// Returns the device type.
    pub fn device_type(self) -> PciDeviceType {
        self.device_type
    }

    /// Returns whether the device has multiple functions.
    pub fn has_multi_funcs(self) -> bool {
        self.has_multi_funcs
    }
}

/// Represents the type of PCI device, determined by the device's header type.
///
/// Used to distinguish between general devices, PCI-to-PCI bridges, and PCI-to-Cardbus bridges.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
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
    /// Converts a raw header type value into a `PciDeviceType`.
    pub fn try_from_raw(raw: u8) -> Option<Self> {
        match raw {
            0x00 => Some(PciDeviceType::GeneralDevice),
            0x01 => Some(PciDeviceType::PciToPciBridge(0, 0, 0)),
            0x02 => Some(PciDeviceType::PciToCardbusBridge),
            _ => None,
        }
    }
}

/// Base Address Register (BAR) manager.
#[derive(Debug)]
pub struct BarManager {
    /// There are at most 6 BARs in PCI device.
    bars: [Option<Bar>; 6],
}

impl BarManager {
    /// Gains access to the BAR space and returns None if that BAR is absent.
    pub fn bar(&self, idx: u8) -> &Option<Bar> {
        &self.bars[idx as usize]
    }

    /// Parses the BAR space by PCI device location.
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
