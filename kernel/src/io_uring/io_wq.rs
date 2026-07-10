// SPDX-License-Identifier: MPL-2.0

use core::time::Duration;

use ostd::sync::WaitQueue;

use super::{io_context::IoUringContext, ops::IoUringOp, thread::IoUringThreadOptions};
use crate::prelude::*;

const IO_WORKER_IDLE_INTERVAL: Duration = Duration::from_millis(1);

/// Queues prepared requests for asynchronous execution.
pub(super) struct IoWq {
    ring_context: Weak<IoUringContext>,
    request_queue: Mutex<VecDeque<Arc<dyn IoUringOp>>>,
    wait_queue: WaitQueue,
}

enum WorkerWaitResult {
    Request(Arc<dyn IoUringOp>),
    IdleTimedOut,
    Stop,
}

impl IoWq {
    pub(super) fn new(ring_context: Weak<IoUringContext>) -> Self {
        Self {
            ring_context,
            request_queue: Mutex::new(VecDeque::new()),
            wait_queue: WaitQueue::new(),
        }
    }

    pub(super) fn start_thread(self: &Arc<Self>, ctx: &Context) {
        let worker_thread_local =
            IoUringThreadOptions::clone_thread_local(ctx.thread_local).unwrap();
        let io_wq = self.clone();
        IoUringThreadOptions::new(move || io_worker_loop(io_wq), worker_thread_local).spawn();
    }

    pub(super) fn enqueue(&self, request: Arc<dyn IoUringOp>) {
        self.request_queue.lock().push_back(request);
        self.wait_queue.wake_one();
    }

    fn wait_for_request(&self) -> WorkerWaitResult {
        let result = self.wait_queue.wait_until_or_timeout(
            || {
                if self.ring_context.strong_count() == 0 {
                    Some(WorkerWaitResult::Stop)
                } else {
                    self.request_queue
                        .lock()
                        .pop_front()
                        .map(WorkerWaitResult::Request)
                }
            },
            &IO_WORKER_IDLE_INTERVAL,
        );

        result.unwrap_or(WorkerWaitResult::IdleTimedOut)
    }
}

fn io_worker_loop(io_wq: Arc<IoWq>) {
    loop {
        let request = match io_wq.wait_for_request() {
            WorkerWaitResult::Request(request) => request,
            WorkerWaitResult::IdleTimedOut => continue,
            WorkerWaitResult::Stop => break,
        };

        let completion = request.execute();

        let Some(context) = io_wq.ring_context.upgrade() else {
            warn!("failed to complete io_uring request: ring context is gone");
            break;
        };

        if let Err(err) = context.post_completion(completion) {
            warn!("failed to complete io_uring request: {:?}", err);
        }
    }
}
