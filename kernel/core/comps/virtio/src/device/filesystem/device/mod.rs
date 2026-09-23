// SPDX-License-Identifier: MPL-2.0

//! Virtiofs device request handling.
//!
//! This module defines [`FileSystemDevice`], which initializes the virtiofs
//! queues, tracks in-flight requests, and sends typed FUSE operations to the
//! server.

mod queue;
mod request;
mod session;
mod virtio_ops;
mod waiter;

use alloc::{string::String, sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicU64, Ordering};

use aster_fuse::{
    FuseCompleteFn, FuseError, FuseNodeId, FuseOperation, FuseUnique, ReplyExpectation,
    ReplyHeader, ReqHeader,
};
use ostd::{
    mm::{
        dma::{FromDevice, ToDevice},
        io::util::HasVmReaderWriter,
    },
    sync::{LocalIrqDisabled, SpinLock},
};
use queue::FsRequestQueue;
use request::{FuseRequest, RequestBufs};
use spin::Once;
use waiter::{FuseWaiter, ReplyBufs};

pub use self::session::{AttrVersion, FuseSession};
use crate::{
    device::filesystem::{
        pool,
        pool::{FuseDataBuf, FuseReplyBuf, FuseReplyBufs, FuseRequestBuf, VirtiofsDmaPool},
    },
    transport::DeviceTransport,
};

/// Maximum data pages packed into one FUSE read request as DMA buffers.
///
/// A read request reserves one virtqueue descriptor for its request header and
/// one for its reply header. Batching beyond this limit must be split across
/// multiple requests.
pub const MAX_READ_DATA_PAGES_PER_REQUEST: usize = queue::MAX_DMA_BUFS_PER_REQUEST - 2;

/// Maximum data pages packed into one FUSE write request as DMA buffers.
///
/// A write request reserves one descriptor each for its request header, reply
/// header, and write reply payload. Batching beyond this limit must be split
/// across multiple requests.
pub const MAX_WRITE_DATA_PAGES_PER_REQUEST: usize = queue::MAX_DMA_BUFS_PER_REQUEST - 3;

static FILESYSTEM_DEVICES: Once<SpinLock<Vec<Arc<FileSystemDevice>>, LocalIrqDisabled>> =
    Once::new();

/// A virtiofs device that issues FUSE requests to a server.
///
/// # Locking
///
/// FUSE requests may be submitted while the caller holds sleepable VFS
/// guards. The request path takes virtio-fs locks in this order:
/// VFS sleepable guard -> selected request queue `SpinLock`.
pub struct FileSystemDevice {
    transport: SpinLock<DeviceTransport, LocalIrqDisabled>,
    hiprio_queue: Arc<FsRequestQueue>,
    request_queues: Vec<Arc<FsRequestQueue>>,
    to_device_pool: VirtiofsDmaPool<ToDevice>,
    from_device_pool: VirtiofsDmaPool<FromDevice>,
    next_unique: AtomicU64,
    tag: String,
    notify_supported: bool,
}

impl FileSystemDevice {
    fn new(
        transport: DeviceTransport,
        hiprio_queue: Arc<FsRequestQueue>,
        request_queues: Vec<Arc<FsRequestQueue>>,
        tag: String,
        notify_supported: bool,
    ) -> Self {
        let (to_device_arena_pool, from_device_arena_pool) = pool::dma_arena_pools_singleton();

        Self {
            transport: SpinLock::new(transport),
            hiprio_queue,
            request_queues,
            to_device_pool: VirtiofsDmaPool::new(to_device_arena_pool),
            from_device_pool: VirtiofsDmaPool::new(from_device_arena_pool),
            // Start request IDs at 1 and keep 0 unused. In FUSE,
            // `unique == 0` is reserved for unsolicited notification messages
            // rather than ordinary request/reply matching.
            next_unique: AtomicU64::new(1),
            tag,
            notify_supported,
        }
    }

