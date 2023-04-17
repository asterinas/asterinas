pub mod device;

use bitflags::bitflags;
use int_to_c_enum::TryFromInt;
use jinux_pci::capability::vendor::virtio::CapabilityVirtioData;
use jinux_pci::util::BAR;
use jinux_util::frame_ptr::InFramePtr;
use pod::Pod;

pub const BLK_SIZE: usize = 512;

bitflags! {
    /// features for virtio block device
    pub(crate) struct BLKFeatures : u64{
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
pub struct VirtioBLKConfig {
    capacity: u64,
    size_max: u64,
    geometry: VirtioBLKGeometry,
    blk_size: u32,
    topology: VirtioBLKTopology,
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
pub struct VirtioBLKGeometry {
    cylinders: u16,
    heads: u8,
    sectors: u8,
}

#[derive(Debug, Copy, Clone, Pod)]
#[repr(C)]
pub struct VirtioBLKTopology {
    physical_block_exp: u8,
    alignment_offset: u8,
    min_io_size: u16,
    opt_io_size: u32,
}

impl VirtioBLKConfig {
    pub(crate) fn new(cap: &CapabilityVirtioData, bars: [Option<BAR>; 6]) -> InFramePtr<Self> {
        let bar = cap.bar;
        let offset = cap.offset;
        match bars[bar as usize].expect("Virtio pci block cfg:bar is none") {
            BAR::Memory(address, _, _, _) => InFramePtr::new(address as usize + offset as usize)
                .expect("can not get in frame ptr for virtio block config"),
            BAR::IO(_, _) => {
                panic!("Virtio pci block cfg:bar is IO type")
            }
        }
    }
}
