// SPDX-License-Identifier: MPL-2.0

//! PCI device common definitions or functions.

#![expect(dead_code)]

use alloc::vec::Vec;

use super::{
    capability::Capability,
    cfg_space::{access::PciDeviceLocation, AddrLen, Bar, Command, Status},
    device_info::PciDeviceInfo,
};

/// PCI common device, Contains a range of information and functions common to PCI devices.
#[derive(Debug)]
pub struct PciCommonDevice {
    device_info: PciDeviceInfo,
    location: PciDeviceLocation,
    bar_manager: BarManager,
    capabilities: Vec<Capability>,
}

impl PciCommonDevice {
    /// PCI device information
    pub fn device_info(&self) -> &PciDeviceInfo {
        &self.device_info
    }

    /// PCI device location
    pub(crate) fn location(&self) -> &PciDeviceLocation {
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

    /// Gets the PCI Command
    pub fn command(&self) -> Command {
        Command::from_bits_truncate(self.location.read_command().unwrap())
    }

    /// Sets the PCI Command
    pub fn set_command(&self, command: Command) {
        let _ = self.location.write_command(command.bits());
    }

    /// Gets the PCI status
    pub fn status(&self) -> Status {
        Status::from_bits_truncate(self.location.read_status().unwrap())
    }

    pub(super) fn new(mut location: PciDeviceLocation) -> Option<Self> {
        location.acquire_io_mem().ok()?;
        if location.read_vendor_id().ok()? == 0xFFFF {
            // not exists
            return None;
        }

        let capabilities = Vec::new();
        let device_info = PciDeviceInfo::new(&location);
        let bar_manager = BarManager::new(&location);
        let mut device = Self {
            device_info,
            location,
            bar_manager,
            capabilities,
        };
        device.capabilities = Capability::device_capabilities(&device);
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
    fn new(location: &PciDeviceLocation) -> Self {
        let header_type = location.read_header_type().unwrap() & !(1 << 7);
        // Get the max bar amount, header type=0 => end device; header type=1 => PCI bridge.
        let max = match header_type {
            0 => 6,
            1 => 2,
            _ => 0,
        };
        let mut idx = 0;
        let mut bars = [None, None, None, None, None, None];
        while idx < max {
            if let Ok(bar) = Bar::new(location, idx) {
                let mut idx_step = 0;
                match &bar {
                    Bar::Memory(memory_bar) => {
                        if memory_bar.address_length() == AddrLen::Bits64 {
                            idx_step = 1;
                        }
                    }
                    Bar::Io(_) => {}
                }
                bars[idx as usize] = Some(bar);
                idx += idx_step;
            }
            idx += 1;
        }
        Self { bars }
    }
}
