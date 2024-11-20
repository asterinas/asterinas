// SPDX-License-Identifier: MPL-2.0

use alloc::{
    boxed::Box,
    collections::BTreeMap,
    string::{String, ToString},
    sync::Arc,
    vec,
    vec::Vec,
};
use core::{fmt::Debug, hint::spin_loop, mem::size_of};

use aster_block::{
    bio::{bio_segment_pool_init, BioEnqueueError, BioStatus, BioType, SubmittedBio},
    request_queue::{BioRequest, BioRequestSingleQueue},
    BlockDeviceMeta,
};
use id_alloc::IdAlloc;
use log::{debug, info};
use ostd::{
    mm::{DmaDirection, DmaStream, DmaStreamSlice, FrameAllocOptions, VmIo},
    sync::SpinLock,
    trap::TrapFrame,
    Pod,
};

use super::{BlockFeatures, VirtioBlockConfig, VirtioBlockFeature};
use crate::{
    device::{
        block::{ReqType, RespStatus},
        VirtioDeviceError,
    },
    queue::VirtQueue,
    transport::{ConfigManager, VirtioTransport},
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
        let is_legacy = transport.is_legacy_version();
        let device = DeviceInner::init(transport)?;
        let device_id = if is_legacy {
            // FIXME: legacy device do not support `GetId` request.
            "legacy_blk".to_string()
        } else {
            device.request_device_id()
        };

        let block_device = Arc::new(Self {
            device,
            // Each bio request includes an additional 1 request and 1 response descriptor,
            // therefore this upper bound is set to (QUEUE_SIZE - 2).
            queue: BioRequestSingleQueue::with_max_nr_segments_per_bio(
                (DeviceInner::QUEUE_SIZE - 2) as usize,
            ),
        });

        aster_block::register_device(device_id, block_device);

        bio_segment_pool_init();
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
            BioType::Flush => self.device.flush(request),
            BioType::Discard => todo!(),
        }
    }

    /// Negotiate features for the device specified bits 0~23
    pub(crate) fn negotiate_features(features: u64) -> u64 {
        let mut support_features = BlockFeatures::from_bits_truncate(features);
        support_features.remove(BlockFeatures::MQ);
        support_features.bits
    }
}

impl aster_block::BlockDevice for BlockDevice {
    fn enqueue(&self, bio: SubmittedBio) -> Result<(), BioEnqueueError> {
        self.queue.enqueue(bio)
    }

    fn metadata(&self) -> BlockDeviceMeta {
        BlockDeviceMeta {
            max_nr_segments_per_bio: self.queue.max_nr_segments_per_bio(),
            nr_sectors: self.device.config_manager.capacity_sectors(),
        }
    }
}

