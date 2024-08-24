// SPDX-License-Identifier: MPL-2.0

pub mod device;

use aster_block::SECTOR_SIZE;
use aster_util::{field_ptr, safe_ptr::SafePtr};
use bitflags::bitflags;
use int_to_c_enum::TryFromInt;
use ostd::{io_mem::IoMem, Pod};

use crate::transport::VirtioTransport;

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
    size_max: u64,
    /// The geometry of the device.
    geometry: VirtioBlockGeometry,
    /// The block size. If `logical_block_size` is not given in qemu cmdline,
    /// `blk_size` will be set to sector size (512 bytes) by default.
    blk_size: u32,
    /// The topology of the device.
    topology: VirtioBlockTopology,
    /// Writeback mode.
    writeback: u8,
    unused0: [u8; 3],
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

impl VirtioBlockConfig {
    pub(self) fn new(transport: &dyn VirtioTransport) -> SafePtr<Self, IoMem> {
        let memory = transport.device_config_memory();
        SafePtr::new(memory, 0)
    }

    pub(self) const fn sector_size() -> usize {
        SECTOR_SIZE
    }

    pub(self) fn read_block_size(this: &SafePtr<Self, IoMem>) -> ostd::prelude::Result<usize> {
        field_ptr!(this, Self, blk_size)
            .read_once()
            .map(|val| val as usize)
    }

    pub(self) fn read_capacity_sectors(
        this: &SafePtr<Self, IoMem>,
    ) -> ostd::prelude::Result<usize> {
        field_ptr!(this, Self, capacity)
            .read_once()
            .map(|val| val as usize)
    }
}
