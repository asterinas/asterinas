// SPDX-License-Identifier: MPL-2.0

use super::PciDeviceCfgSpace;
use crate::{io::IoMem, mm::VmIoOnce, Result};

/// PCI device Location
#[derive(Debug, Clone)]
pub(crate) struct PciDeviceLocation {
    /// Segment group base address
    pub segment_group_base_addr: usize,
    /// Bus number
    pub bus: u8,
    /// Device number
    pub device: u8,
    /// Device number
    pub function: u8,
    /// Configuration space
    pub cfg_space: Option<IoMem>,
}

impl PciDeviceLocation {
    const MIN_BUS: u8 = 0;
    const MAX_BUS: u8 = 255;
    const MIN_DEVICE: u8 = 0;
    const MAX_DEVICE: u8 = 31;
    const MIN_FUNCTION: u8 = 0;
    const MAX_FUNCTION: u8 = 7;

    /// Returns an iterator that enumerates all possible PCI device locations.
    pub fn all() -> impl Iterator<Item = PciDeviceLocation> {
        let segment_group_base_addr_vec = crate::arch::pci::collect_segment_group_base_addrs();
        core::iter::from_coroutine(
            #[coroutine]
            || {
                for segment_group_base_addr in segment_group_base_addr_vec {
                    for bus in Self::MIN_BUS..=Self::MAX_BUS {
                        for device in Self::MIN_DEVICE..=Self::MAX_DEVICE {
                            for function in Self::MIN_FUNCTION..=Self::MAX_FUNCTION {
                                let loc = PciDeviceLocation {
                                    segment_group_base_addr,
                                    bus,
                                    device,
                                    function,
                                    cfg_space: None,
                                };
                                yield loc;
                            }
                        }
                    }
                }
            },
        )
    }

    /// The page table of all devices is the same. So we can use any device ID.
    /// FIXME: Distinguish different device ID.
    pub fn zero() -> Self {
        Self {
            segment_group_base_addr: 0,
            bus: 0,
            device: 0,
            function: 0,
            cfg_space: None,
        }
    }
}

impl PciDeviceLocation {
    pub fn acquire_io_mem(&mut self) -> Result<()> {
        let start_paddr = self.segment_group_base_addr
            + ((self.bus as usize) << 20)
            + ((self.device as usize) << 15)
            + ((self.function as usize) << 12);
        let io_mem = IoMem::acquire(start_paddr..start_paddr + PciDeviceCfgSpace::SIZE)?;
        self.cfg_space = Some(io_mem);
        Ok(())
    }

    pub const BIT32_ALIGN_MASK: usize = 0xFFFC;

    pub fn read8(&self, offset: usize) -> Result<u8> {
        let val = self.read32(offset & Self::BIT32_ALIGN_MASK)?;
        Ok(((val >> ((offset & 0b11) << 3)) & 0xFF) as u8)
    }

    pub fn read16(&self, offset: usize) -> Result<u16> {
        let val = self.read32(offset & Self::BIT32_ALIGN_MASK)?;
        Ok(((val >> ((offset & 0b10) << 3)) & 0xFFFF) as u16)
    }

    pub fn read32(&self, offset: usize) -> Result<u32> {
        debug_assert_eq!(
            offset & 0b11,
            0,
            "misaligned PCI configuration dword u32 read"
        );
        self.cfg_space
            .as_ref()
            .unwrap()
            .read_once::<u32>(offset)
            .map(u32::from_le)
    }

    pub fn write8(&self, offset: usize, val: u8) -> Result<()> {
        let old = self.read32(offset & Self::BIT32_ALIGN_MASK)?;
        let dest = (offset & 0b11) << 3;
        let mask = (0xFF << dest) as u32;
        self.write32(
            offset & Self::BIT32_ALIGN_MASK,
            ((val as u32) << dest) | (old & !mask),
        )
    }

    pub fn write16(&self, offset: usize, val: u16) -> Result<()> {
        let old = self.read32(offset & Self::BIT32_ALIGN_MASK)?;
        let dest = (offset & 0b10) << 3;
        let mask = (0xFFFF << dest) as u32;
        self.write32(
            offset & Self::BIT32_ALIGN_MASK,
            ((val as u32) << dest) | (old & !mask),
        )
    }

    pub fn write32(&self, offset: usize, val: u32) -> Result<()> {
        debug_assert_eq!(
            offset & 0b11,
            0,
            "misaligned PCI configuration dword u32 write"
        );
        self.cfg_space
            .as_ref()
            .unwrap()
            .write_once::<u32>(offset, &val.to_le())
    }
}

macro_rules! define_cfg_space_and_impl_read_write_for_location {
    (
        $(#[$meta:meta])*
        $vis:vis struct $struct_name:ident {
            $(
                $(#[$field_meta:meta])*
                $field_vis:vis $field_name:ident : $field_type:ty,
            )*
        }
    ) => {
        $(#[$meta])*
        $vis struct $struct_name {
            $(
                $(#[$field_meta])*
                $field_vis $field_name: $field_type,
            )*
        }

        impl PciDeviceLocation {
            paste::paste! {
                $(
                    #[doc = concat!("Reads the `", stringify!($field_name), "` field from the PCI configuration space.")]
                    #[allow(clippy::allow_attributes)]
                    #[allow(unused)]
                    pub fn [<read_ $field_name>](&self) -> Result<$field_type> {
                        const OFFSET: usize = core::mem::offset_of!($struct_name, $field_name);
                        match core::mem::size_of::<$field_type>() {
                            1 => Ok(self.read8(OFFSET)? as $field_type),
                            2 => Ok(self.read16(OFFSET)? as $field_type),
                            4 => Ok(self.read32(OFFSET)? as $field_type),
                            _ => Err(crate::Error::InvalidArgs),
                        }
                    }

                    #[doc = concat!("Writes the `", stringify!($field_name), "` field to the PCI configuration space.")]
                    #[allow(clippy::allow_attributes)]
                    #[allow(unused)]
                    pub fn [<write_ $field_name>](&self, value: $field_type) -> Result<()> {
                        const OFFSET: usize = core::mem::offset_of!($struct_name, $field_name);
                        match core::mem::size_of::<$field_type>() {
                            1 => self.write8(OFFSET, value as u8),
                            2 => self.write16(OFFSET, value as u16),
                            4 => self.write32(OFFSET, value as u32),
                            _ => Err(crate::Error::InvalidArgs),
                        }
                    }
                )*
            }
        }
    };
}

pub(super) use define_cfg_space_and_impl_read_write_for_location;
