use crate::process::Process;
use alloc::sync::Arc;
use jinux_pci::msix::MSIX;
use jinux_pci::PCIDevice;
use jinux_util::frame_ptr::InFramePtr;
use jinux_virtio::device::block::device::BLKDevice;
use jinux_virtio::device::block::BlkResp;
use jinux_virtio::PCIVirtioDevice;
use jinux_virtio::VitrioPciCommonCfg;
use lazy_static::lazy_static;
use log::info;
use spin::mutex::Mutex;

use super::BlockDevice;
pub const BLK_SIZE: usize = 512;
use jinux_frame::TrapFrame;
pub struct VirtioBlockDevice {
    blk_device: Mutex<BLKDevice>,
    pub common_cfg: InFramePtr<VitrioPciCommonCfg>,
    msix: MSIX,
}

lazy_static! {
    // TODO: use dyn BlockDevice instead
    pub static ref BLOCK_DEVICE: Arc<Mutex<Option<VirtioBlockDevice>>> = Arc::new(Mutex::new(None)) ;
}

impl VirtioBlockDevice {
    pub fn read_block_nb(&self, block_id: usize, buf: &mut [u8], res: &mut BlkResp) {
        self.blk_device.lock().read_block_nb(block_id, buf, res);
    }
}

impl BlockDevice for VirtioBlockDevice {
    fn read_block(&self, block_id: usize, buf: &mut [u8]) {
        self.blk_device.lock().read_block(block_id, buf);
    }
    /// it is blocking now
    fn write_block(&self, block_id: usize, buf: &[u8]) {
        self.blk_device.lock().write_block(block_id, buf);
    }
    fn handle_irq(&self) {
        info!("handle irq in block device!");
    }
}

impl VirtioBlockDevice {
    fn new(mut virtio_device: PCIVirtioDevice) -> Self {
        fn handle_block_device(frame: &TrapFrame) {
            info!("pci block device queue interrupt");
            BLOCK_DEVICE.lock().as_ref().unwrap().handle_irq();
        }
        virtio_device.register_interrupt_functions(&handle_block_device);
        let blk_device = Mutex::new(match virtio_device.device {
            jinux_virtio::device::VirtioDevice::Block(blk) => blk,
            _ => {
                panic!("Error when creating new block device, the input device is other type of virtio device");
            }
        });
        Self {
            blk_device,
            common_cfg: virtio_device.common_cfg,
            msix: virtio_device.msix,
        }
    }
}

pub fn init(pci_device: Arc<PCIDevice>) {
    let virtio_device = PCIVirtioDevice::new(pci_device);
    let mut a = BLOCK_DEVICE.lock();
    a.replace(VirtioBlockDevice::new(virtio_device));
    let dev = a.as_ref().unwrap();
    drop(a);
}
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
        info!("write block");
        block_device
            .lock()
            .as_ref()
            .unwrap()
            .write_block(i as usize, &write_buffer);
        info!("read block");
        block_device
            .lock()
            .as_ref()
            .unwrap()
            .read_block(i as usize, &mut read_buffer);
        assert_eq!(write_buffer, read_buffer);
    }
    info!("block device test passed!");
}
#[allow(unused)]
pub fn block_device_test() {
    let _ = Process::spawn_kernel_process(|| {
        inner_block_device_test();
    });
}
