// SPDX-License-Identifier: MPL-2.0

//! Request queue and kernel thread for serialized devtmpfs node operations.

use ostd::sync::{Waiter, Waker};
use spin::Once;

use super::{DevtmpfsNode, tree};
use crate::{prelude::*, thread::kernel_thread::ThreadOptions};

/// Creates a device node through `devtmpfsd`.
///
/// The request is queued to the dedicated devtmpfs kernel thread and this
/// function waits until the node has been created or the creation fails.
pub(crate) fn create_node(node: DevtmpfsNode) -> Result<()> {
    submit(Request::CreateNode(node))
}

/// Deletes a device node through `devtmpfsd`.
///
/// The request is queued to the dedicated devtmpfs kernel thread and this
/// function waits until the deletion has completed or failed. The deletion only
/// unlinks nodes that were created by `devtmpfsd` and still match the requested
/// device type and device ID.
pub(crate) fn delete_node(node: DevtmpfsNode) -> Result<()> {
    submit(Request::DeleteNode(node))
}

pub(super) fn init() {
    ThreadOptions::new(devtmpfsd).spawn();
}

fn submit(request: Request) -> Result<()> {
    let (waiter, waker) = Waiter::new_pair();
    let request = Arc::new(PendingRequest::new(request, waker));

    REQUEST_QUEUE.requests.lock().push_back(request.clone());
    if let Some(waker) = REQUEST_QUEUE.waker.get() {
        waker.wake_up();
    } else {
        // `devtmpfsd` will check the requests after initializing the waker.
    }

    waiter.wait();

    // `result` is guaranteed to be available after `waiter.wait()`.
    *request.result.get().unwrap()
}

fn devtmpfsd() {
    let (waiter, waker) = Waiter::new_pair();

    REQUEST_QUEUE.waker.call_once(|| waker);

    loop {
        let request = REQUEST_QUEUE.requests.lock().pop_front();
        let Some(request) = request else {
            waiter.wait();
            continue;
        };

        let result = match &request.request {
            Request::CreateNode(node) => tree::create_node(node),
            Request::DeleteNode(node) => tree::delete_node(node),
        };
        request.result.call_once(|| result);
        request.waker.wake_up();
    }
}

struct RequestQueue {
    requests: SpinLock<VecDeque<Arc<PendingRequest>>>,
    waker: Once<Arc<Waker>>,
}

static REQUEST_QUEUE: RequestQueue = RequestQueue {
    requests: SpinLock::new(VecDeque::new()),
    waker: Once::new(),
};

struct PendingRequest {
    request: Request,
    result: Once<Result<()>>,
    waker: Arc<Waker>,
}

impl PendingRequest {
    fn new(request: Request, waker: Arc<Waker>) -> Self {
        Self {
            request,
            result: Once::new(),
            waker,
        }
    }
}

enum Request {
    CreateNode(DevtmpfsNode),
    DeleteNode(DevtmpfsNode),
}

#[cfg(ktest)]
pub(super) fn init_for_ktest() {
    static START: Once<()> = Once::new();

    crate::time::clocks::init_for_ktest();
    START.call_once(|| {
        ThreadOptions::new(devtmpfsd).spawn();
    });
}
