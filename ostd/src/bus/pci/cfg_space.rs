// SPDX-License-Identifier: MPL-2.0

//! The PCI configuration space.
//!
//! Reference: <https://wiki.osdev.org/PCI>

use alloc::sync::Arc;
use core::mem::size_of;

use bitflags::bitflags;

use super::PciDeviceLocation;
use crate::{
    arch::device::io_port::{PortRead, PortWrite},
    io_mem::IoMem,
    mm::{
        page_prop::{CachePolicy, PageFlags},
        PodOnce, VmIoOnce,
    },
    Error, Result,
};

/// Offset in PCI device's common configuration space(Not the PCI bridge).
#[repr(u16)]
pub enum PciDeviceCommonCfgOffset {
    /// Vendor ID
    VendorId = 0x00,
    /// Device ID
    DeviceId = 0x02,
    /// PCI Command
    Command = 0x04,
    /// PCI Status
    Status = 0x06,
    /// Revision ID
    RevisionId = 0x08,
    /// Class code
    ClassCode = 0x09,
    /// Cache Line Size
    CacheLineSize = 0x0C,
    /// Latency Timer
    LatencyTimer = 0x0D,
    /// Header Type: Identifies the layout of the header.
    HeaderType = 0x0E,
    /// BIST: Represents the status and allows control of a devices BIST(built-in self test).
    Bist = 0x0F,
    /// Base Address Register #0
    Bar0 = 0x10,
    /// Base Address Register #1
    Bar1 = 0x14,
    /// Base Address Register #2
    Bar2 = 0x18,
    /// Base Address Register #3
    Bar3 = 0x1C,
    /// Base Address Register #4
    Bar4 = 0x20,
    /// Base Address Register #5
    Bar5 = 0x24,
    /// Cardbus CIS Pointer
    CardbusCisPtr = 0x28,
    /// Subsystem Vendor ID
    SubsystemVendorId = 0x2C,
    /// Subsystem ID
    SubsystemId = 0x2E,
    /// Expansion ROM base address
    XromBar = 0x30,
    /// Capabilities pointer
    CapabilitiesPointer = 0x34,
    /// Interrupt Line
    InterruptLine = 0x3C,
    /// INterrupt PIN
    InterruptPin = 0x3D,
    /// Min Grant
    MinGrant = 0x3E,
    /// Max latency
    MaxLatency = 0x3F,
}

bitflags! {
    /// PCI device common config space command register.
    pub struct Command: u16 {
        /// Sets to 1 if the device can respond to I/O Space accesses.
        const IO_SPACE                  =  1 << 0;
        /// Sets to 1 if the device can respond to Memory SPace accesses.
        const MEMORY_SPACE              =  1 << 1;
        /// Sets to 1 if the device can behave as a bus master.
        const BUS_MASTER                =  1 << 2;
        /// Sets to 1 if the device can monitor Special Cycle operations.
        const SPECIAL_CYCLES            =  1 << 3;
        /// Memory Write and Invalidate Enable. Set to 1 if the device can
        /// generate the Memory Write and Invalidate command.
        const MWI_ENABLE                =  1 << 4;
        /// Sets to 1 if the device does not respond to palette register writes
        /// and will snoop the data.
        const VGA_PALETTE_SNOOP         =  1 << 5;
        /// Sets to 1 if the device will takes its normal action when a parity
        /// error is detected.
        const PARITY_ERROR_RESPONSE     =  1 << 6;
        /// Sets to 1 if the SERR# driver is enabled.
        const SERR_ENABLE               =  1 << 8;
        /// Sets to 1 if the device is allowed to generate fast back-to-back
        /// transactions
        const FAST_BACK_TO_BACK_ENABLE  =  1 << 9;
        /// Sets to 1 if the assertion of the devices INTx# signal is disabled.
        const INTERRUPT_DISABLE         =  1 << 10;
    }
}

bitflags! {
    /// PCI device common config space status register.
    pub struct Status: u16 {
        /// The status of the device's INTx# signal.
        const INTERRUPT_STATUS          = 1 << 3;
        /// Sets to 1 if the device support capabilities.
        const CAPABILITIES_LIST         = 1 << 4;
        /// Sets to 1 if the device is capable of running at 66 MHz.
        const MHZ66_CAPABLE             = 1 << 5;
        /// Sets to 1 if the device can accept fast back-to-back transactions
        /// that are not from the same agent.
        const FAST_BACK_TO_BACK_CAPABLE = 1 << 7;
        /// This bit is only set when the following conditions are met:
        /// 1. The bus agent asserted PERR# on a read or observed an assertion
        /// of PERR# on a write
        /// 2. The agent setting the bit acted as the bus master for the
        /// operation in which the error occurred
        /// 3. Bit 6 of the Command register (Parity Error Response bit) is set
        ///  to 1.
        const MASTER_DATA_PARITY_ERROR  = 1 << 8;
        /// The read-only bit that represent the slowest time that a device will
        /// assert DEVSEL# for any bus command except Configuration Space read
        /// and writes.
        ///
        /// If both `DEVSEL_MEDIUM_TIMING` and `DEVSEL_SLOW_TIMING` are not set,
        /// then it represents fast timing
        const DEVSEL_MEDIUM_TIMING      = 1 << 9;
        /// Check `DEVSEL_MEDIUM_TIMING`
        const DEVSEL_SLOW_TIMING        = 1 << 10;
        /// Sets to 1 when a target device terminates a transaction with Target-
        /// Abort.
        const SIGNALED_TARGET_ABORT     = 1 << 11;
        /// Sets to 1 by a master device when its transaction is terminated with
        /// Target-Abort
        const RECEIVED_TARGET_ABORT     = 1 << 12;
        /// Sets to 1 by a master device when its transaction (except for Special
        /// Cycle transactions) is terminated with Master-Abort.
        const RECEIVED_MASTER_ABORT     = 1 << 13;
        /// Sets to 1 when the device asserts SERR#
        const SIGNALED_SYSTEM_ERROR     = 1 << 14;
        /// Sets to 1 when the device detects a parity error, even if parity error
        /// handling is disabled.
        const DETECTED_PARITY_ERROR     = 1 << 15;
    }
}

