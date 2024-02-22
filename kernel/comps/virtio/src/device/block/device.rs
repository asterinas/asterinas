// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, string::String, sync::Arc, vec, vec::Vec};
use core::{fmt::Debug, hint::spin_loop, mem::size_of};

use aster_block::{
    bio::{BioEnqueueError, BioStatus, BioType, SubmittedBio},
    id::Sid,
    request_queue::{BioRequest, BioRequestSingleQueue},
};
use aster_frame::{
    io_mem::IoMem,
    sync::SpinLock,
    trap::TrapFrame,
    vm::{DmaDirection, DmaStream, DmaStreamSlice, VmAllocOptions, VmIo},
};
use aster_util::safe_ptr::SafePtr;
use log::info;
use pod::Pod;

use super::{BlockFeatures, VirtioBlockConfig};
use crate::{
    device::{
        block::{ReqType, RespStatus},
        VirtioDeviceError,
    },
    queue::VirtQueue,
    transport::VirtioTransport,
};

#[derive(Debug)]
pub struct BlockDevice {
    device: DeviceInner,
    /// The software staging queue.
    queue: BioRequestSingleQueue,
}

impl BlockDevice {
    /// Creates a new VirtIO-Block driver and registers it.
    pub(crate) fn init(transport: Box<dyn VirtioTransport>) -> Result<(), VirtioDeviceError> {
        let block_device = {
            let device = DeviceInner::init(transport)?;
            Self {
                device,
                queue: BioRequestSingleQueue::new(),
            }
        };

        let device_id = block_device.device.device_id.clone().unwrap();
        aster_block::register_device(device_id, Arc::new(block_device));
        Ok(())
    }

    /// Dequeues a `BioRequest` from the software staging queue and
    /// processes the request.
    ///
    /// TODO: Current read and write operations are still synchronous，
    /// it needs to be modified to use the queue-based asynchronous programming pattern.
    pub fn handle_requests(&self) {
        let request = self.queue.dequeue();
        match request.type_() {
            BioType::Read => self.do_read(&request),
            BioType::Write => self.do_write(&request),
            BioType::Flush | BioType::Discard => todo!(),
        }
    }

    fn do_read(&self, request: &BioRequest) {
        let start_sid = request.sid_range().start;
        let dma_streams: Vec<(DmaStream, usize, usize)> = request
            .bios()
            .flat_map(|bio| {
                bio.segments().iter().map(|segment| {
                    let dma_stream =
                        DmaStream::map(segment.pages().clone(), DmaDirection::FromDevice, false)
                            .unwrap();
                    (dma_stream, segment.offset(), segment.nbytes())
                })
            })
            .collect();
        let dma_slices: Vec<DmaStreamSlice> = dma_streams
            .iter()
            .map(|(stream, offset, len)| DmaStreamSlice::new(stream, *offset, *len))
            .collect();

        self.device.read(start_sid, &dma_slices);

        dma_slices.iter().for_each(|dma_slice| {
            dma_slice.sync().unwrap();
        });
        drop(dma_slices);
        drop(dma_streams);

        request.bios().for_each(|bio| {
            bio.complete(BioStatus::Complete);
        });
    }

    fn do_write(&self, request: &BioRequest) {
        let start_sid = request.sid_range().start;
        let dma_streams: Vec<(DmaStream, usize, usize)> = request
            .bios()
            .flat_map(|bio| {
                bio.segments().iter().map(|segment| {
                    let dma_stream =
                        DmaStream::map(segment.pages().clone(), DmaDirection::ToDevice, false)
                            .unwrap();
                    (dma_stream, segment.offset(), segment.nbytes())
                })
            })
            .collect();
        let dma_slices: Vec<DmaStreamSlice> = dma_streams
            .iter()
            .map(|(stream, offset, len)| DmaStreamSlice::new(stream, *offset, *len))
            .collect();

        self.device.write(start_sid, &dma_slices);
        drop(dma_slices);
        drop(dma_streams);

        request.bios().for_each(|bio| {
            bio.complete(BioStatus::Complete);
        });
    }

    /// Negotiate features for the device specified bits 0~23
    pub(crate) fn negotiate_features(features: u64) -> u64 {
        let feature = BlockFeatures::from_bits(features).unwrap();
        let support_features = BlockFeatures::from_bits(features).unwrap();
        (feature & support_features).bits
    }
}

impl aster_block::BlockDevice for BlockDevice {
    fn enqueue(&self, bio: SubmittedBio) -> Result<(), BioEnqueueError> {
        self.queue.enqueue(bio)
    }

