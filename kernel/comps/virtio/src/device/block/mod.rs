// SPDX-License-Identifier: MPL-2.0

pub mod device;

use core::mem::offset_of;

use aster_block::SECTOR_SIZE;
use aster_util::safe_ptr::SafePtr;
use bitflags::bitflags;
use int_to_c_enum::TryFromInt;
use ostd::Pod;

use crate::transport::{ConfigManager, VirtioTransport};

pub static DEVICE_NAME: &str = "Virtio-Block";

bitflags! {
    /// features for virtio block device
    pub(crate) struct BlockFeatures : u64 {
        const BARRIER       = 1 << 0;
        const SIZE_MAX      = 1 << 1;
        const SEG_MAX       = 1 << 2;
        const GEOMETRY      = 1 << 4;
        const RO            = 1 << 5;
        const BLK_SIZE      = 1 << 6;
        const SCSI          = 1 << 7;
        const FLUSH         = 1 << 9;
        const TOPOLOGY      = 1 << 10;
        const CONFIG_WCE    = 1 << 11;
        const MQ            = 1 << 12;
        const DISCARD       = 1 << 13;
        const WRITE_ZEROES  = 1 << 14;
    }
}

#[repr(u32)]
#[derive(Debug, Copy, Clone, TryFromInt)]
pub enum ReqType {
    In = 0,
    Out = 1,
    Flush = 4,
    GetId = 8,
    Discard = 11,
    WriteZeroes = 13,
}

#[repr(u8)]
#[derive(Debug, Eq, PartialEq, Copy, Clone, TryFromInt)]
pub enum RespStatus {
    /// Ok.
    Ok = 0,
    /// IoErr.
    IoErr = 1,
    /// Unsupported yet.
    Unsupported = 2,
    /// Not ready.
    _NotReady = 3,
}

#[derive(Debug, Copy, Clone, Pod)]
#[repr(C)]
pub struct VirtioBlockConfig {
    /// The number of 512-byte sectors.
    capacity: u64,
    /// The maximum segment size.
    size_max: u32,
    /// The maximum number of segments.
    seg_max: u32,
    /// The geometry of the device.
    geometry: VirtioBlockGeometry,
    /// The block size. If `logical_block_size` is not given in qemu cmdline,
    /// `blk_size` will be set to sector size (512 bytes) by default.
    blk_size: u32,
    /// The topology of the device.
    topology: VirtioBlockTopology,
    /// Writeback mode.
    writeback: u8,
    unused0: u8,
    /// The number of virtqueues.
    num_queues: u16,
    /// The maximum discard sectors for one segment.
    max_discard_sectors: u32,
    /// The maximum number of discard segments in a discard command.
    max_discard_seg: u32,
    /// Discard commands must be aligned to this number of sectors.
    discard_sector_alignment: u32,
    /// The maximum number of write zeroes sectors in one segment.
    max_write_zeroes_sectors: u32,
    /// The maximum number of segments in a write zeroes command.
    max_write_zeroes_seg: u32,
    /// Set if a write zeroes command may result in the
    /// deallocation of one or more of the sectors.
    write_zeros_may_unmap: u8,
    unused1: [u8; 3],
}

#[derive(Debug, Copy, Clone, Pod)]
#[repr(C)]
pub struct VirtioBlockGeometry {
    cylinders: u16,
    heads: u8,
    sectors: u8,
}

#[derive(Debug, Copy, Clone, Pod)]
#[repr(C)]
pub struct VirtioBlockTopology {
    /// Exponent for physical block per logical block.
    physical_block_exp: u8,
    /// Alignment offset in logical blocks.
    alignment_offset: u8,
    /// Minimum I/O size without performance penalty in logical blocks.
    min_io_size: u16,
    /// Optimal sustained I/O size in logical blocks.
    opt_io_size: u32,
}

#[derive(Debug, Copy, Clone)]
#[repr(C)]
pub struct VirtioBlockFeature {
    support_flush: bool,
}

impl VirtioBlockConfig {
    pub(self) fn new_manager(transport: &dyn VirtioTransport) -> ConfigManager<Self> {
        let safe_ptr = transport
            .device_config_mem()
            .map(|mem| SafePtr::new(mem, 0));
        let bar_space = transport.device_config_bar();

        ConfigManager::new(safe_ptr, bar_space)
    }

    pub(self) const fn sector_size() -> usize {
        SECTOR_SIZE
    }
}

impl ConfigManager<VirtioBlockConfig> {
    pub(super) fn read_config(&self) -> VirtioBlockConfig {
        let mut blk_config = VirtioBlockConfig::new_uninit();
        // Only following fields are defined in legacy interface.
        let cap_low = self
            .read_once::<u32>(offset_of!(VirtioBlockConfig, capacity))
            .unwrap() as u64;
        let cap_high = self
            .read_once::<u32>(offset_of!(VirtioBlockConfig, capacity) + 4)
            .unwrap() as u64;
        blk_config.capacity = cap_high << 32 | cap_low;
        blk_config.size_max = self
            .read_once::<u32>(offset_of!(VirtioBlockConfig, size_max))
            .unwrap();
        blk_config.seg_max = self
            .read_once::<u32>(offset_of!(VirtioBlockConfig, seg_max))
            .unwrap();
        blk_config.geometry.cylinders = self
            .read_once::<u16>(
                offset_of!(VirtioBlockConfig, geometry)
                    + offset_of!(VirtioBlockGeometry, cylinders),
            )
            .unwrap();
        blk_config.geometry.heads = self
            .read_once::<u8>(
                offset_of!(VirtioBlockConfig, geometry) + offset_of!(VirtioBlockGeometry, heads),
            )
            .unwrap();
        blk_config.geometry.sectors = self
            .read_once::<u8>(
                offset_of!(VirtioBlockConfig, geometry) + offset_of!(VirtioBlockGeometry, sectors),
            )
            .unwrap();
        blk_config.blk_size = self
            .read_once::<u32>(offset_of!(VirtioBlockConfig, blk_size))
            .unwrap();

        if self.is_modern() {
            // TODO: read more field if modern interface exists.
        }

        blk_config
    }

    pub(self) fn block_size(&self) -> usize {
        self.read_once::<u32>(offset_of!(VirtioBlockConfig, blk_size))
            .unwrap() as usize
    }

    pub(self) fn capacity_sectors(&self) -> usize {
        let cap_low = self
            .read_once::<u32>(offset_of!(VirtioBlockConfig, capacity))
            .unwrap() as usize;
        let cap_high = self
            .read_once::<u32>(offset_of!(VirtioBlockConfig, capacity) + 4)
            .unwrap() as usize;

        cap_high << 32 | cap_low
    }
}

impl VirtioBlockFeature {
    pub(self) fn new(transport: &dyn VirtioTransport) -> Self {
        let support_flush = transport.read_device_features() & BlockFeatures::FLUSH.bits() == 1;
        VirtioBlockFeature { support_flush }
    }
}
