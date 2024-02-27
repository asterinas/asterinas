// SPDX-License-Identifier: MPL-2.0

pub mod device;

use aster_frame::io_mem::IoMem;
use aster_util::safe_ptr::SafePtr;
use bitflags::bitflags;
use int_to_c_enum::TryFromInt;
use pod::Pod;

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
    capacity: u64,
    size_max: u64,
    geometry: VirtioBlockGeometry,
    blk_size: u32,
    topology: VirtioBlockTopology,
    writeback: u8,
    unused0: [u8; 3],
    max_discard_sectors: u32,
    max_discard_seg: u32,
    discard_sector_alignment: u32,
    max_write_zeroes_sectors: u32,
    max_write_zeroes_seg: u32,
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
    physical_block_exp: u8,
    alignment_offset: u8,
    min_io_size: u16,
    opt_io_size: u32,
}

impl VirtioBlockConfig {
    pub(self) fn new(transport: &dyn VirtioTransport) -> SafePtr<Self, IoMem> {
        let memory = transport.device_config_memory();
        SafePtr::new(memory, 0)
    }
}