    /// Submits a FUSE operation and returns a waiter for request completion.
    ///
    /// # Locking
    ///
    /// Builds the request before taking the selected request queue `SpinLock`.
    /// The submit path may sleep waiting for free queue descriptors, but the
    /// queue lock is never held across that wait.
    fn submit_fuse_op<Op: FuseOperation>(
        &self,
        nodeid: FuseNodeId,
        operation: &mut Op,
        data_bufs: Option<FuseDataBuf>,
        complete_fn: Option<FuseCompleteFn>,
    ) -> Result<Arc<FuseWaiter>, FuseError> {
        let request = self.prepare_request(nodeid, operation, data_bufs, complete_fn)?;
        let waiter = request.waiter().clone();

        let queue = self.select_request_queue(request.nodeid());
        self.submit(queue, request)?;

        Ok(waiter)
    }

    fn init_completion_taskless(&self) {
        self.hiprio_queue.init_completion_taskless();

        for queue in &self.request_queues {
            queue.init_completion_taskless();
        }
    }

    fn prepare_request<Op: FuseOperation>(
        &self,
        nodeid: FuseNodeId,
        operation: &mut Op,
        data_bufs: Option<FuseDataBuf>,
        complete_fn: Option<FuseCompleteFn>,
    ) -> Result<FuseRequest, FuseError> {
        let unique = self.alloc_unique();
        let reply_expectation = operation.reply_expectation();

        let data_buf_len = match data_bufs.as_ref() {
            Some(FuseDataBuf::Write(data_bufs)) => data_bufs.iter().map(FuseRequestBuf::len).sum(),
            _ => 0,
        };

        let request_buf =
            self.alloc_and_fill_request_buf(nodeid, operation, unique, data_buf_len)?;

        let (request_bufs, reply_bufs) = match data_bufs {
            Some(FuseDataBuf::Read(data_bufs)) => {
                let reply_bufs = self.alloc_reply_bufs(reply_expectation, Some(data_bufs))?;
                let mut request_bufs = RequestBufs::new();
                request_bufs.push(request_buf);
                (request_bufs, reply_bufs)
            }
            Some(FuseDataBuf::Write(data_bufs)) => {
                for data_buf in data_bufs.iter() {
                    data_buf.sync_to_device().unwrap();
                }

                let reply_bufs = self.alloc_reply_bufs(reply_expectation, None)?;
                if reply_bufs.is_empty() {
                    return Err(FuseError::MalformedResponse);
                }

                let mut request_bufs = RequestBufs::new();
                request_bufs.push(request_buf);
                request_bufs.extend(data_bufs);
                (request_bufs, reply_bufs)
            }
            None => {
                let reply_bufs = self.alloc_reply_bufs(reply_expectation, None)?;

                let mut request_bufs = RequestBufs::new();
                request_bufs.push(request_buf);
                (request_bufs, reply_bufs)
            }
        };

        Ok(FuseRequest::new(
            unique,
            nodeid,
            reply_expectation,
            request_bufs,
            reply_bufs,
            complete_fn,
        ))
    }

    fn select_request_queue(&self, nodeid: FuseNodeId) -> &FsRequestQueue {
        let request_queue_count = self.request_queues.len();
        let queue_index = if request_queue_count <= 1 {
            0
        } else {
            (nodeid.as_u64() as usize) % request_queue_count
        };

        self.request_queues[queue_index].as_ref()
    }

    fn alloc_unique(&self) -> FuseUnique {
        FuseUnique::new(self.next_unique.fetch_add(1, Ordering::Relaxed))
    }

    fn submit(
        &self,
        request_queue: &FsRequestQueue,
        request: FuseRequest,
    ) -> Result<(), FuseError> {
        request_queue.add_request(request)
    }

