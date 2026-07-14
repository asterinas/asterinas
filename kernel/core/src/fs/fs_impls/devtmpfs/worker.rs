// SPDX-License-Identifier: MPL-2.0

//! Request queue and kernel thread for serialized devtmpfs node operations.

use ostd::sync::WaitQueue;
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
    let request = Arc::new(PendingRequest::new(request));
    let queue = request_queue();
    queue.requests.lock().push_back(request.clone());
    queue.wait_queue.wake_one();

    request
        .wait_queue
        .wait_until(|| request.result.lock().take())
}

fn devtmpfsd() {
    let queue = request_queue();

    loop {
        let request = queue
            .wait_queue
            .wait_until(|| queue.requests.lock().pop_front());
        let result = match &request.request {
            Request::CreateNode(node) => tree::create_node(node),
            Request::DeleteNode(node) => tree::delete_node(node),
        };

        *request.result.lock() = Some(result);
        request.wait_queue.wake_all();
    }
}

fn request_queue() -> &'static RequestQueue {
    static REQUEST_QUEUE: Once<RequestQueue> = Once::new();

    REQUEST_QUEUE.call_once(|| RequestQueue {
        requests: SpinLock::new(VecDeque::new()),
        wait_queue: WaitQueue::new(),
    })
}

struct RequestQueue {
    requests: SpinLock<VecDeque<Arc<PendingRequest>>>,
    wait_queue: WaitQueue,
}

struct PendingRequest {
    request: Request,
    result: Mutex<Option<Result<()>>>,
    wait_queue: WaitQueue,
}

impl PendingRequest {
    fn new(request: Request) -> Self {
        Self {
            request,
            result: Mutex::new(None),
            wait_queue: WaitQueue::new(),
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
