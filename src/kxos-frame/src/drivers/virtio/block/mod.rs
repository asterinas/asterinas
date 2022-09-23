mod virtio_blk;

use core::{any::Any, mem::transmute};

use crate::prelude::*;
use crate::{
    cell::Cell,
    drivers::pci::{PortOpsImpl, PCI_BAR},
    info, mm,
    trap::{IrqCallbackHandle, IrqLine, TrapFrame},
};
use pci::{CSpaceAccessMethod, Location, PCIDevice};

use super::AsBuf;

pub const BLK_SIZE: usize = 512;

pub trait BlockDevice: Send + Sync + Any {
    fn read_block(&self, block_id: usize, buf: &mut [u8]) -> Result<()>;
    fn write_block(&self, block_id: usize, buf: &[u8]) -> Result<()>;
    fn handle_irq(&self);
}

pub static BLOCK_DEVICE: Cell<Arc<dyn BlockDevice>> = unsafe {
    transmute(&0 as *const _ as *const virtio_blk::VirtIOBlock as *const dyn BlockDevice)
};

static mut BLOCK_DEVICE_IRQ_CALLBACK_LIST: Vec<IrqCallbackHandle> = Vec::new();

pub fn init(loc: PCIDevice) {
    let dev = virtio_blk::VirtIOBlock::new(loc);
    unsafe {
        (BLOCK_DEVICE.get() as *mut Arc<dyn BlockDevice>).write(Arc::new(dev));
    }
}

#[repr(u8)]
#[derive(Debug, Eq, PartialEq, Copy, Clone)]
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

#[derive(Debug)]
#[repr(C)]
pub struct VirtioBLKConfig {
    pub capacity: u64,
    pub size_max: u64,
    pub geometry: VirtioBLKGeometry,
    pub blk_size: u32,
    pub topology: VirtioBLKTopology,
    pub writeback: u8,
    pub unused0: [u8; 3],
    pub max_discard_sectors: u32,
    pub max_discard_seg: u32,
    pub discard_sector_alignment: u32,
    pub max_write_zeroes_sectors: u32,
    pub max_write_zeroes_seg: u32,
    pub write_zeros_may_unmap: u8,
    pub unused1: [u8; 3],
}

#[repr(C)]
#[derive(Debug)]
struct BlkReq {
    type_: ReqType,
    reserved: u32,
    sector: u64,
}

/// Response of a VirtIOBlk request.
#[repr(C)]
#[derive(Debug)]
pub struct BlkResp {
    pub status: RespStatus,
}

#[repr(u32)]
#[derive(Debug)]
enum ReqType {
    In = 0,
    Out = 1,
    Flush = 4,
    Discard = 11,
    WriteZeroes = 13,
}

#[derive(Debug)]
#[repr(C)]
pub struct VirtioBLKGeometry {
    pub cylinders: u16,
    pub heads: u8,
    pub sectors: u8,
}

#[derive(Debug)]
#[repr(C)]
pub struct VirtioBLKTopology {
    pub physical_block_exp: u8,
    pub alignment_offset: u8,
    pub min_io_size: u16,
    pub opt_io_size: u32,
}

impl VirtioBLKConfig {
    pub unsafe fn new(loc: Location, cap_ptr: u16) -> &'static mut Self {
        let ops = &PortOpsImpl;
        let am = CSpaceAccessMethod::IO;
        let bar = am.read8(ops, loc, cap_ptr + 4);
        let offset = am.read32(ops, loc, cap_ptr + 8);
        let bar_address = am.read32(ops, loc, PCI_BAR + bar as u16 * 4) & (!(0b1111));
        &mut *(mm::phys_to_virt(bar_address as usize + offset as usize) as *const usize
            as *mut Self)
    }
}

impl Default for BlkResp {
    fn default() -> Self {
        BlkResp {
            status: RespStatus::_NotReady,
        }
    }
}

unsafe impl AsBuf for BlkReq {}
unsafe impl AsBuf for BlkResp {}

pub fn read_block(block_id: usize, buf: &mut [u8]) -> Result<()> {
    BLOCK_DEVICE.get().read_block(block_id, buf)
}

pub fn write_block(block_id: usize, buf: &[u8]) -> Result<()> {
    BLOCK_DEVICE.get().write_block(block_id, buf)
}

#[allow(unused)]
fn block_device_test() {
    let block_device = BLOCK_DEVICE.clone();
    let mut write_buffer = [0u8; 512];
    let mut read_buffer = [0u8; 512];
    info!("test:{:x}", write_buffer.as_ptr() as usize);
    for i in 0..512 {
        for byte in write_buffer.iter_mut() {
            *byte = i as u8;
        }
        block_device.write_block(i as usize, &write_buffer);
        block_device.read_block(i as usize, &mut read_buffer);
        assert_eq!(write_buffer, read_buffer);
    }
    info!("block device test passed!");
}
