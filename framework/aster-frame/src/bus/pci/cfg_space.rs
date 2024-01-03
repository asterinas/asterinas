// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;
use bitflags::bitflags;

use crate::{io_mem::IoMem, Error, Result};

use super::PciDeviceLocation;

#[repr(u16)]
pub enum PciDeviceCommonCfgOffset {
    VendorId = 0x00,
    DeviceId = 0x02,
    Command = 0x04,
    Status = 0x06,
    RevisionId = 0x08,
    ClassCode = 0x09,
    CacheLineSize = 0x0C,
    LatencyTimer = 0x0D,
    HeaderType = 0x0E,
    Bist = 0x0F,
    Bar0 = 0x10,
    Bar1 = 0x14,
    Bar2 = 0x18,
    Bar3 = 0x1C,
    Bar4 = 0x20,
    Bar5 = 0x24,
    CardbusCisPtr = 0x28,
    SubsystemVendorId = 0x2C,
    SubsystemId = 0x2E,
    XromBar = 0x30,
    CapabilitiesPointer = 0x34,
    InterruptLine = 0x3C,
    InterruptPin = 0x3D,
    MinGrant = 0x3E,
    MaxLatency = 0x3F,
}

bitflags! {
    /// PCI device common config space command register.
    pub struct Command: u16 {
        const IO_SPACE                  =  1 << 0;
        const MEMORY_SPACE              =  1 << 1;
        const BUS_MASTER                =  1 << 2;
        const SPECIAL_CYCLES            =  1 << 3;
        const MWI_ENABLE                =  1 << 4;
        const VGA_PALETTE_SNOOP         =  1 << 5;
        const PARITY_ERROR_RESPONSE     =  1 << 6;
        const STEPPING_CONTROL          =  1 << 7;
        const SERR_ENABLE               =  1 << 8;
        const FAST_BACK_TO_BACK_ENABLE  =  1 << 9;
        const INTERRUPT_DISABLE         =  1 << 10;
    }
}

bitflags! {
    /// PCI device common config space status register.
    pub struct Status: u16 {
        const INTERRUPT_STATUS          = 1 << 3;
        const CAPABILITIES_LIST         = 1 << 4;
        const MHZ66_CAPABLE             = 1 << 5;
        const FAST_BACK_TO_BACK_CAPABLE = 1 << 7;
        const MASTER_DATA_PARITY_ERROR  = 1 << 8;
        const DEVSEL_MEDIUM_TIMING      = 1 << 9;
        const DEVSEL_SLOW_TIMING        = 1 << 10;
        const SIGNALED_TARGET_ABORT     = 1 << 11;
        const RECEIVED_TARGET_ABORT     = 1 << 12;
        const RECEIVED_MASTER_ABORT     = 1 << 13;
        const SIGNALED_SYSTEM_ERROR     = 1 << 14;
        const DETECTED_PARITY_ERROR     = 1 << 15;
    }
}

/// BAR space in PCI common config space.
#[derive(Debug, Clone)]
pub enum Bar {
    Memory(Arc<MemoryBar>),
    Io(Arc<IoBar>),
}

impl Bar {
    pub(super) fn new(location: PciDeviceLocation, index: u8) -> Result<Self> {
        if index >= 6 {
            return Err(Error::InvalidArgs);
        }
        // Get the original value first, then write all 1 to the register to get the length
        let raw = location.read32(index as u16 * 4 + PciDeviceCommonCfgOffset::Bar0 as u16);
        if raw == 0 {
            // no BAR
            return Err(Error::InvalidArgs);
        }
        Ok(if raw & 1 == 0 {
            Self::Memory(Arc::new(MemoryBar::new(&location, index)?))
        } else {
            // IO BAR
            Self::Io(Arc::new(IoBar::new(&location, index)?))
        })
    }
}

#[derive(Debug, Clone)]
pub struct MemoryBar {
    base: u64,
    size: u32,
    /// Whether this bar is prefetchable, allowing the CPU to get the data
    /// in advance.
    prefetchable: bool,
    address_length: AddrLen,
    io_memory: IoMem,
}

impl MemoryBar {
    /// Memory BAR bits type
    pub fn address_length(&self) -> AddrLen {
        self.address_length
    }

    pub fn prefetchable(&self) -> bool {
        self.prefetchable
    }

    pub fn base(&self) -> u64 {
        self.base
    }

    pub fn size(&self) -> u32 {
        self.size
    }

    pub fn io_mem(&self) -> &IoMem {
        &self.io_memory
    }

    /// Create a memory BAR structure.
    fn new(location: &PciDeviceLocation, index: u8) -> Result<Self> {
        // Get the original value first, then write all 1 to the register to get the length
        let offset = index as u16 * 4 + PciDeviceCommonCfgOffset::Bar0 as u16;
        let raw = location.read32(offset);
        location.write32(offset, !0);
        let len_encoded = location.read32(offset);
        location.write32(offset, raw);
        let mut address_length = AddrLen::Bits32;
        // base address, it may be bit64 or bit32
        let base: u64 = match (raw & 0b110) >> 1 {
            // bits32
            0 => (raw & !0xF) as u64,
            // bits64
            2 => {
                address_length = AddrLen::Bits64;
                ((raw & !0xF) as u64) | ((location.read32(offset + 4) as u64) << 32)
            }
            _ => {
                return Err(Error::InvalidArgs);
            }
        };
        // length
        let size = !(len_encoded & !0xF).wrapping_add(1);
        let prefetchable = raw & 0b1000 != 0;
        // The BAR is located in I/O memory region
        Ok(MemoryBar {
            base,
            size,
            prefetchable,
            address_length,
            io_memory: unsafe { IoMem::new((base as usize)..((base + size as u64) as usize)) },
        })
    }
}

/// Whether this BAR is 64bit address or 32bit address
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum AddrLen {
    Bits32,
    Bits64,
}

#[derive(Debug, Clone, Copy)]
pub struct IoBar {
    base: u32,
    size: u32,
}

impl IoBar {
    pub fn base(&self) -> u32 {
        self.base
    }

    pub fn size(&self) -> u32 {
        self.size
    }

    fn new(location: &PciDeviceLocation, index: u8) -> Result<Self> {
        let offset = index as u16 * 4 + PciDeviceCommonCfgOffset::Bar0 as u16;
        let raw = location.read32(offset);
        location.write32(offset, !0);
        let len_encoded = location.read32(offset);
        location.write32(offset, raw);
        let len = !(len_encoded & !0x3) + 1;
        Ok(Self {
            base: raw & !0x3,
            size: len,
        })
    }
}
