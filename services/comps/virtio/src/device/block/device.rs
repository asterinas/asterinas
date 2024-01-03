// SPDX-License-Identifier: MPL-2.0

use core::{
    fmt::Debug,
    hint::spin_loop,
    mem::size_of,
    sync::atomic::{AtomicUsize, Ordering},
};

use alloc::{boxed::Box, collections::VecDeque, string::ToString, sync::Arc, vec::Vec};
use aster_block::{
    bio::{BioEnqueueError, BioStatus, BioType, SubmittedBio},
    id::Sid,
    request_queue::{BioRequest, BioRequestQueue},
};
use aster_frame::{
    io_mem::IoMem,
    sync::SpinLock,
    sync::{Mutex, WaitQueue},
    trap::TrapFrame,
    vm::{VmAllocOptions, VmFrame, VmIo, VmReader, VmWriter},
};
use aster_util::safe_ptr::SafePtr;
use log::info;
use pod::Pod;

use crate::{
    device::block::{ReqType, RespStatus},
    device::VirtioDeviceError,
    queue::VirtQueue,
    transport::VirtioTransport,
};

use super::{BlockFeatures, VirtioBlockConfig};

#[derive(Debug)]
pub struct BlockDevice {
    device: DeviceInner,
    /// The software staging queue.
    queue: BioSingleQueue,
}

impl BlockDevice {
    /// Creates a new VirtIO-Block driver and registers it.
    pub(crate) fn init(transport: Box<dyn VirtioTransport>) -> Result<(), VirtioDeviceError> {
        let block_device = {
            let device = DeviceInner::init(transport)?;
            Self {
                device,
                queue: BioSingleQueue::new(),
            }
        };
        aster_block::register_device(super::DEVICE_NAME.to_string(), Arc::new(block_device));
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

        let writers = {
            let mut writers = Vec::new();
            for bio in request.bios() {
                for segment in bio.segments() {
                    writers.push(segment.writer());
                }
            }
            writers
        };

        self.device.read(start_sid, writers.as_slice());

        for bio in request.bios() {
            bio.complete(BioStatus::Complete);
        }
    }

