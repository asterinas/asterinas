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

use alloc::{boxed::Box, string::String, sync::Arc, vec::Vec};
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
use request::FuseRequest;
use smallvec::smallvec;
use spin::Once;
use waiter::{FuseWaiter, ReplyBufs};

pub use self::session::{AttrVersion, FuseSession};
use crate::{
    device::filesystem::pool::{FuseDataBuf, FuseReplyBuf, FuseRequestBuf, SizeClassedDmaPool},
    transport::VirtioTransport,
};

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
    transport: SpinLock<Box<dyn VirtioTransport>, LocalIrqDisabled>,
    hiprio_queue: Arc<FsRequestQueue>,
    request_queues: Vec<Arc<FsRequestQueue>>,
    to_device_pool: Arc<SizeClassedDmaPool<ToDevice>>,
    from_device_pool: Arc<SizeClassedDmaPool<FromDevice>>,
    next_unique: AtomicU64,
    tag: String,
    notify_supported: bool,
}

impl FileSystemDevice {
    fn new(
        transport: Box<dyn VirtioTransport>,
        hiprio_queue: Arc<FsRequestQueue>,
        request_queues: Vec<Arc<FsRequestQueue>>,
        tag: String,
        notify_supported: bool,
    ) -> Self {
        Self {
            transport: SpinLock::new(transport),
            hiprio_queue,
            request_queues,
            to_device_pool: SizeClassedDmaPool::new(),
            from_device_pool: SizeClassedDmaPool::new(),
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
        data_buf: Option<FuseDataBuf>,
        complete_fn: Option<FuseCompleteFn>,
    ) -> Result<Arc<FuseWaiter>, FuseError> {
        let request = self.prepare_request(nodeid, operation, data_buf, complete_fn)?;
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
        data_buf: Option<FuseDataBuf>,
        complete_fn: Option<FuseCompleteFn>,
    ) -> Result<FuseRequest, FuseError> {
        let unique = self.alloc_unique();

        let data_buf_len = match data_buf.as_ref() {
            Some(FuseDataBuf::Write(data_buf)) => data_buf.len(),
            _ => 0,
        };

        let request_buf =
            self.alloc_and_fill_request_buf(nodeid, operation, unique, data_buf_len)?;

        let (request_bufs, reply_bufs) = match data_buf {
            Some(FuseDataBuf::Read(data_buf)) => (
                smallvec![request_buf],
                self.alloc_reply_bufs(operation.reply_expectation(), Some(data_buf))?,
            ),
            Some(FuseDataBuf::Write(data_buf)) => {
                data_buf.sync_to_device().unwrap();

                let reply_bufs = self.alloc_reply_bufs(operation.reply_expectation(), None)?;
                if reply_bufs.header().is_none() {
                    return Err(FuseError::MalformedResponse);
                }

                (smallvec![request_buf, data_buf], reply_bufs)
            }
            None => {
                let reply_bufs = self.alloc_reply_bufs(operation.reply_expectation(), None)?;

                (smallvec![request_buf], reply_bufs)
            }
        };

        Ok(FuseRequest::new(
            unique,
            nodeid,
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
        data_buf: Option<FuseReplyBuf>,
    ) -> Result<ReplyBufs, FuseError> {
        match (reply_expectation, data_buf) {
            (ReplyExpectation::None, None) => Ok(ReplyBufs::new_none()),
            (ReplyExpectation::HeaderOnly, None) => {
                Ok(ReplyBufs::new_header_only(self.alloc_reply_header_buf()?))
            }
            (ReplyExpectation::Payload(payload_size), None) => Ok(ReplyBufs::new_with_payload(
                self.alloc_reply_header_buf()?,
                self.alloc_reply_payload_buf(payload_size.get())?,
            )),
            (ReplyExpectation::Payload(payload_size), Some(data_buf)) => {
                if payload_size.get() > data_buf.len() {
                    return Err(FuseError::BufferTooSmall);
                }

                Ok(ReplyBufs::new_with_payload(
                    self.alloc_reply_header_buf()?,
                    data_buf,
                ))
            }
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