/// BAR space in PCI common config space.
#[derive(Debug, Clone)]
pub enum Bar {
    /// Memory BAR
    Memory(Arc<MemoryBar>),
    /// I/O BAR
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

    /// Reads a value of a specified type at a specified offset.
    pub fn read_once<T: PodOnce + PortRead>(&self, offset: usize) -> Result<T> {
        match self {
            Bar::Memory(mem_bar) => mem_bar.io_mem().read_once(offset),
            Bar::Io(io_bar) => io_bar.read(offset as u32),
        }
    }

    /// Writes a value of a specified type at a specified offset.
    pub fn write_once<T: PodOnce + PortWrite>(&self, offset: usize, value: T) -> Result<()> {
        match self {
            Bar::Memory(mem_bar) => mem_bar.io_mem().write_once(offset, &value),
            Bar::Io(io_bar) => io_bar.write(offset as u32, value),
        }
    }
}

/// Memory BAR
#[derive(Debug, Clone)]
pub struct MemoryBar {
    base: u64,
    size: u32,
    prefetchable: bool,
    address_length: AddrLen,
    io_memory: IoMem,
}

impl MemoryBar {
    /// Memory BAR bits type
    pub fn address_length(&self) -> AddrLen {
        self.address_length
    }

    /// Whether this bar is prefetchable, allowing the CPU to get the data
    /// in advance.
    pub fn prefetchable(&self) -> bool {
        self.prefetchable
    }

    /// Base address
    pub fn base(&self) -> u64 {
        self.base
    }

    /// Size of the memory
    pub fn size(&self) -> u32 {
        self.size
    }

    /// Grants I/O memory access
    pub fn io_mem(&self) -> &IoMem {
        &self.io_memory
    }

    /// Creates a memory BAR structure.
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
        let size = (!(len_encoded & !0xF)).wrapping_add(1);
        let prefetchable = raw & 0b1000 != 0;
        // The BAR is located in I/O memory region
        Ok(MemoryBar {
            base,
            size,
            prefetchable,
            address_length,
            io_memory: unsafe {
                IoMem::new(
                    (base as usize)..((base + size as u64) as usize),
                    PageFlags::RW,
                    CachePolicy::Uncacheable,
                )
            },
        })
    }
}

/// Whether this BAR is 64bit address or 32bit address
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum AddrLen {
    /// 32 bits
    Bits32,
    /// 64 bits
    Bits64,
}

/// I/O port BAR.
#[derive(Debug, Clone, Copy)]
pub struct IoBar {
    base: u32,
    size: u32,
}

impl IoBar {
    /// Base port
    pub fn base(&self) -> u32 {
        self.base
    }

    /// Size of the port
    pub fn size(&self) -> u32 {
        self.size
    }

    /// Reads from port
    pub fn read<T: PortRead>(&self, offset: u32) -> Result<T> {
        // Check alignment
        if (self.base + offset) % size_of::<T>() as u32 != 0 {
            return Err(Error::InvalidArgs);
        }
        // Check overflow
        if self.size < size_of::<T>() as u32 || offset > self.size - size_of::<T>() as u32 {
            return Err(Error::InvalidArgs);
        }
        // SAFETY: The range of ports accessed is within the scope managed by the IoBar and
        // an out-of-bounds check is performed.
        unsafe { Ok(T::read_from_port((self.base + offset) as u16)) }
    }

    /// Writes to port
    pub fn write<T: PortWrite>(&self, offset: u32, value: T) -> Result<()> {
        // Check alignment
        if (self.base + offset) % size_of::<T>() as u32 != 0 {
            return Err(Error::InvalidArgs);
        }
        // Check overflow
        if size_of::<T>() as u32 > self.size || offset > self.size - size_of::<T>() as u32 {
            return Err(Error::InvalidArgs);
        }
        // SAFETY: The range of ports accessed is within the scope managed by the IoBar and
        // an out-of-bounds check is performed.
        unsafe { T::write_to_port((self.base + offset) as u16, value) }
        Ok(())
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
