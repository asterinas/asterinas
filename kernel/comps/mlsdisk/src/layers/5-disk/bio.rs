// SPDX-License-Identifier: MPL-2.0

//! Block I/O (BIO).
use alloc::collections::VecDeque;
use core::{
    any::{Any, TypeId},
    ptr::NonNull,
    sync::atomic::{AtomicUsize, Ordering},
};

use hashbrown::HashMap;

use crate::{
    os::{Mutex, MutexGuard},
    prelude::*,
    Buf,
};

/// A queue for managing block I/O requests (`BioReq`).
/// It provides a concurrency-safe way to store and manage
/// block I/O requests that need to be processed by a block device.
pub struct BioReqQueue {
    queue: Mutex<VecDeque<BioReq>>,
    num_reqs: AtomicUsize,
}

impl BioReqQueue {
    /// Create a new `BioReqQueue` instance.
    pub fn new() -> Self {
        Self {
            queue: Mutex::new(VecDeque::new()),
            num_reqs: AtomicUsize::new(0),
        }
    }

    /// Enqueue a block I/O request.
    pub fn enqueue(&self, req: BioReq) -> Result<()> {
        req.submit();
        self.queue.lock().push_back(req);
        self.num_reqs.fetch_add(1, Ordering::Release);
        Ok(())
    }

    /// Dequeue a block I/O request.
    pub fn dequeue(&self) -> Option<BioReq> {
        if let Some(req) = self.queue.lock().pop_front() {
            self.num_reqs.fetch_sub(1, Ordering::Release);
            Some(req)
        } else {
            debug_assert_eq!(self.num_reqs.load(Ordering::Acquire), 0);
            None
        }
    }

    /// Returns the number of pending requests in this queue.
    pub fn num_reqs(&self) -> usize {
        self.num_reqs.load(Ordering::Acquire)
    }

    /// Returns whether there are no pending requests in this queue.
    pub fn is_empty(&self) -> bool {
        self.num_reqs() == 0
    }
}

/// A block I/O request.
pub struct BioReq {
    type_: BioType,
    addr: BlockId,
    nblocks: u32,
    bufs: Mutex<Vec<Buf>>,
    status: Mutex<BioStatus>,
    on_complete: Option<BioReqOnCompleteFn>,
    ext: Mutex<HashMap<TypeId, Box<dyn Any + Send + Sync>>>,
}

/// The type of a block request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BioType {
    /// A read request.
    Read,
    /// A write request.
    Write,
    /// A sync request.
    Sync,
}

/// A response from a block device.
pub type BioResp = Result<()>;

/// The type of the callback function invoked upon the completion of
/// a block I/O request.
pub type BioReqOnCompleteFn = fn(/* req = */ &BioReq, /* resp = */ &BioResp);

/// The status describing a block I/O request.
#[derive(Clone, Debug)]
enum BioStatus {
    Init,
    Submitted,
    Completed(BioResp),
}

impl BioReq {
    /// Returns the type of the request.
    pub fn type_(&self) -> BioType {
        self.type_
    }

    /// Returns the starting address of requested blocks.
    ///
    /// The return value is meaningless if the request is not a read or write.
    pub fn addr(&self) -> BlockId {
        self.addr
    }

    /// Access the immutable buffers with a closure.
    pub fn access_bufs_with<F, R>(&self, mut f: F) -> R
    where
        F: FnMut(&[Buf]) -> R,
    {
        let bufs = self.bufs.lock();
        (f)(&bufs)
    }

    /// Access the mutable buffers with a closure.
    pub(super) fn access_mut_bufs_with<F, R>(&self, mut f: F) -> R
    where
        F: FnMut(&mut [Buf]) -> R,
    {
        let mut bufs = self.bufs.lock();
        (f)(&mut bufs)
    }

    /// Take the buffers out of the request.
    pub(super) fn take_bufs(&self) -> Vec<Buf> {
        let mut bufs = self.bufs.lock();
        let mut ret_bufs = Vec::new();
        core::mem::swap(&mut *bufs, &mut ret_bufs);
        ret_bufs
    }

    /// Returns the number of buffers associated with the request.
    ///
    /// If the request is a flush, then the returned value is meaningless.
    pub fn nbufs(&self) -> usize {
        self.bufs.lock().len()
    }