    fn handle_irq(&self) {
        info!("Virtio block device handle irq");
    }
}

#[derive(Debug)]
struct DeviceInner {
    config: SafePtr<VirtioBlockConfig, IoMem>,
    queue: SpinLock<VirtQueue>,
    transport: Box<dyn VirtioTransport>,
    block_requests: DmaStream,
    block_responses: DmaStream,
    id_allocator: SpinLock<Vec<u8>>,
    device_id: Option<String>,
}

impl DeviceInner {
    /// Creates and inits the device.
    pub fn init(mut transport: Box<dyn VirtioTransport>) -> Result<Self, VirtioDeviceError> {
        let config = VirtioBlockConfig::new(transport.as_mut());
        let num_queues = transport.num_queues();
        if num_queues != 1 {
            return Err(VirtioDeviceError::QueuesAmountDoNotMatch(num_queues, 1));
        }

        let queue = VirtQueue::new(0, 64, transport.as_mut()).expect("create virtqueue failed");
        let block_requests = {
            let vm_segment = VmAllocOptions::new(1)
                .is_contiguous(true)
                .alloc_contiguous()
                .unwrap();
            DmaStream::map(vm_segment, DmaDirection::Bidirectional, false).unwrap()
        };
        let block_responses = {
            let vm_segment = VmAllocOptions::new(1)
                .is_contiguous(true)
                .alloc_contiguous()
                .unwrap();
            DmaStream::map(vm_segment, DmaDirection::Bidirectional, false).unwrap()
        };
        let mut device = Self {
            config,
            queue: SpinLock::new(queue),
            transport,
            block_requests,
            block_responses,
            id_allocator: SpinLock::new((0..64).collect()),
            device_id: None,
        };

        let device_id = device.get_id();
        let cloned_device_id = device_id.clone();

        let handle_block_device = move |_: &TrapFrame| {
            aster_block::get_device(device_id.as_str())
                .unwrap()
                .handle_irq();
        };

        device.device_id = Some(cloned_device_id);

        device
            .transport
            .register_cfg_callback(Box::new(config_space_change))
            .unwrap();

        device
            .transport
            .register_queue_callback(0, Box::new(handle_block_device), false)
            .unwrap();

        fn config_space_change(_: &TrapFrame) {
            info!("Virtio block device config space change");
        }
        device.transport.finish_init();
        Ok(device)
    }

    // TODO: Most logic is the same as read and write, there should be a refactor.
    // TODO: Should return an Err instead of panic if the device fails.
    fn get_id(&self) -> String {
        let id = self.id_allocator.lock().pop().unwrap() as usize;
        let req_slice = {
            let req_slice = DmaStreamSlice::new(&self.block_requests, id * REQ_SIZE, REQ_SIZE);
            let req = BlockReq {
                type_: ReqType::GetId as _,
                reserved: 0,
                sector: 0,
            };
            req_slice.write_val(0, &req).unwrap();
            req_slice.sync().unwrap();
            req_slice
        };

        let resp_slice = {
            let resp_slice = DmaStreamSlice::new(&self.block_responses, id * RESP_SIZE, RESP_SIZE);
            resp_slice.write_val(0, &BlockResp::default()).unwrap();
            resp_slice
        };
        const MAX_ID_LENGTH: usize = 20;
        let device_id_stream = {
            let segment = VmAllocOptions::new(1)
                .is_contiguous(true)
                .uninit(true)
                .alloc_contiguous()
                .unwrap();
            DmaStream::map(segment, DmaDirection::FromDevice, false).unwrap()
        };
        let device_id_slice = DmaStreamSlice::new(&device_id_stream, 0, MAX_ID_LENGTH);
        let outputs = vec![&device_id_slice, &resp_slice];

        let mut queue = self.queue.lock_irq_disabled();
        let token = queue
            .add_dma_buf(&[&req_slice], outputs.as_slice())
            .expect("add queue failed");
        queue.notify();
        while !queue.can_pop() {
            spin_loop();
        }
        queue.pop_used_with_token(token).expect("pop used failed");

        resp_slice.sync().unwrap();
        self.id_allocator.lock().push(id as u8);
        let resp: BlockResp = resp_slice.read_val(0).unwrap();
        match RespStatus::try_from(resp.status).unwrap() {
            RespStatus::Ok => {}
            _ => panic!("io error in block device"),
        };

        let device_id = {
            device_id_slice.sync().unwrap();
            let mut device_id = vec![0u8; MAX_ID_LENGTH];
            let _ = device_id_slice.read_bytes(0, &mut device_id);
            let len = device_id
                .iter()
                .position(|&b| b == 0)
                .unwrap_or(MAX_ID_LENGTH);
            device_id.truncate(len);
            device_id
        };
        String::from_utf8(device_id).unwrap()
    }

