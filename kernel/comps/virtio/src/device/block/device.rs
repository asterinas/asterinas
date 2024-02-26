// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, collections::BTreeMap, string::String, sync::Arc, vec, vec::Vec};
use core::{fmt::Debug, hint::spin_loop, mem::size_of};

use aster_block::{
    bio::{BioEnqueueError, BioStatus, BioType, SubmittedBio},
    request_queue::{BioRequest, BioRequestSingleQueue},
};
use aster_frame::{
    io_mem::IoMem,
    sync::SpinLock,
    trap::TrapFrame,
    vm::{DmaDirection, DmaStream, DmaStreamSlice, VmAllocOptions, VmIo},
};
use aster_util::{id_allocator::IdAlloc, safe_ptr::SafePtr};
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
        let device = {
            let mut device = DeviceInner::init(transport)?;
            let device_id = device.get_id();
            device.finish_init(device_id);
            device
        };
        let device_id = device.device_id.clone().unwrap();

        let block_device = Arc::new(Self {
            device,
            queue: BioRequestSingleQueue::new(),
        });

        aster_block::register_device(device_id, block_device);
        Ok(())
    }

    /// Dequeues a `BioRequest` from the software staging queue and
    /// processes the request.
    pub fn handle_requests(&self) {
        let request = self.queue.dequeue();
        info!("Handle Request: {:?}", request);
        match request.type_() {
            BioType::Read => self.device.do_read(request),
            BioType::Write => self.device.do_write(request),
            BioType::Flush | BioType::Discard => todo!(),
        }
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
        self.device.do_handle_irq();
    }
}

#[derive(Debug)]
struct DeviceInner {
    config: SafePtr<VirtioBlockConfig, IoMem>,
    queue: SpinLock<VirtQueue>,
    transport: Box<dyn VirtioTransport>,
    block_requests: DmaStream,
    block_responses: DmaStream,
    device_id: Option<String>,
    id_allocator: SpinLock<IdAlloc>,
    submitted_requests: SpinLock<BTreeMap<u16, SubmittedRequest>>,
}

impl DeviceInner {
    const QUEUE_SIZE: u16 = 64;

    /// Creates and inits the device.
    fn init(mut transport: Box<dyn VirtioTransport>) -> Result<Self, VirtioDeviceError> {
        let config = VirtioBlockConfig::new(transport.as_mut());
        let num_queues = transport.num_queues();
        if num_queues != 1 {
            return Err(VirtioDeviceError::QueuesAmountDoNotMatch(num_queues, 1));
        }
        let queue = VirtQueue::new(0, Self::QUEUE_SIZE, transport.as_mut())
            .expect("create virtqueue failed");
        let block_requests = {
            let vm_segment = VmAllocOptions::new(1)
                .is_contiguous(true)
                .alloc_contiguous()
                .unwrap();
            DmaStream::map(vm_segment, DmaDirection::Bidirectional, false).unwrap()
        };
        assert!(Self::QUEUE_SIZE as usize * REQ_SIZE <= block_requests.nbytes());
        let block_responses = {
            let vm_segment = VmAllocOptions::new(1)
                .is_contiguous(true)
                .alloc_contiguous()
                .unwrap();
            DmaStream::map(vm_segment, DmaDirection::Bidirectional, false).unwrap()
        };
        assert!(Self::QUEUE_SIZE as usize * RESP_SIZE <= block_responses.nbytes());

        let mut device = Self {
            config,
            queue: SpinLock::new(queue),
            transport,
            block_requests,
            block_responses,
            device_id: None,
            id_allocator: SpinLock::new(IdAlloc::with_capacity(Self::QUEUE_SIZE as usize)),
            submitted_requests: SpinLock::new(BTreeMap::new()),
        };

        device
            .transport
            .register_cfg_callback(Box::new(config_space_change))
            .unwrap();

        device
            .transport
            .register_queue_callback(0, Box::new(dummy_handle_block_device), false)
            .unwrap();

        fn config_space_change(_: &TrapFrame) {
            info!("Virtio block device config space change");
        }

        fn dummy_handle_block_device(_: &TrapFrame) {
            info!("Virtio block device handle block device");
        }

        Ok(device)
    }

    /// Finalizes the device initialization and assigns the device ID.
    pub fn finish_init(&mut self, device_id: String) {
        assert!(self.device_id.is_none());

        let cloned_device_id = device_id.clone();
        self.device_id = Some(cloned_device_id);

        let handle_block_device = move |_: &TrapFrame| {
            aster_block::get_device(device_id.as_str())
                .unwrap()
                .handle_irq();
        };
        self.transport
            .register_queue_callback(0, Box::new(handle_block_device), false)
            .unwrap();

        self.transport.finish_init();
    }