    fn do_write(&self, request: &BioRequest) {
        let start_sid = request.sid_range().start;

        let readers = {
            let mut readers = Vec::new();
            for bio in request.bios() {
                for segment in bio.segments() {
                    readers.push(segment.reader());
                }
            }
            readers
        };

        self.device.write(start_sid, readers.as_slice());

        for bio in request.bios() {
            bio.complete(BioStatus::Complete);
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
    fn request_queue(&self) -> &dyn BioRequestQueue {
        &self.queue
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
    /// Block requests, we use VmFrame to store the requests so that
    /// it can pass to the `add_vm` function
    block_requests: VmFrame,
    /// Block responses, we use VmFrame to store the requests so that
    /// it can pass to the `add_vm` function
    block_responses: VmFrame,
    id_allocator: SpinLock<Vec<u8>>,
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
        Ok(device)
    }

    /// Reads data from the block device, this function is blocking.
    /// FIEME: replace slice with a more secure data structure to use dma mapping.
    pub fn read(&self, sector_id: Sid, buf: &[VmWriter]) {
        // FIXME: Handling cases without id.
        let id = self.id_allocator.lock().pop().unwrap() as usize;
        let req = BlockReq {
            type_: ReqType::In as _,
            reserved: 0,
            sector: sector_id.to_raw(),
        };
        let resp = BlockResp::default();
        self.block_requests
            .write_val(id * size_of::<BlockReq>(), &req)
            .unwrap();
        self.block_responses
            .write_val(id * size_of::<BlockResp>(), &resp)
            .unwrap();
        let req_reader = self
            .block_requests
            .reader()
            .skip(id * size_of::<BlockReq>())
            .limit(size_of::<BlockReq>());
        let resp_writer = self
            .block_responses
            .writer()
            .skip(id * size_of::<BlockResp>())
            .limit(size_of::<BlockResp>());

        let mut outputs: Vec<&VmWriter<'_>> = buf.iter().collect();
        outputs.push(&resp_writer);
        let mut queue = self.queue.lock_irq_disabled();
        let token = queue
            .add_vm(&[&req_reader], outputs.as_slice())
            .expect("add queue failed");
        queue.notify();
        while !queue.can_pop() {
            spin_loop();
        }
        queue.pop_used_with_token(token).expect("pop used failed");
        let resp: BlockResp = self
            .block_responses
            .read_val(id * size_of::<BlockResp>())
            .unwrap();
        self.id_allocator.lock().push(id as u8);
        match RespStatus::try_from(resp.status).unwrap() {
            RespStatus::Ok => {}
            _ => panic!("io error in block device"),
        };
    }

    /// Writes data to the block device, this function is blocking.
    /// FIEME: replace slice with a more secure data structure to use dma mapping.
    pub fn write(&self, sector_id: Sid, buf: &[VmReader]) {
        // FIXME: Handling cases without id.
        let id = self.id_allocator.lock().pop().unwrap() as usize;
        let req = BlockReq {
            type_: ReqType::Out as _,
            reserved: 0,
            sector: sector_id.to_raw(),
        };
        let resp = BlockResp::default();
        self.block_requests
            .write_val(id * size_of::<BlockReq>(), &req)
            .unwrap();
        self.block_responses
            .write_val(id * size_of::<BlockResp>(), &resp)
            .unwrap();
        let req_reader = self
            .block_requests
            .reader()
            .skip(id * size_of::<BlockReq>())
            .limit(size_of::<BlockReq>());
        let resp_writer = self
            .block_responses
            .writer()
            .skip(id * size_of::<BlockResp>())
            .limit(size_of::<BlockResp>());

        let mut queue = self.queue.lock_irq_disabled();
        let mut inputs: Vec<&VmReader<'_>> = buf.iter().collect();
        inputs.insert(0, &req_reader);
        let token = queue
            .add_vm(inputs.as_slice(), &[&resp_writer])
            .expect("add queue failed");
        queue.notify();
        while !queue.can_pop() {
            spin_loop();
        }
        queue.pop_used_with_token(token).expect("pop used failed");
        let resp: BlockResp = self
            .block_responses
            .read_val(id * size_of::<BlockResp>())
            .unwrap();
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

/// Response of a VirtIOBlock request.
#[repr(C)]
#[derive(Debug, Copy, Clone, Pod)]
struct BlockResp {
    pub status: u8,
}

impl Default for BlockResp {
    fn default() -> Self {
        Self {
            status: RespStatus::_NotReady as _,
        }
    }
}

/// A simple block I/O request queue with a single queue.
///
/// It is a FIFO producer-consumer queue, where the producer (e.g., filesystem)
/// submits requests to the queue, and the consumer (e.g., block device driver)
/// continuously consumes and processes these requests from the queue.
pub struct BioSingleQueue {
    queue: Mutex<VecDeque<BioRequest>>,
    num_requests: AtomicUsize,
    wait_queue: WaitQueue,
}

impl BioSingleQueue {
    /// Creates an empty queue.
    pub fn new() -> Self {
        Self {
            queue: Mutex::new(VecDeque::new()),
            num_requests: AtomicUsize::new(0),
            wait_queue: WaitQueue::new(),
        }
    }

    /// Returns the number of requests currently in this queue.
    pub fn num_requests(&self) -> usize {
        self.num_requests.load(Ordering::Relaxed)
    }

    fn dec_num_requests(&self) {
        self.num_requests.fetch_sub(1, Ordering::Relaxed);
    }

    fn inc_num_requests(&self) {
        self.num_requests.fetch_add(1, Ordering::Relaxed);
    }
}

impl Default for BioSingleQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl Debug for BioSingleQueue {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("BioSingleQueue")
            .field("num_requests", &self.num_requests())
            .finish()
    }
}

impl BioRequestQueue for BioSingleQueue {
    /// Enqueues a `SubmittedBio` to this queue.
    ///
    /// When enqueueing the `SubmittedBio`, try to insert it into the last request if the
    /// type is same and the sector range is contiguous.
    /// Otherwise, creates and inserts a new request for the `SubmittedBio`.
    fn enqueue(&self, bio: SubmittedBio) -> Result<(), BioEnqueueError> {
        let mut queue = self.queue.lock();
        if let Some(request) = queue.front_mut() {
            if request.can_merge(&bio) {
                request.merge_bio(bio);
                return Ok(());
            }
        }

        let new_request = BioRequest::from(bio);
        queue.push_front(new_request);
        drop(queue);
        self.inc_num_requests();
        self.wait_queue.wake_all();
        Ok(())
    }

    /// Dequeues a `BioRequest` from this queue.
    fn dequeue(&self) -> BioRequest {
        let mut num_requests = self.num_requests();

        loop {
            if num_requests > 0 {
                if let Some(request) = self.queue.lock().pop_back() {
                    self.dec_num_requests();
                    return request;
                }
            }

            num_requests = self.wait_queue.wait_until(|| {
                let num_requests = self.num_requests();
                if num_requests > 0 {
                    Some(num_requests)
                } else {
                    None
                }
            });
        }
    }
}
