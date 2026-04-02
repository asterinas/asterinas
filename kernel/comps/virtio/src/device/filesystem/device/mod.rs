// SPDX-License-Identifier: MPL-2.0

use alloc::{
    boxed::Box,
    string::{String, ToString},
    sync::Arc,
    vec,
    vec::Vec,
};
use core::{
    cmp,
    hint::spin_loop,
    mem::size_of,
    sync::atomic::{AtomicU64, Ordering},
};

use aster_util::mem_obj_slice::Slice;
use log::{debug, info, warn};
use ostd::{
    arch::trap::TrapFrame,
    mm::{VmIo, VmReader, VmWriter, io::util::HasVmReaderWriter},
    sync::{LocalIrqDisabled, SpinLock, Waiter, Waker},
    timer::{Jiffies, TIMER_FREQ},
};
use ostd_pod::Pod;
use spin::Once;

use super::{
    DEVICE_NAME,
    config::{FileSystemFeatures, VirtioFsConfig},
    pool::{FsDmaBuf, FsDmaPools},
    protocol::*,
};
use crate::{
    device::VirtioDeviceError,
    queue::{QueueError, VirtQueue},
    transport::{VirtioTransport, VirtioTransportError},
};

const HIPRIO_QUEUE_INDEX: u16 = 0;
const DEFAULT_QUEUE_SIZE: u16 = 128;
const REQUEST_WAIT_TIMEOUT_JIFFIES: u64 = 10 * TIMER_FREQ;
const O_RDWR: u32 = 2;

static FILESYSTEM_DEVICES: Once<SpinLock<Vec<Arc<FileSystemDevice>>>> = Once::new();

#[derive(Debug, Clone)]
pub struct VirtioFsDirEntry {
    pub ino: u64,
    pub offset: u64,
    pub type_: u32,
    pub name: String,
}

struct RequestWaitState {
    completed: bool,
    waker: Option<Arc<Waker>>,
}

struct FsRequest {
    _buffers: Vec<FsDmaBuf>,
    wait_state: SpinLock<RequestWaitState, LocalIrqDisabled>,
}

impl FsRequest {
    fn new(buffers: Vec<FsDmaBuf>) -> Arc<Self> {
        Arc::new(Self {
            _buffers: buffers,
            wait_state: SpinLock::new(RequestWaitState {
                completed: false,
                waker: None,
            }),
        })
    }
}

struct FsRequestQueue {
    queue: SpinLock<VirtQueue, LocalIrqDisabled>,
    in_flight_requests: SpinLock<Vec<Option<Arc<FsRequest>>>, LocalIrqDisabled>,
}

impl FsRequestQueue {
    fn new(queue: VirtQueue) -> Self {
        let queue_size = queue.available_desc();
        Self {
            queue: SpinLock::new(queue),
            in_flight_requests: SpinLock::new(vec![None; queue_size]),
        }
    }
}

impl core::fmt::Debug for FsRequestQueue {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FsRequestQueue")
            .field("queue", &self.queue)
            .field(
                "in_flight_requests_len",
                &self
                    .in_flight_requests
                    .lock()
                    .iter()
                    .filter(|request| request.is_some())
                    .count(),
            )
            .finish()
    }
}

#[derive(Debug, Clone, Copy)]
enum QueueSelector {
    Hiprio,
    Request(usize),
}

pub struct FileSystemDevice {
    transport: SpinLock<Box<dyn VirtioTransport>, LocalIrqDisabled>,
    hiprio_queue: FsRequestQueue,
    request_queues: Vec<FsRequestQueue>,
    dma_pools: Arc<FsDmaPools>,
    next_unique: AtomicU64,
    tag: String,
    notify_supported: bool,
}

impl core::fmt::Debug for FileSystemDevice {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FileSystemDevice")
            .field("transport", &self.transport)
            .field("hiprio_queue", &self.hiprio_queue)
            .field("request_queues", &self.request_queues)
            .field("tag", &self.tag)
            .field("notify_supported", &self.notify_supported)
            .finish()
    }
}

mod client;
mod helpers;
mod virtio_ops;

pub fn get_device_by_tag(tag: &str) -> Option<Arc<FileSystemDevice>> {
    let devices = FILESYSTEM_DEVICES.get()?;
    let devices = devices.disable_irq().lock();
    devices.iter().find(|device| device.tag == tag).cloned()
}

fn config_space_change(_: &TrapFrame) {
    debug!("Virtio-FS device configuration space change");
}

fn map_transport_err(_: VirtioTransportError) -> VirtioDeviceError {
    VirtioDeviceError::QueueUnknownError
}