    /// Returns the number of blocks to read or write by this request.
    ///
    /// If the request is a flush, then the returned value is meaningless.
    pub fn nblocks(&self) -> usize {
        self.nblocks as usize
    }

    /// Returns the extensions of the request.
    ///
    /// The extensions of a request is a set of objects that may be added, removed,
    /// or accessed by block devices and their users. Each of the extension objects
    /// must have a different type. To avoid conflicts, it is recommended to use only
    /// private types for the extension objects.
    pub fn ext(&self) -> MutexGuard<HashMap<TypeId, Box<dyn Any + Send + Sync>>> {
        self.ext.lock()
    }

    /// Update the status of the request to "completed" by giving the response
    /// to the request.
    ///
    /// After the invoking this API, the request is considered completed, which
    /// means the request must have taken effect. For example, a completed read
    /// request must have all its buffers filled with data.
    ///
    /// # Panics
    ///
    /// If the request has not been submitted yet, or has been completed already,
    /// this method will panic.
    pub(super) fn complete(&self, resp: BioResp) {
        let mut status = self.status.lock();
        match *status {
            BioStatus::Submitted => {
                if let Some(on_complete) = self.on_complete {
                    (on_complete)(self, &resp);
                }

                *status = BioStatus::Completed(resp);
            }
            _ => panic!("cannot complete before submitting or complete twice"),
        }
    }

    /// Mark the request as submitted.
    pub(super) fn submit(&self) {
        let mut status = self.status.lock();
        match *status {
            BioStatus::Init => *status = BioStatus::Submitted,
            _ => unreachable!(),
        }
    }
}

/// A builder for `BioReq`.
pub struct BioReqBuilder {
    type_: BioType,
    addr: Option<BlockId>,
    bufs: Option<Vec<Buf>>,
    on_complete: Option<BioReqOnCompleteFn>,
    ext: Option<HashMap<TypeId, Box<dyn Any + Send + Sync>>>,
}

impl BioReqBuilder {
    /// Creates a builder of a block request of the given type.
    pub fn new(type_: BioType) -> Self {
        Self {
            type_,
            addr: None,
            bufs: None,
            on_complete: None,
            ext: None,
        }
    }

    /// Specify the block address of the request.
    pub fn addr(mut self, addr: BlockId) -> Self {
        self.addr = Some(addr);
        self
    }

    /// Give the buffers of the request.
    pub fn bufs(mut self, bufs: Vec<Buf>) -> Self {
        self.bufs = Some(bufs);
        self
    }

    /// Specify a callback invoked when the request is complete.
    pub fn on_complete(mut self, on_complete: BioReqOnCompleteFn) -> Self {
        self.on_complete = Some(on_complete);
        self
    }

    /// Add an extension object to the request.
    pub fn ext<T: Any + Send + Sync + Sized>(mut self, obj: T) -> Self {
        if self.ext.is_none() {
            self.ext = Some(HashMap::new());
        }
        let _ = self
            .ext
            .as_mut()
            .unwrap()
            .insert(TypeId::of::<T>(), Box::new(obj));
        self
    }

    /// Build the request.
    pub fn build(mut self) -> BioReq {
        let type_ = self.type_;
        if type_ == BioType::Sync {
            debug_assert!(
                self.addr.is_none(),
                "addr is only meaningful for a read or write",
            );
            debug_assert!(
                self.bufs.is_none(),
                "bufs is only meaningful for a read or write",
            );
        }

        let addr = self.addr.unwrap_or(0 as BlockId);

        let bufs = self.bufs.take().unwrap_or_default();
        let nblocks = {
            let nbytes = bufs
                .iter()
                .map(|buf| buf.as_slice().len())
                .fold(0_usize, |sum, len| sum.saturating_add(len));
            (nbytes / BLOCK_SIZE) as u32
        };

        let ext = self.ext.take().unwrap_or_default();
        let on_complete = self.on_complete.take();

        BioReq {
            type_,
            addr,
            nblocks,
            bufs: Mutex::new(bufs),
            status: Mutex::new(BioStatus::Init),
            on_complete,
            ext: Mutex::new(ext),
        }
    }
}