#[derive(Debug)]
struct DeviceInner {
    config_manager: ConfigManager<VirtioBlockConfig>,
    features: VirtioBlockFeature,
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
        let config_manager = VirtioBlockConfig::new_manager(transport.as_ref());
        debug!("virio_blk_config = {:?}", config_manager.read_config());
        assert_eq!(
            config_manager.block_size(),
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
        let features = VirtioBlockFeature::new(transport.as_ref());
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
            config_manager,
            features,
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

        {
            let mut transport = device.transport.lock();
            transport
                .register_cfg_callback(Box::new(handle_config_change))
                .unwrap();
            transport
                .register_queue_callback(0, Box::new(handle_irq), false)
                .unwrap();
            transport.finish_init();
        }

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
                let mut queue = self.queue.lock();
                let Ok((token, _)) = queue.pop_used() else {
                    return;
                };
                self.submitted_requests.lock().remove(&token).unwrap()
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
                    .bio_request
                    .bios()
                    .flat_map(|bio| {
                        bio.segments()
                            .iter()
                            .map(|segment| segment.inner_dma_slice())
                    })
                    .for_each(|dma_slice| dma_slice.sync().unwrap());
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
        let id = self.id_allocator.disable_irq().lock().alloc().unwrap();
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

        let mut queue = self.queue.disable_irq().lock();
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
        self.id_allocator.disable_irq().lock().free(id);
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

    /// Reads data from the device, this function is non-blocking.
    fn read(&self, bio_request: BioRequest) {
        let id = self.id_allocator.disable_irq().lock().alloc().unwrap();
        let req_slice = {
            let req_slice =
                DmaStreamSlice::new(self.block_requests.clone(), id * REQ_SIZE, REQ_SIZE);
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
            let resp_slice =
                DmaStreamSlice::new(self.block_responses.clone(), id * RESP_SIZE, RESP_SIZE);
            resp_slice.write_val(0, &BlockResp::default()).unwrap();
            resp_slice
        };

        let outputs = {
            let mut outputs: Vec<&DmaStreamSlice<_>> =
                Vec::with_capacity(bio_request.num_segments() + 1);
            let dma_slices_iter = bio_request.bios().flat_map(|bio| {
                bio.segments()
                    .iter()
                    .map(|segment| segment.inner_dma_slice())
            });
            outputs.extend(dma_slices_iter);
            outputs.push(&resp_slice);
            outputs
        };

        let num_used_descs = outputs.len() + 1;
        // FIXME: Split the request if it is too big
        if num_used_descs > Self::QUEUE_SIZE as usize {
            panic!("The request size surpasses the queue size");
        }

        loop {
            let mut queue = self.queue.disable_irq().lock();
            if num_used_descs > queue.available_desc() {
                continue;
            }
            let token = queue
                .add_dma_buf(&[&req_slice], outputs.as_slice())
                .expect("add queue failed");
            if queue.should_notify() {
                queue.notify();
            }

            // Records the submitted request
            let submitted_request = SubmittedRequest::new(id as u16, bio_request);
            self.submitted_requests
                .disable_irq()
                .lock()
                .insert(token, submitted_request);
            return;
        }
    }

    /// Writes data to the device, this function is non-blocking.
    fn write(&self, bio_request: BioRequest) {
        let id = self.id_allocator.disable_irq().lock().alloc().unwrap();
        let req_slice = {
            let req_slice =
                DmaStreamSlice::new(self.block_requests.clone(), id * REQ_SIZE, REQ_SIZE);
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
            let resp_slice =
                DmaStreamSlice::new(self.block_responses.clone(), id * RESP_SIZE, RESP_SIZE);
            resp_slice.write_val(0, &BlockResp::default()).unwrap();
            resp_slice
        };

        let inputs = {
            let mut inputs: Vec<&DmaStreamSlice<_>> =
                Vec::with_capacity(bio_request.num_segments() + 1);
            inputs.push(&req_slice);
            let dma_slices_iter = bio_request.bios().flat_map(|bio| {
                bio.segments()
                    .iter()
                    .map(|segment| segment.inner_dma_slice())
            });
            inputs.extend(dma_slices_iter);
            inputs
        };

        let num_used_descs = inputs.len() + 1;
        // FIXME: Split the request if it is too big
        if num_used_descs > Self::QUEUE_SIZE as usize {
            panic!("The request size surpasses the queue size");
        }
        loop {
            let mut queue = self.queue.disable_irq().lock();
            if num_used_descs > queue.available_desc() {
                continue;
            }
            let token = queue
                .add_dma_buf(inputs.as_slice(), &[&resp_slice])
                .expect("add queue failed");
            if queue.should_notify() {
                queue.notify();
            }

            // Records the submitted request
            let submitted_request = SubmittedRequest::new(id as u16, bio_request);
            self.submitted_requests
                .disable_irq()
                .lock()
                .insert(token, submitted_request);
            return;
        }
    }

    /// Flushes any cached data from the guest to the persistent storage on the host.
    /// This will be ignored if the device doesn't support the `VIRTIO_BLK_F_FLUSH` feature.
    fn flush(&self, bio_request: BioRequest) {
        if self.features.support_flush {
            bio_request.bios().for_each(|bio| {
                bio.complete(BioStatus::Complete);
            });
            return;
        }

        let id = self.id_allocator.disable_irq().lock().alloc().unwrap();
        let req_slice = {
            let req_slice = DmaStreamSlice::new(&self.block_requests, id * REQ_SIZE, REQ_SIZE);
            let req = BlockReq {
                type_: ReqType::Flush as _,
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

        let num_used_descs = 1;
        loop {
            let mut queue = self.queue.disable_irq().lock();
            if num_used_descs > queue.available_desc() {
                continue;
            }
            let token = queue
                .add_dma_buf(&[&req_slice], &[&resp_slice])
                .expect("add queue failed");
            if queue.should_notify() {
                queue.notify();
            }

            // Records the submitted request
            let submitted_request = SubmittedRequest::new(id as u16, bio_request);
            self.submitted_requests
                .disable_irq()
                .lock()
                .insert(token, submitted_request);
            return;
        }
    }
}

/// A submitted bio request for callback.
#[derive(Debug)]
struct SubmittedRequest {
    id: u16,
    bio_request: BioRequest,
}

impl SubmittedRequest {
    pub fn new(id: u16, bio_request: BioRequest) -> Self {
        Self { id, bio_request }
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