    // TODO: Most logic is the same as read and write, there should be a refactor.
    // TODO: Should return an Err instead of panic if the device fails.
    fn get_id(&self) -> String {
        let id = self.id_allocator.lock().alloc().unwrap();
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
        if queue.should_notify() {
            queue.notify();
        }
        while !queue.can_pop() {
            spin_loop();
        }
        queue.pop_used_with_token(token).expect("pop used failed");

        resp_slice.sync().unwrap();
        self.id_allocator.lock().free(id);
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

    /// Handles the irq issued from the device
    fn do_handle_irq(&self) {
        loop {
            // Pops the complete request
            let Some(complete_request) = ({
                let mut queue = self.queue.lock_irq_disabled();
                let Ok((token, _)) = queue.pop_used() else {
                    return;
                };
                self.submitted_requests.lock().remove(&token)
            }) else {
                continue;
            };

            // Handles the response
            let id = complete_request.id as usize;
            let resp_slice = DmaStreamSlice::new(&self.block_responses, id * RESP_SIZE, RESP_SIZE);
            resp_slice.sync().unwrap();
            let resp: BlockResp = resp_slice.read_val(0).unwrap();
            self.id_allocator.lock().free(id);
            match RespStatus::try_from(resp.status).unwrap() {
                RespStatus::Ok => {}
                // FIXME: Return an error instead of triggering a kernel panic
                _ => panic!("io error in block device"),
            };

            // Synchronize DMA mapping if read from the device
            if let BioType::Read = complete_request.bio_request.type_() {
                complete_request
                    .bufs
                    .iter()
                    .for_each(|(stream, offset, len)| {
                        stream.sync(*offset..*offset + *len).unwrap();
                    });
            }

            // Completes the bio request
            complete_request.bio_request.bios().for_each(|bio| {
                bio.complete(BioStatus::Complete);
            });
        }
    }

    /// Reads data from the device, this function is no-blocking.
    fn do_read(&self, bio_request: BioRequest) {
        let dma_streams: Vec<(DmaStream, usize, usize)> = bio_request
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

        let id = self.id_allocator.lock().alloc().unwrap();
        let req_slice = {
            let req_slice = DmaStreamSlice::new(&self.block_requests, id * REQ_SIZE, REQ_SIZE);
            let req = BlockReq {
                type_: ReqType::In as _,
                reserved: 0,
                sector: bio_request.sid_range().start.to_raw(),
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
            let mut outputs: Vec<&DmaStreamSlice> = Vec::with_capacity(dma_slices.len() + 1);
            outputs.extend(dma_slices.iter());
            outputs.push(&resp_slice);
            outputs
        };

        let mut queue = self.queue.lock_irq_disabled();
        let token = queue
            .add_dma_buf(&[&req_slice], outputs.as_slice())
            .expect("add queue failed");
        if queue.should_notify() {
            queue.notify();
        }

        // Records the submitted request
        let submitted_request = SubmittedRequest::new(id as u16, bio_request, dma_streams);
        self.submitted_requests
            .lock()
            .insert(token, submitted_request);
    }

    /// Writes data to the device, this function is no-blocking.
    fn do_write(&self, bio_request: BioRequest) {
        let dma_streams: Vec<(DmaStream, usize, usize)> = bio_request
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

        let id = self.id_allocator.lock().alloc().unwrap();
        let req_slice = {
            let req_slice = DmaStreamSlice::new(&self.block_requests, id * REQ_SIZE, REQ_SIZE);
            let req = BlockReq {
                type_: ReqType::Out as _,
                reserved: 0,
                sector: bio_request.sid_range().start.to_raw(),
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
            let mut inputs: Vec<&DmaStreamSlice> = Vec::with_capacity(dma_slices.len() + 1);
            inputs.push(&req_slice);
            inputs.extend(dma_slices.iter());
            inputs
        };

        let mut queue = self.queue.lock_irq_disabled();
        let token = queue
            .add_dma_buf(inputs.as_slice(), &[&resp_slice])
            .expect("add queue failed");
        if queue.should_notify() {
            queue.notify();
        }

        // Records the submitted request
        let submitted_request = SubmittedRequest::new(id as u16, bio_request, dma_streams);
        self.submitted_requests
            .lock()
            .insert(token, submitted_request);
    }
}

#[derive(Debug)]
struct SubmittedRequest {
    id: u16,
    bio_request: BioRequest,
    bufs: Vec<(DmaStream, usize, usize)>,
}

impl SubmittedRequest {
    pub fn new(id: u16, bio_request: BioRequest, bufs: Vec<(DmaStream, usize, usize)>) -> Self {
        Self {
            id,
            bio_request,
            bufs,
        }
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
