use kxos_frame::Pod;
use kxos_frame_pod_derive::Pod;
use kxos_pci::capability::vendor::virtio::CapabilityVirtioData;
use kxos_pci::util::BAR;
use kxos_util::frame_ptr::InFramePtr;

pub const BLK_SIZE: usize = 512;

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