    fn alloc_and_fill_request_buf(
        &self,
        nodeid: FuseNodeId,
        operation: &mut impl FuseOperation,
        unique: FuseUnique,
        data_buf_len: usize,
    ) -> Result<FuseRequestBuf, FuseError> {
        let request_buf_len = (size_of::<ReqHeader>() as u32)
            .checked_add(operation.body_len() as u32)
            .ok_or(FuseError::LengthOverflow)?;
        let data_buf_len = u32::try_from(data_buf_len).map_err(|_| FuseError::LengthOverflow)?;

        let total_len = request_buf_len
            .checked_add(data_buf_len)
            .ok_or(FuseError::LengthOverflow)?;

        let request_buf = self
            .to_device_pool
            .alloc_request_buf(request_buf_len as usize)
            .map_err(FuseError::ResourceAlloc)?;

        let request_header = ReqHeader::new(total_len, operation.opcode() as u32, unique, nodeid);

        let mut writer = request_buf.writer().unwrap();
        writer.write_val(&request_header).unwrap();
        operation.write_body(&mut writer)?;

        request_buf.sync_to_device().unwrap();

        Ok(request_buf)
    }

    fn alloc_reply_bufs(
        &self,
        reply_expectation: ReplyExpectation,
        data_bufs: Option<FuseReplyBufs>,
    ) -> Result<ReplyBufs, FuseError> {
        match (reply_expectation, data_bufs) {
            (ReplyExpectation::None, None) => Ok(ReplyBufs::new(None)),
            (ReplyExpectation::HeaderOnly, None) => {
                let header_buf = self.alloc_reply_header_buf()?;
                Ok(ReplyBufs::new(Some(header_buf)))
            }
            (
                ReplyExpectation::FixedPayload(payload_size)
                | ReplyExpectation::VariablePayload(payload_size),
                None,
            ) => {
                let mut reply_bufs = ReplyBufs::new(Some(self.alloc_reply_header_buf()?));
                reply_bufs.push_payload(self.alloc_reply_payload_buf(payload_size.get())?);
                Ok(reply_bufs)
            }
            (ReplyExpectation::WritePayload { .. }, None) => {
                let mut reply_bufs = ReplyBufs::new(Some(self.alloc_reply_header_buf()?));
                reply_bufs.push_payload(
                    self.alloc_reply_payload_buf(size_of::<aster_fuse::ops::write::WriteReply>())?,
                );
                Ok(reply_bufs)
            }
            (
                ReplyExpectation::FixedPayload(payload_size)
                | ReplyExpectation::VariablePayload(payload_size),
                Some(data_bufs),
            ) => {
                if payload_size.get() > data_bufs.iter().map(FuseReplyBuf::len).sum() {
                    return Err(FuseError::BufferTooSmall);
                }

                let mut reply_bufs = ReplyBufs::new(Some(self.alloc_reply_header_buf()?));
                for data_buf in data_bufs {
                    reply_bufs.push_payload(data_buf);
                }
                Ok(reply_bufs)
            }
            (ReplyExpectation::WritePayload { .. }, Some(_)) => Err(FuseError::MalformedResponse),
            (_, Some(_)) => Err(FuseError::MalformedResponse),
        }
    }

    fn alloc_reply_payload_buf(&self, payload_size: usize) -> Result<FuseReplyBuf, FuseError> {
        self.alloc_from_device_buf(payload_size)
    }

    fn alloc_reply_header_buf(&self) -> Result<FuseReplyBuf, FuseError> {
        self.alloc_from_device_buf(size_of::<ReplyHeader>())
    }

    fn alloc_from_device_buf(&self, len: usize) -> Result<FuseReplyBuf, FuseError> {
        self.from_device_pool
            .alloc_reply_buf(len)
            .map_err(FuseError::ResourceAlloc)
    }
}

fn register_device(device: Arc<FileSystemDevice>) {
    FILESYSTEM_DEVICES
        .call_once(|| SpinLock::new(Vec::new()))
        .lock()
        .push(device);
}

/// Finds the virtio-fs device registered with the given `tag`.
pub fn find_device_by_tag(tag: &str) -> Option<Arc<FileSystemDevice>> {
    let devices = FILESYSTEM_DEVICES.get()?;
    let devices = devices.lock();
    devices
        .iter()
        .find(|device| device.tag.as_str() == tag)
        .cloned()
}

/// Virtio-fs reserves queue 0 for high-priority requests such as `FUSE_FORGET`.
const HIPRIO_QUEUE_INDEX: u16 = 0;

/// The default queue size for any queue in virtio-fs.
const DEFAULT_QUEUE_SIZE: u16 = 128;
