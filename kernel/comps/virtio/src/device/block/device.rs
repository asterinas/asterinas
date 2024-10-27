// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, collections::BTreeMap, string::String, sync::Arc, vec, vec::Vec};
use core::{fmt::Debug, hint::spin_loop, mem::size_of};

use aster_block::{
    bio::{BioEnqueueError, BioStatus, BioType, SubmittedBio},
    request_queue::{BioRequest, BioRequestSingleQueue},
    BlockDeviceMeta,
};
use aster_util::safe_ptr::SafePtr;
use id_alloc::IdAlloc;
use log::info;
use ostd::{
    io_mem::IoMem,
    mm::{DmaDirection, DmaStream, DmaStreamSlice, FrameAllocOptions, VmIo},
    sync::SpinLock,
    trap::TrapFrame,
    Pod,
};

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
    device: Arc<DeviceInner>,
    /// The software staging queue.
    queue: BioRequestSingleQueue,
}

impl BlockDevice {
    /// Creates a new VirtIO-Block driver and registers it.
    pub(crate) fn init(transport: Box<dyn VirtioTransport>) -> Result<(), VirtioDeviceError> {
        let device = DeviceInner::init(transport)?;
        let device_id = device.request_device_id();

        let block_device = Arc::new(Self {
            device,
            // Each bio request includes an additional 1 request and 1 response descriptor,
            // therefore this upper bound is set to (QUEUE_SIZE - 2).
            queue: BioRequestSingleQueue::with_max_nr_segments_per_bio(
                (DeviceInner::QUEUE_SIZE - 2) as usize,
            ),
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
            BioType::Read => self.device.read(request),
            BioType::Write => self.device.write(request),
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

    fn metadata(&self) -> BlockDeviceMeta {
        BlockDeviceMeta {
            max_nr_segments_per_bio: self.queue.max_nr_segments_per_bio(),
            nr_sectors: VirtioBlockConfig::read_capacity_sectors(&self.device.config).unwrap(),
        }
    }
}

#[derive(Debug)]
struct DeviceInner {
    config: SafePtr<VirtioBlockConfig, IoMem>,
    queue: SpinLock<VirtQueue>,
    transport: SpinLock<Box<dyn VirtioTransport>>,
    block_requests: DmaStream,
    block_responses: DmaStream,
    id_allocator: SpinLock<IdAlloc>,
    submitted_requests: SpinLock<BTreeMap<u16, SubmittedRequest>>,
}

impl DeviceInner {
    const QUEUE_SIZE: u16 = 64;

    /// Creates and inits the device.
    pub fn init(mut transport: Box<dyn VirtioTransport>) -> Result<Arc<Self>, VirtioDeviceError> {
        let config = VirtioBlockConfig::new(transport.as_mut());
        assert_eq!(
            VirtioBlockConfig::read_block_size(&config).unwrap(),
            VirtioBlockConfig::sector_size(),
            "currently not support customized device logical block size"
        );
        let num_queues = transport.num_queues();
        if num_queues != 1 {
            // FIXME: support Multi-Queue Block IO Queueing Mechanism
            // (`BlkFeatures::MQ`) to accelerate multi-processor requests for
            // block devices. When SMP is enabled on x86, the feature is on.
            // We should also consider negotiating the feature in the future.
            // return Err(VirtioDeviceError::QueuesAmountDoNotMatch(num_queues, 1));
            log::warn!(
                "Not supporting Multi-Queue Block IO Queueing Mechanism, only using the first queue"
            );
        }
        let queue = VirtQueue::new(0, Self::QUEUE_SIZE, transport.as_mut())
            .expect("create virtqueue failed");
        let block_requests = {
            let vm_segment = FrameAllocOptions::new(1).alloc_contiguous().unwrap();
            DmaStream::map(vm_segment, DmaDirection::Bidirectional, false).unwrap()
        };
        assert!(Self::QUEUE_SIZE as usize * REQ_SIZE <= block_requests.nbytes());
        let block_responses = {
            let vm_segment = FrameAllocOptions::new(1).alloc_contiguous().unwrap();
            DmaStream::map(vm_segment, DmaDirection::Bidirectional, false).unwrap()
        };
        assert!(Self::QUEUE_SIZE as usize * RESP_SIZE <= block_responses.nbytes());

        let device = Arc::new(Self {
            config,
            queue: SpinLock::new(queue),
            transport: SpinLock::new(transport),
            block_requests,
            block_responses,
            id_allocator: SpinLock::new(IdAlloc::with_capacity(Self::QUEUE_SIZE as usize)),
            submitted_requests: SpinLock::new(BTreeMap::new()),
        });

        let cloned_device = device.clone();
        let handle_irq = move |_: &TrapFrame| {
            cloned_device.handle_irq();
        };

        let cloned_device = device.clone();
        let handle_config_change = move |_: &TrapFrame| {
            cloned_device.handle_config_change();
        };

        device.transport.lock_with(|transport| {
            transport
                .register_cfg_callback(Box::new(handle_config_change))
                .unwrap();
            transport
                .register_queue_callback(0, Box::new(handle_irq), false)
                .unwrap();
            transport.finish_init();
        });

        Ok(device)
    }

    /// Handles the irq issued from the device
    fn handle_irq(&self) {
        info!("Virtio block device handle irq");
        // When we enter the IRQs handling function,
        // IRQs have already been disabled,
        // so there is no need to call `disable_irq`.
        loop {
            // Pops the complete request
            let complete_request = {
                let Some(req) = self.queue.lock_with(|queue| {
                    let Ok((token, _)) = queue.pop_used() else {
                        return None;
                    };
                    Some(
                        self.submitted_requests
                            .lock_with(|rqs| rqs.remove(&token).unwrap()),
                    )
                }) else {
                    return;
                };
                req
            };

            // Handles the response
            let id = complete_request.id as usize;
            let resp_slice = DmaStreamSlice::new(&self.block_responses, id * RESP_SIZE, RESP_SIZE);
            resp_slice.sync().unwrap();
            let resp: BlockResp = resp_slice.read_val(0).unwrap();
            self.id_allocator.lock_with(|a| a.free(id));
            match RespStatus::try_from(resp.status).unwrap() {
                RespStatus::Ok => {}
                // FIXME: Return an error instead of triggering a kernel panic
                _ => panic!("io error in block device"),
            };

            // Synchronize DMA mapping if read from the device
            if let BioType::Read = complete_request.bio_request.type_() {
                complete_request
                    .dma_bufs
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

    fn handle_config_change(&self) {
        info!("Virtio block device config space change");
    }

    // TODO: Most logic is the same as read and write, there should be a refactor.
    // TODO: Should return an Err instead of panic if the device fails.
    fn request_device_id(&self) -> String {
        let id = self
            .id_allocator
            .disable_irq()
            .lock_with(|a| a.alloc())
            .unwrap();
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
            let segment = FrameAllocOptions::new(1)
                .uninit(true)
                .alloc_contiguous()
                .unwrap();
            DmaStream::map(segment, DmaDirection::FromDevice, false).unwrap()
        };
        let device_id_slice = DmaStreamSlice::new(&device_id_stream, 0, MAX_ID_LENGTH);
        let outputs = vec![&device_id_slice, &resp_slice];

        self.queue.disable_irq().lock_with(|queue| {
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
            self.id_allocator.disable_irq().lock_with(|a| a.free(id));
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
        })
    }

    /// Reads data from the device, this function is non-blocking.
    fn read(&self, bio_request: BioRequest) {
        let dma_streams = Self::dma_stream_map(&bio_request);

        let id = self
            .id_allocator
            .disable_irq()
            .lock_with(|a| a.alloc())
            .unwrap();

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

        let num_used_descs = dma_streams.len() + 1;
        // FIXME: Split the request if it is too big
        if num_used_descs > Self::QUEUE_SIZE as usize {
            panic!("The request size surpasses the queue size");
        }

        // Call this closure once the queue is ready
        self.once_queue_ready(num_used_descs, |queue: &mut VirtQueue| {
            let resp_slice = {
                let resp_slice =
                    DmaStreamSlice::new(&self.block_responses, id * RESP_SIZE, RESP_SIZE);
                resp_slice.write_val(0, &BlockResp::default()).unwrap();
                resp_slice
            };

            let dma_slices: Vec<DmaStreamSlice> = dma_streams
                .iter()
                .map(|(stream, offset, len)| DmaStreamSlice::new(stream, *offset, *len))
                .collect();

            let outputs = {
                let mut outputs: Vec<&DmaStreamSlice> = Vec::with_capacity(num_used_descs - 1);
                outputs.extend(dma_slices.iter());
                outputs.push(&resp_slice);
                outputs
            };

            debug_assert_eq!(num_used_descs, outputs.len());

            let token = queue
                .add_dma_buf(&[&req_slice], outputs.as_slice())
                .expect("add queue failed");
            if queue.should_notify() {
                queue.notify();
            }

            // Records the submitted request
            let submitted_request = SubmittedRequest::new(id as u16, bio_request, dma_streams);
            self.submitted_requests
                .disable_irq()
                .lock_with(|rqs| rqs.insert(token, submitted_request));
        });
    }

    /// Writes data to the device, this function is non-blocking.
    fn write(&self, bio_request: BioRequest) {
        let dma_streams = Self::dma_stream_map(&bio_request);

        let id = self
            .id_allocator
            .disable_irq()
            .lock_with(|a| a.alloc())
            .unwrap();

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

        let num_used_descs = dma_streams.len() + 1;
        // FIXME: Split the request if it is too big
        if num_used_descs > Self::QUEUE_SIZE as usize {
            panic!("The request size surpasses the queue size");
        }

        self.once_queue_ready(num_used_descs, |queue: &mut VirtQueue| {
            let resp_slice = {
                let resp_slice =
                    DmaStreamSlice::new(&self.block_responses, id * RESP_SIZE, RESP_SIZE);
                resp_slice.write_val(0, &BlockResp::default()).unwrap();
                resp_slice
            };

            let dma_slices: Vec<DmaStreamSlice> = dma_streams
                .iter()
                .map(|(stream, offset, len)| DmaStreamSlice::new(stream, *offset, *len))
                .collect();

            let inputs = {
                let mut inputs: Vec<&DmaStreamSlice> = Vec::with_capacity(num_used_descs);
                inputs.push(&req_slice);
                inputs.extend(dma_slices.iter());
                inputs
            };

            debug_assert_eq!(num_used_descs, inputs.len());

            let token = queue
                .add_dma_buf(inputs.as_slice(), &[&resp_slice])
                .expect("add queue failed");
            if queue.should_notify() {
                queue.notify();
            }

            // Records the submitted request
            let submitted_request = SubmittedRequest::new(id as u16, bio_request, dma_streams);
            self.submitted_requests
                .disable_irq()
                .lock_with(|rqs| rqs.insert(token, submitted_request));
        });
    }

    /// Calls the closure once the queue have enough available descriptors.
    fn once_queue_ready<F, R>(&self, num_avail_descs: usize, f: F) -> R
    where
        F: FnOnce(&mut VirtQueue) -> R,
    {
        let mut once_queue_ready = Some(f);

        loop {
            if let Some(r) = self.queue.disable_irq().lock_with(|queue| {
                if queue.available_desc() < num_avail_descs {
                    return None; // retry
                }

                Some(once_queue_ready.take().unwrap().call_once((queue,)))
            }) {
                return r;
            }
        }
    }

    /// Performs DMA mapping for the segments in bio request.
    fn dma_stream_map(bio_request: &BioRequest) -> Vec<(DmaStream, usize, usize)> {
        let dma_direction = match bio_request.type_() {
            BioType::Read => DmaDirection::FromDevice,
            BioType::Write => DmaDirection::ToDevice,
            _ => todo!(),
        };

        bio_request
            .bios()
            .flat_map(|bio| {
                bio.segments().iter().map(|segment| {
                    let dma_stream =
                        DmaStream::map(segment.pages().clone().into(), dma_direction, false)
                            .unwrap();
                    (dma_stream, segment.offset(), segment.nbytes())
                })
            })
            .collect()
    }
}

/// A submitted bio request for callback.
#[derive(Debug)]
struct SubmittedRequest {
    id: u16,
    bio_request: BioRequest,
    dma_bufs: Vec<(DmaStream, usize, usize)>,
}

impl SubmittedRequest {
    pub fn new(id: u16, bio_request: BioRequest, dma_bufs: Vec<(DmaStream, usize, usize)>) -> Self {
        Self {
            id,
            bio_request,
            dma_bufs,
        }
    }
}

/// VirtIOBlock request.
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
