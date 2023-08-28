pub mod device;

use bitflags::bitflags;
use int_to_c_enum::TryFromInt;
use jinux_frame::io_mem::IoMem;
use jinux_util::safe_ptr::SafePtr;
use pod::Pod;

use crate::transport::VirtioTransport;

pub const BLK_SIZE: usize = 512;
pub static DEVICE_NAME: &'static str = "Virtio-Block";

bitflags! {
    /// features for virtio block device
    pub(crate) struct BlkFeatures : u64{
        const SIZE_MAX      = 1 << 1;
        const SEG_MAX       = 1 << 2;
        const GEOMETRY      = 1 << 4;
        const RO            = 1 << 5;
        const BLK_SIZE      = 1 << 6;
        const FLUSH         = 1 << 9;
        const TOPOLOGY      = 1 << 10;
        const CONFIG_WCE    = 1 << 11;
        const DISCARD       = 1 << 13;
        const WRITE_ZEROES  = 1 << 14;

    }

}

#[repr(C)]
#[derive(Debug, Copy, Clone, Pod)]
pub struct BlkReq {
    pub type_: u32,
    pub reserved: u32,
    pub sector: u64,
}

/// Response of a VirtIOBlk request.
#[repr(C)]
#[derive(Debug, Copy, Clone, Pod)]
pub struct BlkResp {
    pub status: u8,
}

impl Default for BlkResp {
    fn default() -> Self {
        BlkResp {
            status: RespStatus::_NotReady as _,
        }
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
pub struct VirtioBlkConfig {
    capacity: u64,
    size_max: u64,
    geometry: VirtioBlkGeometry,
    blk_size: u32,
    topology: VirtioBlkTopology,
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
pub struct VirtioBlkGeometry {
    cylinders: u16,
    heads: u8,
    sectors: u8,
}

#[derive(Debug, Copy, Clone, Pod)]
#[repr(C)]
pub struct VirtioBlkTopology {
    physical_block_exp: u8,
    alignment_offset: u8,
    min_io_size: u16,
    opt_io_size: u32,
}

impl VirtioBlkConfig {
    pub(self) fn new(transport: &mut dyn VirtioTransport) -> SafePtr<Self, IoMem> {
        let memory = transport.device_config_memory();
        SafePtr::new(memory, 0)
    }
}
