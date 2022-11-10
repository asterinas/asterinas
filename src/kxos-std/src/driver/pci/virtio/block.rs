use core::hint::spin_loop;

use crate::process::Process;
use alloc::sync::Arc;
use alloc::vec::Vec;
use kxos_frame::info;
use kxos_pci::PCIDevice;
use kxos_virtio::PCIVirtioDevice;
use lazy_static::lazy_static;
use spin::mutex::Mutex;

use super::BlockDevice;
pub const BLK_SIZE: usize = 512;
use kxos_frame::Pod;
use kxos_frame::TrapFrame;
pub struct VirtioBlockDevice {
    virtio_device: PCIVirtioDevice,
}

#[repr(C)]
#[derive(Debug, Copy, Clone, Pod)]
pub struct BlkReq {
    pub type_: ReqType,
    pub reserved: u32,
    pub sector: u64,
}

/// Response of a VirtIOBlk request.
#[repr(C)]
#[derive(Debug, Copy, Clone, Pod)]
pub struct BlkResp {
    pub status: RespStatus,
}

#[repr(u32)]
#[derive(Debug, Copy, Clone, Pod)]
pub enum ReqType {
    In = 0,
    Out = 1,
    Flush = 4,
    Discard = 11,
    WriteZeroes = 13,
}

#[repr(u8)]
#[derive(Debug, Eq, PartialEq, Copy, Clone, Pod)]
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

impl Default for BlkResp {
    fn default() -> Self {
        BlkResp {
            status: RespStatus::_NotReady,
        }
    }
}

lazy_static! {
    // TODO: use dyn BlockDevice instead
    pub static ref BLOCK_DEVICE: Arc<Mutex<Option<VirtioBlockDevice>>> = Arc::new(Mutex::new(None)) ;
}

impl BlockDevice for VirtioBlockDevice {
    fn read_block(&self, block_id: usize, buf: &mut [u8]) {
        assert_eq!(buf.len(), BLK_SIZE);
        let req = BlkReq {
            type_: ReqType::In,
            reserved: 0,
            sector: block_id as u64,
        };
        let mut resp = BlkResp::default();
        let mut queue = self.virtio_device.get_queue(0);
        queue
            .add(&[req.as_bytes()], &[buf, resp.as_bytes_mut()])
            .expect("add queue failed");
        queue.notify();
        while !queue.can_pop() {
            spin_loop();
        }
        queue.pop_used().expect("pop used failed");
        match resp.status {
            RespStatus::Ok => {}
            _ => panic!("io error in block device"),
        };
    }
    /// it is blocking now
    fn write_block(&self, block_id: usize, buf: &[u8]) {
        assert_eq!(buf.len(), BLK_SIZE);
        let req = BlkReq {
            type_: ReqType::Out,
            reserved: 0,
            sector: block_id as u64,
        };
        let mut resp = BlkResp::default();
        let mut queue = self.virtio_device.get_queue(0);
        queue
            .add(&[req.as_bytes(), buf], &[resp.as_bytes_mut()])
            .expect("add queue failed");
        queue.notify();

        while !queue.can_pop() {
            spin_loop();
        }
        queue.pop_used().expect("pop used failed");
        match resp.status {
            RespStatus::Ok => {}
            _ => panic!("io error in block device"),
        };
    }
    fn handle_irq(&self) {
        info!("handle irq in block device!");
    }
}

impl VirtioBlockDevice {
    fn new(mut virtio_device: PCIVirtioDevice) -> Self {
        fn handle_block_device(frame: &TrapFrame) {
            info!("pci block device queue interrupt");
            BLOCK_DEVICE.lock().as_ref().unwrap().handle_irq()
        }
        let mut functions = Vec::new();
        functions.push(handle_block_device);
        virtio_device.register_queue_interrupt_functions(&mut functions);
        Self { virtio_device }
    }
}

pub fn init(pci_device: Arc<PCIDevice>) {
    let virtio_device = PCIVirtioDevice::new(pci_device);
    let mut a = BLOCK_DEVICE.lock();
    a.replace(VirtioBlockDevice::new(virtio_device));
    drop(a);
}

#[allow(unused)]
pub fn block_device_test() {
    fn inner_block_device_test() {
        let block_device = BLOCK_DEVICE.clone();
        let mut write_buffer = [0u8; 512];
        let mut read_buffer = [0u8; 512];
        info!("write_buffer address:{:x}", write_buffer.as_ptr() as usize);
        info!("read_buffer address:{:x}", read_buffer.as_ptr() as usize);
        for i in 0..512 {
            for byte in write_buffer.iter_mut() {
                *byte = i as u8;
            }
            block_device
                .lock()
                .as_ref()
                .unwrap()
                .write_block(i as usize, &write_buffer);
            block_device
                .lock()
                .as_ref()
                .unwrap()
                .read_block(i as usize, &mut read_buffer);
            assert_eq!(write_buffer, read_buffer);
        }
        info!("block device test passed!");
    }

    let test_process = Process::spawn_kernel_process(|| {
        inner_block_device_test();
    });
}