    /// Reads data from the block device, this function is blocking.
    pub fn read(&self, sector_id: Sid, bufs: &[DmaStreamSlice]) {
        // FIXME: Handling cases without id.
        let id = self.id_allocator.lock().pop().unwrap() as usize;

        let req_slice = {
            let req_slice = DmaStreamSlice::new(&self.block_requests, id * REQ_SIZE, REQ_SIZE);
            let req = BlockReq {
                type_: ReqType::In as _,
                reserved: 0,
                sector: sector_id.to_raw(),
            };
            req_slice.write_val(0, &req).unwrap();
            req_slice.sync().unwrap();
            req_slice
        };

        let resp_slice = {
            let resp_slice = DmaStreamSlice::new(&self.block_responses, id * RESP_SIZE, RESP_SIZE);
            resp_slice.write_val(0, &BlockResp::default()).unwrap();
            resp_slice
        };

        let outputs = {
            let mut outputs: Vec<&DmaStreamSlice> = bufs.iter().collect();
            outputs.push(&resp_slice);
            outputs
        };

        let mut queue = self.queue.lock_irq_disabled();
        let token = queue
            .add_dma_buf(&[&req_slice], outputs.as_slice())
            .expect("add queue failed");
        queue.notify();
        while !queue.can_pop() {
            spin_loop();
        }
        queue.pop_used_with_token(token).expect("pop used failed");

        resp_slice.sync().unwrap();
        let resp: BlockResp = resp_slice.read_val(0).unwrap();
        self.id_allocator.lock().push(id as u8);
        match RespStatus::try_from(resp.status).unwrap() {
            RespStatus::Ok => {}
            _ => panic!("io error in block device"),
        };
    }

    /// Writes data to the block device, this function is blocking.
    pub fn write(&self, sector_id: Sid, bufs: &[DmaStreamSlice]) {
        // FIXME: Handling cases without id.
        let id = self.id_allocator.lock().pop().unwrap() as usize;

        let req_slice = {
            let req_slice = DmaStreamSlice::new(&self.block_requests, id * REQ_SIZE, REQ_SIZE);
            let req = BlockReq {
                type_: ReqType::Out as _,
                reserved: 0,
                sector: sector_id.to_raw(),
            };
            req_slice.write_val(0, &req).unwrap();
            req_slice.sync().unwrap();
            req_slice
        };

        let resp_slice = {
            let resp_slice = DmaStreamSlice::new(&self.block_responses, id * RESP_SIZE, RESP_SIZE);
            resp_slice.write_val(0, &BlockResp::default()).unwrap();
            resp_slice
        };

        let inputs = {
            let mut inputs: Vec<&DmaStreamSlice> = bufs.iter().collect();
            inputs.insert(0, &req_slice);
            inputs
        };

        let mut queue = self.queue.lock_irq_disabled();
        let token = queue
            .add_dma_buf(inputs.as_slice(), &[&resp_slice])
            .expect("add queue failed");
        queue.notify();
        while !queue.can_pop() {
            spin_loop();
        }
        queue.pop_used_with_token(token).expect("pop used failed");

        resp_slice.sync().unwrap();
        let resp: BlockResp = resp_slice.read_val(0).unwrap();
        self.id_allocator.lock().push(id as u8);
        match RespStatus::try_from(resp.status).unwrap() {
            RespStatus::Ok => {}
            _ => panic!("io error in block device:{:?}", resp.status),
        };
    }
}

#[repr(C)]
#[derive(Debug, Copy, Clone, Pod)]
struct BlockReq {
    pub type_: u32,
    pub reserved: u32,
    pub sector: u64,
}

const REQ_SIZE: usize = size_of::<BlockReq>();

/// Response of a VirtIOBlock request.
#[repr(C)]
#[derive(Debug, Copy, Clone, Pod)]
struct BlockResp {
    pub status: u8,
}

const RESP_SIZE: usize = size_of::<BlockResp>();

impl Default for BlockResp {
    fn default() -> Self {
        Self {
            status: RespStatus::_NotReady as _,
        }
    }
}
