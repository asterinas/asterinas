// SPDX-License-Identifier: MPL-2.0

//! Virtio-fs request queue management.
//!
//! This module defines [`FsRequestQueue`], which submits [`FuseRequest`] values
//! to a virtqueue, tracks in-flight requests, and completes replies outside the
//! IRQ-disabled queue lock.

use alloc::{sync::Arc, vec::Vec};
use core::mem;

use aster_softirq::Taskless;
use aster_util::{mem_obj_slice::Slice, slot_vec::SlotVec};
use ostd::{
    mm::dma::{FromDevice, ToDevice},
    sync::{LocalIrqDisabled, SpinLock},
};
use smallvec::SmallVec;
use spin::Once;

use super::request::FuseRequest;
use crate::{device::filesystem::pool::FsDmaStorage, queue::VirtQueue};

/// A virtio-fs request queue and its in-flight request state.
pub(super) struct FsRequestQueue {
    inner: SpinLock<FsRequestQueueInner, LocalIrqDisabled>,
    taskless: Once<Arc<Taskless>>,
}

struct FsRequestQueueInner {
    queue: VirtQueue,
    in_flight_requests: SlotVec<FuseRequest>,
    pending_completions: Vec<CompletedFuseRequest>,
}

struct CompletedFuseRequest {
    request: FuseRequest,
    reply_len: usize,
}

impl FsRequestQueue {
    /// Creates a request queue backed by a virtqueue.
    pub(super) fn new(queue: VirtQueue) -> Arc<Self> {
        Arc::new(Self {
            inner: SpinLock::new(FsRequestQueueInner {
                queue,
                in_flight_requests: SlotVec::new(),
                pending_completions: Vec::new(),
            }),
            taskless: Once::new(),
        })
    }

    /// Initializes the deferred completion handler for this queue.
    pub(super) fn init_completion_taskless(self: &Arc<Self>) {
        let queue_weak = Arc::downgrade(self);

        self.taskless.call_once(|| {
            Taskless::new(move || {
                let Some(queue) = queue_weak.upgrade() else {
                    return;
                };

                let completions = {
                    let mut inner = queue.inner.lock();
                    mem::take(&mut inner.pending_completions)
                };
                for completion in completions {
                    completion.request.finish_reply(completion.reply_len);
                }
            })
        });
    }

    /// Submits a FUSE request to the virtqueue.
    pub(super) fn add_request(&self, request: FuseRequest) {
        let mut inner = self.inner.lock();
        let token = {
            let request_bufs = request
                .request_bufs()
                .iter()
                .map(|buf| buf.as_dma_slice())
                .collect::<SmallVec<[&Slice<FsDmaStorage<ToDevice>>; 2]>>();
            let reply_bufs = request
                .waiter()
                .reply_bufs()
                .iter()
                .map(|buf| buf.as_dma_slice())
                .collect::<SmallVec<[&Slice<FsDmaStorage<FromDevice>>; 2]>>();

            inner
                .queue
                .add_dma_bufs(request_bufs.as_slice(), reply_bufs.as_slice())
                .unwrap()
        };

        inner.in_flight_requests.put_at(token as usize, request);

        if inner.queue.should_notify() {
            inner.queue.notify();
        }
    }

    /// Moves completed virtqueue descriptors into the pending-completion list.
    pub(super) fn drain_completed_requests(&self) {
        let mut inner = self.inner.lock();
        while let Ok((token, len)) = inner.queue.pop_used() {
            let reply_len = len as usize;
            let request = inner.in_flight_requests.remove(token as usize).unwrap();
            inner
                .pending_completions
                .push(CompletedFuseRequest { request, reply_len });
        }
    }

    /// Schedules deferred completion processing for this queue.
    pub(super) fn schedule_completion_taskless(&self) {
        self.taskless.get().unwrap().schedule();
    }
}
