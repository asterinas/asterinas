use core::hint::spin_loop;

use alloc::vec::Vec;
use jinux_frame::offset_of;
use jinux_pci::{capability::vendor::virtio::CapabilityVirtioData, util::BAR};
use jinux_util::frame_ptr::InFramePtr;
use pod::Pod;

use crate::{
    device::block::{BlkReq, BlkResp, ReqType, RespStatus, BLK_SIZE},
    device::VirtioDeviceError,
    queue::{QueueError, VirtQueue},
    VitrioPciCommonCfg,
};

use super::{BLKFeatures, VirtioBLKConfig};

#[derive(Debug)]
pub struct BLKDevice {
    config: InFramePtr<VirtioBLKConfig>,
    queue: VirtQueue,
}

impl BLKDevice {
    /// Create a new VirtIO-Block driver.
    /// msix_vector_left should at least have one element or n elements where n is the virtqueue amount
    pub(crate) fn new(
        cap: &CapabilityVirtioData,
        bars: [Option<BAR>; 6],
        common_cfg: &InFramePtr<VitrioPciCommonCfg>,
        notify_base_address: usize,
        notify_off_multiplier: u32,
        mut msix_vector_left: Vec<u16>,
    ) -> Result<Self, VirtioDeviceError> {
        let config = VirtioBLKConfig::new(cap, bars);
        let num_queues = common_cfg.read_at(offset_of!(VitrioPciCommonCfg, num_queues)) as u16;
        if num_queues != 1 {
            return Err(VirtioDeviceError::QueuesAmountDoNotMatch(num_queues, 1));
        }
        let queue = VirtQueue::new(
            &common_cfg,
            0 as usize,
            128,
            notify_base_address as usize,
            notify_off_multiplier,
            msix_vector_left.pop().unwrap(),
        )
        .expect("create virtqueue failed");
        Ok(Self { config, queue })
    }

    /// Negotiate features for the device specified bits 0~23
    pub(crate) fn negotiate_features(features: u64) -> u64 {
        let feature = BLKFeatures::from_bits(features).unwrap();
        let support_features = BLKFeatures::from_bits(features).unwrap();
        (feature & support_features).bits
    }

    /// read data from block device, this function is blocking
    pub fn read_block(&mut self, block_id: usize, buf: &mut [u8]) {
        assert_eq!(buf.len(), BLK_SIZE);
        let req = BlkReq {
            type_: ReqType::In,
            reserved: 0,
            sector: block_id as u64,
        };
        let mut resp = BlkResp::default();
        let token = self
            .queue
            .add(&[req.as_bytes()], &[buf, resp.as_bytes_mut()])
            .expect("add queue failed");
        self.queue.notify();
        while !self.queue.can_pop() {
            spin_loop();
        }
        self.queue
            .pop_used_with_token(token)
            .expect("pop used failed");
        match resp.status {
            RespStatus::Ok => {}
            _ => panic!("io error in block device"),
        };
    }
    /// write data to block device, this function is blocking
    pub fn write_block(&mut self, block_id: usize, buf: &[u8]) {
        assert_eq!(buf.len(), BLK_SIZE);
        let req = BlkReq {
            type_: ReqType::Out,
            reserved: 0,
            sector: block_id as u64,
        };
        let mut resp = BlkResp::default();
        let token = self
            .queue
            .add(&[req.as_bytes(), buf], &[resp.as_bytes_mut()])
            .expect("add queue failed");
        self.queue.notify();
        while !self.queue.can_pop() {
            spin_loop();
        }
        self.queue
            .pop_used_with_token(token)
            .expect("pop used failed");
        let st = resp.status;
        match st {
            RespStatus::Ok => {}
            _ => panic!("io error in block device:{:?}", st),
        };
    }

    pub fn pop_used(&mut self) -> Result<(u16, u32), QueueError> {
        self.queue.pop_used()
    }

    pub fn pop_used_with_token(&mut self, token: u16) -> Result<u32, QueueError> {
        self.queue.pop_used_with_token(token)
    }

    /// read data from block device, this function is non-blocking
    /// return value is token
    pub fn read_block_non_blocking(
        &mut self,
        block_id: usize,
        buf: &mut [u8],
        req: &mut BlkReq,
        resp: &mut BlkResp,
    ) -> u16 {
        assert_eq!(buf.len(), BLK_SIZE);
        *req = BlkReq {
            type_: ReqType::In,
            reserved: 0,
            sector: block_id as u64,
        };
        let token = self
            .queue
            .add(&[req.as_bytes()], &[buf, resp.as_bytes_mut()])
            .unwrap();
        self.queue.notify();
        token
    }

    /// write data to block device, this function is non-blocking
    /// return value is token
    pub fn write_block_non_blocking(
        &mut self,
        block_id: usize,
        buf: &[u8],
        req: &mut BlkReq,
        resp: &mut BlkResp,
    ) -> u16 {
        assert_eq!(buf.len(), BLK_SIZE);
        *req = BlkReq {
            type_: ReqType::Out,
            reserved: 0,
            sector: block_id as u64,
        };
        let token = self
            .queue
            .add(&[req.as_bytes(), buf], &[resp.as_bytes_mut()])
            .expect("add queue failed");
        self.queue.notify();
        token
    }
}
