// SPDX-License-Identifier: MPL-2.0

//! PCI device common definitions or functions.

#![allow(dead_code)]
#![allow(unused_variables)]

use alloc::vec::Vec;

use super::{
    capability::Capability,
    cfg_space::{AddrLen, Bar, Command, PciDeviceCommonCfgOffset, Status},
    device_info::{PciDeviceId, PciDeviceLocation},
};

/// PCI common device, Contains a range of information and functions common to PCI devices.
#[derive(Debug)]
pub struct PciCommonDevice {
    device_id: PciDeviceId,
    location: PciDeviceLocation,
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

    /// Gets the PCI Command
    pub fn command(&self) -> Command {
        Command::from_bits_truncate(
            self.location
                .read16(PciDeviceCommonCfgOffset::Command as u16),
        )
    }

    /// Sets the PCI Command
    pub fn set_command(&self, command: Command) {
        self.location
            .write16(PciDeviceCommonCfgOffset::Command as u16, command.bits())
    }

    /// Gets the PCI status
    pub fn status(&self) -> Status {
        Status::from_bits_truncate(
            self.location
                .read16(PciDeviceCommonCfgOffset::Status as u16),
        )
    }

    pub(super) fn new(location: PciDeviceLocation) -> Option<Self> {
        if location.read16(0) == 0xFFFF {
            // not exists
            return None;
        }

        let capabilities = Vec::new();
        let device_id = PciDeviceId::new(location);
        let bar_manager = BarManager::new(location);
        let mut device = Self {
            device_id,
            location,
            bar_manager,
            capabilities,
        };
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
    /// BARs, the bool indicate whether this bar should exposed to unprivileged part.
    bars: [Option<(Bar, bool)>; 6],
}

impl BarManager {
    /// Gain access to the BAR space and return None if that BAR is set to be invisible or absent.
    pub fn bar(&self, idx: u8) -> Option<Bar> {
        let (bar, visible) = self.bars[idx as usize].clone()?;
        if visible {
            Some(bar)
        } else {
            None
        }
    }

    /// Parse the BAR space by PCI device location.
    fn new(location: PciDeviceLocation) -> Self {
        let header_type = location.read8(PciDeviceCommonCfgOffset::HeaderType as u16) & !(1 << 7);
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
                bars[idx as usize] = Some((bar, true));
                idx += idx_step;
            }
            idx += 1;
        }
        Self { bars }
    }

    pub(super) fn set_invisible(&mut self, idx: u8) {
        if self.bars[idx as usize].is_some() {
            let Some((bar, _)) = self.bars[idx as usize].clone() else {
                return;
            };
            self.bars[idx as usize] = Some((bar, false));
        }
        let Some((_, visible)) = self.bars[idx as usize] else {
            return;
        };
    }

    /// Gain access to the BAR space and return None if that BAR is absent.
    pub(super) fn bar_space_without_invisible(&self, idx: u8) -> Option<Bar> {
        if let Some((bar, _)) = self.bars[idx as usize].clone() {
            Some(bar)
        } else {
            None
        }
    }
}
