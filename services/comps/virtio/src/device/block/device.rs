use core::hint::spin_loop;

use alloc::{boxed::Box, string::ToString, sync::Arc};
use jinux_frame::{io_mem::IoMem, sync::SpinLock, trap::TrapFrame};
use jinux_util::safe_ptr::SafePtr;
use log::info;
use pod::Pod;

use crate::{
    device::block::{BlkReq, BlkResp, ReqType, RespStatus, BLK_SIZE},
    device::VirtioDeviceError,
    queue::VirtQueue,
    transport::VirtioTransport,
};

use super::{BlkFeatures, VirtioBlkConfig};

#[derive(Debug)]
pub struct BlockDevice {
    config: SafePtr<VirtioBlkConfig, IoMem>,
    queue: SpinLock<VirtQueue>,
    transport: Box<dyn VirtioTransport>,
}

impl BlockDevice {
    /// read data from block device, this function is blocking
    /// FIEME: replace slice with a more secure data structure to use dma mapping.
    pub fn read(&self, block_id: usize, buf: &mut [u8]) {
        assert_eq!(buf.len(), BLK_SIZE);
        let req = BlkReq {
            type_: ReqType::In as _,
            reserved: 0,
            sector: block_id as u64,
        };
        let mut resp = BlkResp::default();
        let mut queue = self.queue.lock_irq_disabled();
        let token = queue
            .add(&[req.as_bytes()], &[buf, resp.as_bytes_mut()])
            .expect("add queue failed");
        queue.notify();
        while !queue.can_pop() {
            spin_loop();
        }
        queue.pop_used_with_token(token).expect("pop used failed");
        match RespStatus::try_from(resp.status).unwrap() {
            RespStatus::Ok => {}
            _ => panic!("io error in block device"),
        };
    }
    /// write data to block device, this function is blocking
    /// FIEME: replace slice with a more secure data structure to use dma mapping.
    pub fn write(&self, block_id: usize, buf: &[u8]) {
        assert_eq!(buf.len(), BLK_SIZE);
        let req = BlkReq {
            type_: ReqType::Out as _,
            reserved: 0,
            sector: block_id as u64,
        };
        let mut resp = BlkResp::default();
        let mut queue = self.queue.lock_irq_disabled();
        let token = queue
            .add(&[req.as_bytes(), buf], &[resp.as_bytes_mut()])
            .expect("add queue failed");
        queue.notify();
        while !queue.can_pop() {
            spin_loop();
        }
        queue.pop_used_with_token(token).expect("pop used failed");
        let st = resp.status;
        match RespStatus::try_from(st).unwrap() {
            RespStatus::Ok => {}
            _ => panic!("io error in block device:{:?}", st),
        };
    }

    /// Create a new VirtIO-Block driver.
    pub(crate) fn init(mut transport: Box<dyn VirtioTransport>) -> Result<(), VirtioDeviceError> {
        let config = VirtioBlkConfig::new(transport.as_mut());
        let num_queues = transport.num_queues();
        if num_queues != 1 {
            return Err(VirtioDeviceError::QueuesAmountDoNotMatch(num_queues, 1));
        }
        let queue = VirtQueue::new(0, 64, transport.as_mut()).expect("create virtqueue failed");
        let mut device = Self {
            config,
            queue: SpinLock::new(queue),
            transport,
        };

        device
            .transport
            .register_cfg_callback(Box::new(config_space_change))
            .unwrap();
        device
            .transport
            .register_queue_callback(0, Box::new(handle_block_device), false)
            .unwrap();

        fn handle_block_device(_: &TrapFrame) {
            jinux_block::get_device(super::DEVICE_NAME)
                .unwrap()
                .handle_irq();
        }

        fn config_space_change(_: &TrapFrame) {
            info!("Virtio block device config space change");
        }
        device.transport.finish_init();

        jinux_block::register_device(super::DEVICE_NAME.to_string(), Arc::new(device));

        Ok(())
    }

    /// Negotiate features for the device specified bits 0~23
    pub(crate) fn negotiate_features(features: u64) -> u64 {
        let feature = BlkFeatures::from_bits(features).unwrap();
        let support_features = BlkFeatures::from_bits(features).unwrap();
        (feature & support_features).bits
    }
}

impl jinux_block::BlockDevice for BlockDevice {
    fn read_block(&self, block_id: usize, buf: &mut [u8]) {
        self.read(block_id, buf);
    }

    fn write_block(&self, block_id: usize, buf: &[u8]) {
        self.write(block_id, buf);
    }

    fn handle_irq(&self) {
        info!("Virtio block device handle irq");
    }
}
