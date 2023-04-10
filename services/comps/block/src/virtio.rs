//! Block device based on Virtio

use jinux_frame::trap::TrapFrame;
use jinux_pci::msix::MSIX;
use jinux_util::frame_ptr::InFramePtr;
use jinux_virtio::{device::block::device::BLKDevice, PCIVirtioDevice, VitrioPciCommonCfg};
use log::debug;
use spin::Mutex;

use crate::{BlockDevice, BLK_COMPONENT};

pub struct VirtioBlockDevice {
    blk_device: Mutex<BLKDevice>,
    pub common_cfg: InFramePtr<VitrioPciCommonCfg>,
    msix: MSIX,
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
        debug!("block device handle irq");
    }
}

impl VirtioBlockDevice {
    pub(crate) fn new(mut virtio_device: PCIVirtioDevice) -> Self {
        fn handle_block_device(_: &TrapFrame) {
            BLK_COMPONENT.get().unwrap().blk_device.handle_irq()
        }
        fn config_space_change(_: &TrapFrame) {
            debug!("block device config space change");
        }
        virtio_device.register_interrupt_functions(&config_space_change, &handle_block_device);
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
