use core::{hint::spin_loop, mem::size_of};

use alloc::{boxed::Box, string::ToString, sync::Arc};
use alloc::{boxed::Box, string::ToString, sync::Arc, vec::Vec};
use aster_frame::{
    io_mem::IoMem,
    sync::SpinLock,
    trap::TrapFrame,
    vm::{VmAllocOptions, VmFrame, VmIo, VmReader, VmWriter},
};
use aster_util::safe_ptr::SafePtr;
use log::info;

use crate::{
    device::block::{BlkReq, BlkResp, ReqType, RespStatus},
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
    /// Block requests, we use VmFrame to store the requests so that
    /// it can pass to the `add_vm` function
    block_requests: VmFrame,
    /// Block responses, we use VmFrame to store the requests so that
    /// it can pass to the `add_vm` function
    block_responses: VmFrame,
    id_allocator: SpinLock<Vec<u8>>,
}

impl BlockDevice {
    /// read data from block device, this function is blocking
    /// FIEME: replace slice with a more secure data structure to use dma mapping.
    pub fn read(&self, block_id: usize, buf: &[VmWriter]) {
        // FIXME: Handling cases without id.
        let id = self.id_allocator.lock().pop().unwrap() as usize;
        let req = BlkReq {
            type_: ReqType::In as _,
            reserved: 0,
            sector: block_id as u64,
        };
        let resp = BlkResp::default();
        self.block_requests
            .write_val(id * size_of::<BlkReq>(), &req)
            .unwrap();
        self.block_responses
            .write_val(id * size_of::<BlkResp>(), &resp)
            .unwrap();
        let req = self
            .block_requests
            .reader()
            .skip(id * size_of::<BlkReq>())
            .limit(size_of::<BlkReq>());
        let resp = self
            .block_responses
            .writer()
            .skip(id * size_of::<BlkResp>())
            .limit(size_of::<BlkResp>());

        let mut outputs: Vec<&VmWriter<'_>> = buf.iter().collect();
        outputs.push(&resp);
        let mut queue = self.queue.lock_irq_disabled();
        let token = queue
            .add_vm(&[&req], outputs.as_slice())
            .expect("add queue failed");
        queue.notify();
        while !queue.can_pop() {
            spin_loop();
        }
        queue.pop_used_with_token(token).expect("pop used failed");
        let resp: BlkResp = self
            .block_responses
            .read_val(id * size_of::<BlkResp>())
            .unwrap();
        self.id_allocator.lock().push(id as u8);
        match RespStatus::try_from(resp.status).unwrap() {
            RespStatus::Ok => {}
            _ => panic!("io error in block device"),
        };
    }
    /// write data to block device, this function is blocking
    /// FIEME: replace slice with a more secure data structure to use dma mapping.
    pub fn write(&self, block_id: usize, buf: &[VmReader]) {
        // FIXME: Handling cases without id.
        let id = self.id_allocator.lock().pop().unwrap() as usize;
        let req = BlkReq {
            type_: ReqType::Out as _,
            reserved: 0,
            sector: block_id as u64,
        };
        let resp = BlkResp::default();
        self.block_requests
            .write_val(id * size_of::<BlkReq>(), &req)
            .unwrap();
        self.block_responses
            .write_val(id * size_of::<BlkResp>(), &resp)
            .unwrap();
        let req = self
            .block_requests
            .reader()
            .skip(id * size_of::<BlkReq>())
            .limit(size_of::<BlkReq>());
        let resp = self
            .block_responses
            .writer()
            .skip(id * size_of::<BlkResp>())
            .limit(size_of::<BlkResp>());

        let mut queue = self.queue.lock_irq_disabled();
        let mut inputs: Vec<&VmReader<'_>> = buf.iter().collect();
        inputs.insert(0, &req);
        let token = queue
            .add_vm(inputs.as_slice(), &[&resp])
            .expect("add queue failed");
        queue.notify();
        while !queue.can_pop() {
            spin_loop();
        }
        queue.pop_used_with_token(token).expect("pop used failed");
        let resp: BlkResp = self
            .block_responses
            .read_val(id * size_of::<BlkResp>())
            .unwrap();
        self.id_allocator.lock().push(id as u8);
        match RespStatus::try_from(resp.status).unwrap() {
            RespStatus::Ok => {}
            _ => panic!("io error in block device:{:?}", resp.status),
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
            block_requests: VmAllocOptions::new(1).alloc_single().unwrap(),
            block_responses: VmAllocOptions::new(1).alloc_single().unwrap(),
            id_allocator: SpinLock::new((0..64).collect()),
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
            aster_block::get_device(super::DEVICE_NAME)
                .unwrap()
                .handle_irq();
        }

        fn config_space_change(_: &TrapFrame) {
            info!("Virtio block device config space change");
        }
        device.transport.finish_init();

        aster_block::register_device(super::DEVICE_NAME.to_string(), Arc::new(device));

        Ok(())
    }

    /// Negotiate features for the device specified bits 0~23
    pub(crate) fn negotiate_features(features: u64) -> u64 {
        let feature = BlkFeatures::from_bits(features).unwrap();
        let support_features = BlkFeatures::from_bits(features).unwrap();
        (feature & support_features).bits
    }
}

impl aster_block::BlockDevice for BlockDevice {
    fn read_block(&self, block_id: usize, buf: &[VmWriter]) {
        self.read(block_id, buf);
    }

    fn write_block(&self, block_id: usize, buf: &[VmReader]) {
        self.write(block_id, buf);
    }

    fn handle_irq(&self) {
        info!("Virtio block device handle irq");
    }
}
