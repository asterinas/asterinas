// SPDX-License-Identifier: MPL-2.0

//! Linux-compatible `/dev/vhost-vsock` ABI surface.
//!
//! This module registers the vhost-vsock misc device
//! and bridges vhost virtqueues to Asterinas' AF_VSOCK transport.

use core::{
    hint::spin_loop,
    mem,
    sync::atomic::{AtomicBool, AtomicU16, Ordering},
};

use aster_virtio::device::socket::header::{VirtioVsockHdr as TransportVsockHdr, VirtioVsockOp};
use device_id::{DeviceId, MinorId};
use ostd::{mm::VmIo, task::Task};
use spin::Once;

use crate::{
    device::{Device, DeviceType, DevtmpfsInodeMeta, registry::char},
    events::{IoEvents, Observer},
    fs::{
        file::{
            FileLike, PerOpenFileOps, StatusFlags,
            file_table::{FileDesc, RawFileDesc, get_file_fast},
        },
        vfs::inode::FileOps,
    },
    prelude::*,
    process::signal::{PollAdaptor, PollHandle, Pollable, Pollee, Poller},
    syscall,
    thread::{Thread, kernel_thread::ThreadOptions},
    util::ioctl::{RawIoctl, dispatch_ioctl},
    vm::vmar::{VMAR_CAP_ADDR, VMAR_LOWEST_ADDR, Vmar},
};

const VHOST_VSOCK_MINOR: u32 = 241;
const VIRTIO_F_VERSION_1: u64 = 1 << 32;
const VIRTIO_RING_F_INDIRECT_DESC: u64 = 1 << 28;
const HOST_CID: u64 = 2;
const VIRTQ_DESC_F_NEXT: u16 = 1;
const VIRTQ_DESC_F_WRITE: u16 = 2;
const VIRTQ_DESC_F_INDIRECT: u16 = 4;
const VHOST_SUPPORTED_FEATURES: u64 = VIRTIO_RING_F_INDIRECT_DESC | VIRTIO_F_VERSION_1;
const VHOST_SUPPORTED_BACKEND_FEATURES: u64 = 0;
const VHOST_VSOCK_PAGE_SIZE: u64 = 4096;
const VHOST_VSOCK_MAX_TX_CHAIN_BYTES: usize = 1024 * 1024;
const VHOST_VSOCK_MAX_QUEUE_BYTES: usize = 256 * 1024;
const VHOST_VSOCK_MAX_VRING_NUM: u32 = 32768;
const VHOST_VSOCK_MAX_MEMORY_REGIONS: usize = 64;
const VIRTQ_AVAIL_RING_OFFSET: usize = size_of::<VirtqAvailHeader>();
const VIRTQ_USED_RING_OFFSET: usize = size_of::<VirtqUsedHeader>();
const RX_VRING_INDEX: usize = 0;
const TX_VRING_INDEX: usize = 1;

type VhostVsockBackendRegistry = Arc<SpinLock<BTreeMap<u64, Arc<VhostVsockBackend>>>>;

/// Active QEMU-owned vhost-vsock backends keyed by guest CID.
///
/// The AF_VSOCK transport uses this registry to route host-initiated packets
/// to the matching guest virtqueue backend.
static BACKEND_REGISTRY: Once<VhostVsockBackendRegistry> = Once::new();

#[derive(Debug)]
struct VhostVsockDevice {
    id: DeviceId,
}

impl VhostVsockDevice {
    fn new() -> Arc<Self> {
        let major = super::MISC_MAJOR.get().unwrap().get();
        let minor = MinorId::new(VHOST_VSOCK_MINOR);

        Arc::new(Self {
            id: DeviceId::new(major, minor),
        })
    }
}

impl Device for VhostVsockDevice {
    fn type_(&self) -> DeviceType {
        DeviceType::Char
    }

    fn id(&self) -> DeviceId {
        self.id
    }

    fn devtmpfs_meta(&self) -> Option<DevtmpfsInodeMeta<'_>> {
        Some(DevtmpfsInodeMeta::new("vhost-vsock"))
    }

    fn open(&self) -> Result<Box<dyn PerOpenFileOps>> {
        Ok(Box::new(VhostVsockFile::new()))
    }
}

#[derive(Default)]
struct VhostVsockState {
    owner_set: bool,
    /// Address space that may configure the vhost device after `SET_OWNER`.
    owner_vmar: Option<Arc<Vmar>>,
    features: u64,
    backend_features: u64,
    guest_cid: Option<u64>,
    memory_regions: Vec<VhostMemoryRegion>,
    vring_num: [u32; VHOST_VSOCK_VRING_COUNT],
    vring_base: [u32; VHOST_VSOCK_VRING_COUNT],
    vring_addr: [Option<VhostVringAddr>; VHOST_VSOCK_VRING_COUNT],
    vring_kick: [Option<Arc<dyn FileLike>>; VHOST_VSOCK_VRING_COUNT],
    vring_call: [Option<Arc<dyn FileLike>>; VHOST_VSOCK_VRING_COUNT],
    vring_err: [Option<Arc<dyn FileLike>>; VHOST_VSOCK_VRING_COUNT],
    worker: VhostWorkerState,
}

#[derive(Default)]
enum VhostWorkerState {
    #[default]
    Stopped,
    Running {
        stop: Arc<AtomicBool>,
        thread: Arc<Thread>,
        backend: Arc<VhostVsockBackend>,
    },
    Stopping {
        thread: Arc<Thread>,
        backend: Arc<VhostVsockBackend>,
    },
}

impl VhostWorkerState {
    fn is_busy(&self) -> bool {
        !matches!(self, Self::Stopped)
    }
}

impl VhostVsockState {
    fn snapshot_for_worker(
        &self,
        stop: Arc<AtomicBool>,
        rx_queue: Arc<VhostRxQueue>,
    ) -> Result<WorkerInputs> {
        let owner_vmar = self
            .owner_vmar
            .clone()
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "vhost-vsock owner VMAR not set"))?;
        let guest_cid = self
            .guest_cid
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "vhost-vsock guest CID not set"))?;
        let rx_addr = self.vring_addr[RX_VRING_INDEX].ok_or_else(|| {
            Error::with_message(Errno::EINVAL, "vhost-vsock RX vring addr not set")
        })?;
        let tx_addr = self.vring_addr[TX_VRING_INDEX].ok_or_else(|| {
            Error::with_message(Errno::EINVAL, "vhost-vsock TX vring addr not set")
        })?;

        Ok(WorkerInputs {
            owner_vmar,
            guest_cid,
            mem_regions: self.memory_regions.clone(),
            vring_num: self.vring_num,
            vring_base: self.vring_base,
            vring_addr: [rx_addr, tx_addr],
            vring_kick: self.vring_kick.clone(),
            vring_call: self.vring_call.clone(),
            rx_queue,
            stop,
        })
    }

    fn validate_ready_to_run(&self) -> Result<()> {
        self.ensure_owner_set()?;
        if self.guest_cid.is_none() {
            return_errno_with_message!(Errno::EINVAL, "vhost-vsock guest CID not set");
        }
        if self.memory_regions.is_empty() {
            return_errno_with_message!(Errno::EINVAL, "vhost-vsock memory table not set");
        }
        for index in 0..VHOST_VSOCK_VRING_COUNT {
            if self.vring_num[index] == 0 {
                return_errno_with_message!(Errno::EINVAL, "vhost-vsock vring num not set");
            }
            if self.vring_addr[index].is_none() {
                return_errno_with_message!(Errno::EINVAL, "vhost-vsock vring addr not set");
            }
        }

        Ok(())
    }

    fn ensure_owner_set(&self) -> Result<()> {
        let Some(owner_vmar) = self.owner_vmar.as_ref() else {
            return_errno_with_message!(Errno::EPERM, "vhost-vsock owner is not set");
        };
        if !self.owner_set {
            return_errno_with_message!(Errno::EPERM, "vhost-vsock owner is not set");
        }
        let current_vmar = VhostVsockFile::current_vmar()?;
        if !Arc::ptr_eq(owner_vmar, &current_vmar) {
            return_errno_with_message!(Errno::EPERM, "vhost-vsock caller is not the owner");
        }

        Ok(())
    }

    fn ensure_configurable(&self) -> Result<()> {
        self.ensure_owner_set()?;
        if self.worker.is_busy() {
            return_errno_with_message!(Errno::EBUSY, "vhost-vsock is running");
        }

        Ok(())
    }

    fn stop_worker(&mut self) -> Result<Option<StoppedWorker>> {
        let worker = mem::replace(&mut self.worker, VhostWorkerState::Stopped);
        let (stop, thread, backend) = match worker {
            VhostWorkerState::Stopped => return Ok(None),
            VhostWorkerState::Running {
                stop,
                thread,
                backend,
            } => (stop, thread, backend),
            VhostWorkerState::Stopping { thread, backend } => {
                self.worker = VhostWorkerState::Stopping { thread, backend };
                return_errno_with_message!(Errno::EBUSY, "vhost-vsock worker is stopping");
            }
        };

        self.worker = VhostWorkerState::Stopping {
            thread: thread.clone(),
            backend: backend.clone(),
        };
        {
            stop.store(true, Ordering::Release);
        }
        backend.inputs.rx_queue.close();
        backend.inputs.rx_queue.notify_worker();
        Ok(Some(StoppedWorker { thread, backend }))
    }

    fn begin_reset_owner(&mut self) -> Result<Option<StoppedWorker>> {
        self.ensure_owner_set()?;
        self.stop_worker()
    }
}

struct StoppedWorker {
    thread: Arc<Thread>,
    backend: Arc<VhostVsockBackend>,
}

#[derive(Clone)]
struct WorkerInputs {
    owner_vmar: Arc<Vmar>,
    guest_cid: u64,
    mem_regions: Vec<VhostMemoryRegion>,
    vring_num: [u32; VHOST_VSOCK_VRING_COUNT],
    vring_base: [u32; VHOST_VSOCK_VRING_COUNT],
    vring_addr: [VhostVringAddr; VHOST_VSOCK_VRING_COUNT],
    vring_kick: [Option<Arc<dyn FileLike>>; VHOST_VSOCK_VRING_COUNT],
    vring_call: [Option<Arc<dyn FileLike>>; VHOST_VSOCK_VRING_COUNT],
    rx_queue: Arc<VhostRxQueue>,
    stop: Arc<AtomicBool>,
}

struct VhostVsockBackend {
    inputs: WorkerInputs,
    /// Keeps kick eventfd observers registered for the backend lifetime.
    _kick_pollers: Vec<PollAdaptor<VhostKickObserver>>,
    rx_last_avail: AtomicU16,
    rx_inject_busy: AtomicBool,
    tx_last_avail: AtomicU16,
}

impl VhostVsockBackend {
    fn new(inputs: WorkerInputs) -> Self {
        let kick_pollers = register_kick_pollers(&inputs);
        let rx_base = inputs.vring_base[RX_VRING_INDEX] as u16;
        let tx_base = inputs.vring_base[TX_VRING_INDEX] as u16;
        Self {
            inputs,
            _kick_pollers: kick_pollers,
            rx_last_avail: AtomicU16::new(rx_base),
            rx_inject_busy: AtomicBool::new(false),
            tx_last_avail: AtomicU16::new(tx_base),
        }
    }

    fn inject(&self, packet: VhostVsockPacket<'_>) -> Result<bool> {
        self.inputs.rx_queue.push(packet.into())
    }

    fn drain_rx_queue(&self) -> RxDrainResult {
        while let Some(packet) = self.inputs.rx_queue.pop_front() {
            match self.inject_now(packet.as_packet()) {
                Ok(()) => self.inputs.rx_queue.commit_popped(&packet),
                Err(err) if err.error() == Errno::EAGAIN => {
                    self.inputs.rx_queue.push_front(packet);
                    return RxDrainResult::Blocked;
                }
                Err(_) => {
                    self.inputs.rx_queue.commit_popped(&packet);
                    return RxDrainResult::Stopped;
                }
            }
        }

        RxDrainResult::Drained
    }

    fn inject_now(&self, packet: VhostVsockPacket<'_>) -> Result<()> {
        while self
            .rx_inject_busy
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            spin_loop();
        }
        let mut rx_last_avail = self.rx_last_avail.load(Ordering::Relaxed);
        let result = inject_packet(&self.inputs, &mut rx_last_avail, packet);
        if result.is_ok() {
            self.rx_last_avail.store(rx_last_avail, Ordering::Relaxed);
        }
        self.rx_inject_busy.store(false, Ordering::Release);
        result
    }

    fn process_tx(&self) -> Result<()> {
        let mut tx_last_avail = self.tx_last_avail.load(Ordering::Relaxed);
        let result = process_tx(self, &mut tx_last_avail);
        self.tx_last_avail.store(tx_last_avail, Ordering::Relaxed);
        result
    }
}

struct VhostKickObserver {
    rx_queue: Arc<VhostRxQueue>,
}

impl Observer<IoEvents> for VhostKickObserver {
    fn on_events(&self, events: &IoEvents) {
        if !events.is_empty() {
            self.rx_queue.notify_worker();
        }
    }
}

fn register_kick_pollers(inputs: &WorkerInputs) -> Vec<PollAdaptor<VhostKickObserver>> {
    let mut pollers = Vec::with_capacity(VHOST_VSOCK_VRING_COUNT);
    for kick in inputs.vring_kick.iter().flatten() {
        let mut poller = PollAdaptor::with_observer(VhostKickObserver {
            rx_queue: inputs.rx_queue.clone(),
        });
        if !kick
            .poll(IoEvents::IN, Some(poller.as_handle_mut()))
            .is_empty()
        {
            inputs.rx_queue.notify_worker();
        }
        pollers.push(poller);
    }

    pollers
}

enum RxDrainResult {
    Blocked,
    Drained,
    Stopped,
}

struct VhostVsockPacket<'a> {
    dst_port: u32,
    src_port: u32,
    op: u16,
    flags: u32,
    payload: &'a [u8],
    buf_alloc: u32,
    fwd_cnt: u32,
}

#[derive(Clone)]
struct QueuedVhostVsockPacket {
    dst_port: u32,
    src_port: u32,
    op: u16,
    flags: u32,
    payload: Vec<u8>,
    buf_alloc: u32,
    fwd_cnt: u32,
}

impl<'a> From<VhostVsockPacket<'a>> for QueuedVhostVsockPacket {
    fn from(packet: VhostVsockPacket<'a>) -> Self {
        Self {
            dst_port: packet.dst_port,
            src_port: packet.src_port,
            op: packet.op,
            flags: packet.flags,
            payload: packet.payload.to_vec(),
            buf_alloc: packet.buf_alloc,
            fwd_cnt: packet.fwd_cnt,
        }
    }
}

impl QueuedVhostVsockPacket {
    fn as_packet(&self) -> VhostVsockPacket<'_> {
        VhostVsockPacket {
            dst_port: self.dst_port,
            src_port: self.src_port,
            op: self.op,
            flags: self.flags,
            payload: self.payload.as_slice(),
            buf_alloc: self.buf_alloc,
            fwd_cnt: self.fwd_cnt,
        }
    }

    fn total_len(&self) -> usize {
        VIRTIO_VSOCK_HDR_SIZE.saturating_add(self.payload.len())
    }
}

struct VhostRxQueue {
    inner: SpinLock<VhostRxQueueInner>,
    wake_pending: AtomicBool,
    pollee: Pollee,
}

struct VhostRxQueueInner {
    packets: VecDeque<QueuedVhostVsockPacket>,
    bytes: usize,
    closed: bool,
}

impl VhostRxQueue {
    fn new() -> Self {
        Self {
            inner: SpinLock::new(VhostRxQueueInner {
                packets: VecDeque::new(),
                bytes: 0,
                closed: false,
            }),
            wake_pending: AtomicBool::new(false),
            pollee: Pollee::new(),
        }
    }

    fn push(&self, packet: QueuedVhostVsockPacket) -> Result<bool> {
        let packet_len = packet.total_len();
        let mut inner = self.inner.lock();
        if inner.closed {
            return Ok(false);
        }
        let next_bytes = inner.bytes.checked_add(packet_len).ok_or_else(|| {
            Error::with_message(Errno::EAGAIN, "vhost-vsock RX queue byte count overflow")
        })?;
        if next_bytes > VHOST_VSOCK_MAX_QUEUE_BYTES {
            return_errno_with_message!(Errno::EAGAIN, "vhost-vsock RX queue is full");
        }
        inner.bytes = next_bytes;
        inner.packets.push_back(packet);
        drop(inner);

        self.notify_worker();
        Ok(true)
    }

    fn pop_front(&self) -> Option<QueuedVhostVsockPacket> {
        self.inner.lock().packets.pop_front()
    }

    fn commit_popped(&self, packet: &QueuedVhostVsockPacket) {
        let mut inner = self.inner.lock();
        inner.bytes = inner.bytes.saturating_sub(packet.total_len());
    }

    fn push_front(&self, packet: QueuedVhostVsockPacket) {
        self.inner.lock().packets.push_front(packet);
    }

    fn has_packets(&self) -> bool {
        !self.inner.lock().packets.is_empty()
    }

    fn poll_worker_wake(&self, poller: Option<&mut PollHandle>) -> IoEvents {
        self.pollee.poll_with(IoEvents::IN, poller, || {
            if self.wake_pending.load(Ordering::Acquire) {
                IoEvents::IN
            } else {
                IoEvents::empty()
            }
        })
    }

    fn take_worker_wake(&self) -> bool {
        let was_pending = self.wake_pending.swap(false, Ordering::Acquire);
        if was_pending {
            self.pollee.invalidate();
        }

        was_pending
    }

    fn notify_worker(&self) {
        self.wake_pending.store(true, Ordering::Release);
        self.pollee.notify(IoEvents::IN);
    }

    fn close(&self) {
        self.inner.lock().closed = true;
        self.notify_worker();
    }
}

struct VhostVsockFile {
    state: Mutex<VhostVsockState>,
}

impl VhostVsockFile {
    fn new() -> Self {
        Self {
            state: Mutex::new(VhostVsockState::default()),
        }
    }

    fn check_vring_index(index: u32) -> Result<usize> {
        let index = usize::try_from(index)
            .map_err(|_| Error::with_message(Errno::EINVAL, "the vhost vring index is invalid"))?;
        if index >= VHOST_VSOCK_VRING_COUNT {
            return_errno_with_message!(Errno::EINVAL, "the vhost vring index is out of range");
        }

        Ok(index)
    }

    fn read_memory_regions(
        raw_ioctl: RawIoctl,
        mem: VhostMemory,
    ) -> Result<Vec<VhostMemoryRegion>> {
        let region_count = usize::try_from(mem.nregions).map_err(|_| {
            Error::with_message(Errno::EINVAL, "vhost memory region count is invalid")
        })?;
        if region_count > VHOST_VSOCK_MAX_MEMORY_REGIONS {
            return_errno_with_message!(Errno::EINVAL, "vhost memory region count is too large");
        }
        let regions_offset = raw_ioctl
            .arg()
            .checked_add(size_of::<VhostMemory>())
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "vhost memory table overflow"))?;
        let current =
            Task::current().ok_or_else(|| Error::with_message(Errno::ESRCH, "no current task"))?;
        let thread_local = current.as_thread_local().ok_or_else(|| {
            Error::with_message(Errno::EFAULT, "current task has no thread local")
        })?;
        let userspace = CurrentUserSpace::new(thread_local);
        let mut regions = Vec::with_capacity(region_count);

        for region_index in 0..region_count {
            let offset = region_index
                .checked_mul(size_of::<VhostMemoryRegion>())
                .and_then(|offset| regions_offset.checked_add(offset))
                .ok_or_else(|| {
                    Error::with_message(Errno::EINVAL, "vhost memory region offset overflow")
                })?;
            let region = userspace.read_val::<VhostMemoryRegion>(offset)?;
            if region.flags_padding != 0 {
                return_errno_with_message!(Errno::EINVAL, "vhost memory region flags must be zero");
            }
            regions.push(region);
        }

        Ok(regions)
    }

    /// Starts the data-plane worker once.
    /// The worker observes the guest's virtqueue activity
    /// via cross-process `Vmar::read_alien` reads.
    /// Returns `Ok(())` if the worker was started or is already running.
    fn ensure_worker_started(&self) -> Result<()> {
        let mut state = self.state.lock();
        match &state.worker {
            VhostWorkerState::Stopped => (),
            VhostWorkerState::Running { backend, .. } => {
                if backend.inputs.stop.load(Ordering::Acquire) {
                    return_errno_with_message!(Errno::EIO, "vhost-vsock worker has stopped");
                }
                return Ok(());
            }
            VhostWorkerState::Stopping { .. } => {
                return_errno_with_message!(Errno::EBUSY, "vhost-vsock worker is stopping");
            }
        }
        state.validate_ready_to_run()?;
        let guest_cid = state
            .guest_cid
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "vhost-vsock guest CID not set"))?;
        let stop = Arc::new(AtomicBool::new(false));
        let rx_queue = Arc::new(VhostRxQueue::new());
        let inputs = state.snapshot_for_worker(stop.clone(), rx_queue.clone())?;
        let backend = Arc::new(VhostVsockBackend::new(inputs));
        register_backend(guest_cid, backend.clone())?;
        let worker_backend = backend.clone();
        let worker = ThreadOptions::new(move || worker_loop(worker_backend)).spawn();
        state.worker = VhostWorkerState::Running {
            stop,
            thread: worker,
            backend,
        };
        Ok(())
    }

    fn stop_worker(&self) -> Result<()> {
        let stopped = self.state.lock().stop_worker()?;
        if let Some(stopped) = stopped {
            stopped.thread.join();
            self.finish_stopped_worker(stopped);
        }
        Ok(())
    }

    fn reset_owner(&self) -> Result<()> {
        let stopped = self.state.lock().begin_reset_owner()?;
        if let Some(stopped) = stopped {
            stopped.thread.join();
            unregister_backend_if_matches(&stopped.backend);
        }
        *self.state.lock() = VhostVsockState::default();
        Ok(())
    }

    fn finish_stopped_worker(&self, stopped: StoppedWorker) {
        let mut state = self.state.lock();
        state.vring_base[RX_VRING_INDEX] =
            stopped.backend.rx_last_avail.load(Ordering::Acquire) as u32;
        state.vring_base[TX_VRING_INDEX] =
            stopped.backend.tx_last_avail.load(Ordering::Acquire) as u32;
        unregister_backend_if_matches(&stopped.backend);
        state.worker = VhostWorkerState::Stopped;
    }

    fn current_vmar() -> Result<Arc<Vmar>> {
        let task =
            Task::current().ok_or_else(|| Error::with_message(Errno::ESRCH, "no current task"))?;
        let thread_local = task.as_thread_local().ok_or_else(|| {
            Error::with_message(Errno::EFAULT, "current task has no thread local")
        })?;
        thread_local
            .vmar()
            .borrow()
            .as_ref()
            .map(|vmar| vmar.clone_arc())
            .ok_or_else(|| Error::with_message(Errno::ESRCH, "current task has no VMAR"))
    }

    /// Captures the calling task's address space at `VHOST_SET_OWNER` time.
    fn capture_caller_owner() -> Option<Arc<Vmar>> {
        let task = Task::current()?;
        let thread_local = task.as_thread_local()?;
        Some(thread_local.vmar().borrow().as_ref()?.clone_arc())
    }

    fn get_event_file(raw_fd: RawFileDesc) -> Result<Option<Arc<dyn FileLike>>> {
        if raw_fd == -1 {
            return Ok(None);
        }

        let fd = FileDesc::try_from(raw_fd)?;
        let current =
            Task::current().ok_or_else(|| Error::with_message(Errno::ESRCH, "no current task"))?;
        let thread_local = current.as_thread_local().ok_or_else(|| {
            Error::with_message(Errno::EFAULT, "current task has no thread local")
        })?;
        let mut file_table = thread_local.borrow_file_table_mut();
        let file = get_file_fast!(&mut file_table, fd).into_owned();
        if !syscall::is_event_file(file.as_ref()) {
            return_errno_with_message!(Errno::EINVAL, "vhost-vsock vring file is not an eventfd");
        }
        Ok(Some(file))
    }
}

impl Pollable for VhostVsockFile {
    fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
    }
}

impl FileOps for VhostVsockFile {
    fn read_at(
        &self,
        _offset: usize,
        _writer: &mut VmWriter,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "vhost-vsock does not support read")
    }

    fn write_at(
        &self,
        _offset: usize,
        _reader: &mut VmReader,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "vhost-vsock does not support write")
    }
}

impl PerOpenFileOps for VhostVsockFile {
    fn check_seekable(&self) -> Result<()> {
        return_errno_with_message!(Errno::ESPIPE, "vhost-vsock is not seekable")
    }

    fn is_offset_aware(&self) -> bool {
        false
    }

    fn ioctl(&self, raw_ioctl: RawIoctl) -> Result<i32> {
        use ioctl_defs::*;

        dispatch_ioctl!(match raw_ioctl {
            cmd @ GetFeatures => {
                cmd.write(&VHOST_SUPPORTED_FEATURES)?;
                Ok(0)
            }
            cmd @ SetFeatures => {
                let features = cmd.read()?;
                if features & !VHOST_SUPPORTED_FEATURES != 0 {
                    return_errno_with_message!(
                        Errno::EINVAL,
                        "vhost-vsock feature bits are unsupported"
                    );
                }
                let mut state = self.state.lock();
                state.ensure_configurable()?;
                state.features = features;
                Ok(0)
            }
            SetOwner => {
                let mut state = self.state.lock();
                if state.owner_set {
                    return_errno_with_message!(Errno::EBUSY, "vhost-vsock owner is already set");
                }
                let Some(vmar) = Self::capture_caller_owner() else {
                    return_errno_with_message!(
                        Errno::EINVAL,
                        "vhost-vsock owner address space cannot be captured"
                    );
                };
                state.owner_set = true;
                state.owner_vmar = Some(vmar);
                Ok(0)
            }
            ResetOwner => {
                self.reset_owner()?;
                Ok(0)
            }
            cmd @ SetMemTable => {
                let mem = cmd.read()?;
                if mem.padding != 0 {
                    return_errno_with_message!(
                        Errno::EINVAL,
                        "vhost memory table padding must be zero"
                    );
                }
                self.state.lock().ensure_configurable()?;
                let memory_regions = Self::read_memory_regions(raw_ioctl, mem)?;
                for region in memory_regions.iter() {
                    validate_memory_region(region)?;
                }
                let mut state = self.state.lock();
                state.ensure_configurable()?;
                state.memory_regions = memory_regions;
                Ok(0)
            }
            cmd @ SetVringNum => {
                let vring_state = cmd.read()?;
                let index = Self::check_vring_index(vring_state.index)?;
                validate_vring_num(vring_state.num)?;
                let mut state = self.state.lock();
                state.ensure_configurable()?;
                state.vring_num[index] = vring_state.num;
                Ok(0)
            }
            cmd @ SetVringAddr => {
                let vring_addr = cmd.read()?;
                let index = Self::check_vring_index(vring_addr.index)?;
                let mut state = self.state.lock();
                state.ensure_configurable()?;
                validate_vring_addr(&vring_addr, state.vring_num[index])?;
                state.vring_addr[index] = Some(vring_addr);
                Ok(0)
            }
            cmd @ SetVringBase => {
                let vring_state = cmd.read()?;
                let index = Self::check_vring_index(vring_state.index)?;
                validate_vring_base(vring_state.num)?;
                let mut state = self.state.lock();
                state.ensure_configurable()?;
                state.vring_base[index] = vring_state.num;
                Ok(0)
            }
            cmd @ GetVringBase => {
                let mut vring_state = cmd.read()?;
                let index = Self::check_vring_index(vring_state.index)?;
                let state = self.state.lock();
                state.ensure_owner_set()?;
                vring_state.num = match &state.worker {
                    VhostWorkerState::Stopped => state.vring_base[index],
                    VhostWorkerState::Running { backend, .. } => match index {
                        0 => backend.rx_last_avail.load(Ordering::Acquire) as u32,
                        _ => backend.tx_last_avail.load(Ordering::Acquire) as u32,
                    },
                    VhostWorkerState::Stopping { .. } => {
                        return_errno_with_message!(Errno::EBUSY, "vhost-vsock worker is stopping");
                    }
                };
                cmd.write(&vring_state)?;
                Ok(0)
            }
            cmd @ SetVringKick => {
                let vring_file = cmd.read()?;
                let index = Self::check_vring_index(vring_file.index)?;
                self.state.lock().ensure_configurable()?;
                let file = Self::get_event_file(vring_file.fd)?;
                let mut state = self.state.lock();
                state.ensure_configurable()?;
                state.vring_kick[index] = file;
                Ok(0)
            }
            cmd @ SetVringCall => {
                let vring_file = cmd.read()?;
                let index = Self::check_vring_index(vring_file.index)?;
                self.state.lock().ensure_configurable()?;
                let file = Self::get_event_file(vring_file.fd)?;
                let mut state = self.state.lock();
                state.ensure_configurable()?;
                state.vring_call[index] = file;
                Ok(0)
            }
            cmd @ SetVringErr => {
                let vring_file = cmd.read()?;
                let index = Self::check_vring_index(vring_file.index)?;
                self.state.lock().ensure_configurable()?;
                let file = Self::get_event_file(vring_file.fd)?;
                let mut state = self.state.lock();
                state.ensure_configurable()?;
                state.vring_err[index] = file;
                Ok(0)
            }
            cmd @ SetBackendFeatures => {
                let features = cmd.read()?;
                if features & !VHOST_SUPPORTED_BACKEND_FEATURES != 0 {
                    return_errno_with_message!(
                        Errno::EINVAL,
                        "vhost-vsock backend feature bits are unsupported"
                    );
                }
                let mut state = self.state.lock();
                state.ensure_configurable()?;
                state.backend_features = features;
                Ok(0)
            }
            cmd @ GetBackendFeatures => {
                cmd.write(&VHOST_SUPPORTED_BACKEND_FEATURES)?;
                Ok(0)
            }
            cmd @ SetGuestCid => {
                let guest_cid = cmd.read()?;
                validate_guest_cid(guest_cid)?;
                let mut state = self.state.lock();
                state.ensure_configurable()?;
                state.guest_cid = Some(guest_cid);
                Ok(0)
            }
            cmd @ SetRunning => {
                let running = cmd.read()?;
                if running != 0 {
                    self.ensure_worker_started()?;
                    Ok(0)
                } else {
                    self.state.lock().ensure_owner_set()?;
                    self.stop_worker()?;
                    Ok(0)
                }
            }
            _ => return_errno_with_message!(Errno::ENOTTY, "the ioctl command is unknown"),
        })
    }
}

impl Drop for VhostVsockFile {
    fn drop(&mut self) {
        if let Ok(stopped) = self.state.lock().stop_worker()
            && let Some(stopped) = stopped
        {
            stopped.thread.join();
            unregister_backend_if_matches(&stopped.backend);
        }
    }
}

fn backend_registry() -> &'static Arc<SpinLock<BTreeMap<u64, Arc<VhostVsockBackend>>>> {
    BACKEND_REGISTRY.call_once(|| Arc::new(SpinLock::new(BTreeMap::new())))
}

fn backend_for_guest(guest_cid: u64) -> Option<Arc<VhostVsockBackend>> {
    backend_registry().lock().get(&guest_cid).cloned()
}

fn register_backend(guest_cid: u64, backend: Arc<VhostVsockBackend>) -> Result<()> {
    use alloc::collections::btree_map::Entry;

    let mut registry = backend_registry().lock();
    match registry.entry(guest_cid) {
        Entry::Vacant(entry) => {
            entry.insert(backend);
            Ok(())
        }
        Entry::Occupied(_) => {
            return_errno_with_message!(Errno::EBUSY, "the vhost-vsock guest CID is busy")
        }
    }
}

fn unregister_backend_if_matches(backend: &Arc<VhostVsockBackend>) {
    let guest_cid = backend.inputs.guest_cid;
    let mut registry = backend_registry().lock();
    if registry
        .get(&guest_cid)
        .is_some_and(|registered| Arc::ptr_eq(registered, backend))
    {
        registry.remove(&guest_cid);
    }
}

pub(crate) fn backend_exists(guest_cid: u64) -> bool {
    backend_registry()
        .lock()
        .get(&guest_cid)
        .is_some_and(|backend| !backend.inputs.stop.load(Ordering::Acquire))
}

fn validate_memory_region(region: &VhostMemoryRegion) -> Result<()> {
    if region.memory_size == 0 {
        return_errno_with_message!(Errno::EINVAL, "vhost memory region size is zero");
    }
    if !region.guest_phys_addr.is_multiple_of(VHOST_VSOCK_PAGE_SIZE)
        || !region.userspace_addr.is_multiple_of(VHOST_VSOCK_PAGE_SIZE)
        || !region.memory_size.is_multiple_of(VHOST_VSOCK_PAGE_SIZE)
    {
        return_errno_with_message!(Errno::EINVAL, "vhost memory region is not page aligned");
    }
    region
        .guest_phys_addr
        .checked_add(region.memory_size)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "vhost memory GPA range overflow"))?;
    checked_userspace_range(
        region.userspace_addr,
        region.memory_size,
        "vhost memory userspace range is invalid",
    )?;
    Ok(())
}

fn validate_vring_num(num: u32) -> Result<()> {
    if num == 0 || num > VHOST_VSOCK_MAX_VRING_NUM || !num.is_power_of_two() {
        return_errno_with_message!(Errno::EINVAL, "vhost-vsock vring size is invalid");
    }
    Ok(())
}

fn validate_vring_base(base: u32) -> Result<()> {
    if base > u16::MAX as u32 {
        return_errno_with_message!(Errno::EINVAL, "vhost-vsock vring base is too large");
    }
    Ok(())
}

fn validate_vring_addr(addr: &VhostVringAddr, vring_num: u32) -> Result<()> {
    if addr.flags != 0 || addr.log_guest_addr != 0 {
        return_errno_with_message!(Errno::EINVAL, "vhost-vsock vring logging is unsupported");
    }

    let num = if vring_num == 0 {
        VHOST_VSOCK_MAX_VRING_NUM as usize
    } else {
        vring_num as usize
    };
    let last_index = num - 1;
    let desc_len = num
        .checked_mul(size_of::<VirtqDesc>())
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "vhost-vsock desc table overflow"))?;
    let avail_len = VIRTQ_AVAIL_RING_OFFSET
        .checked_add(num.checked_mul(size_of::<u16>()).ok_or_else(|| {
            Error::with_message(Errno::EINVAL, "vhost-vsock available ring overflow")
        })?)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "vhost-vsock available ring overflow"))?;
    let used_len =
        VIRTQ_USED_RING_OFFSET
            .checked_add(num.checked_mul(size_of::<VirtqUsedElem>()).ok_or_else(|| {
                Error::with_message(Errno::EINVAL, "vhost-vsock used ring overflow")
            })?)
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "vhost-vsock used ring overflow"))?;

    checked_user_elem_addr(
        addr.desc_user_addr,
        0,
        last_index,
        size_of::<VirtqDesc>(),
        "vhost-vsock descriptor table address overflow",
    )?;
    checked_user_elem_addr(
        addr.avail_user_addr,
        VIRTQ_AVAIL_RING_OFFSET,
        last_index,
        size_of::<u16>(),
        "vhost-vsock available ring address overflow",
    )?;
    checked_user_elem_addr(
        addr.used_user_addr,
        VIRTQ_USED_RING_OFFSET,
        last_index,
        size_of::<VirtqUsedElem>(),
        "vhost-vsock used ring address overflow",
    )?;
    checked_userspace_range(
        addr.desc_user_addr,
        desc_len as u64,
        "vhost-vsock descriptor table range is invalid",
    )?;
    checked_userspace_range(
        addr.avail_user_addr,
        avail_len as u64,
        "vhost-vsock available ring range is invalid",
    )?;
    checked_userspace_range(
        addr.used_user_addr,
        used_len as u64,
        "vhost-vsock used ring range is invalid",
    )?;

    Ok(())
}

fn validate_guest_cid(guest_cid: u64) -> Result<()> {
    if guest_cid <= HOST_CID || guest_cid >= u32::MAX as u64 {
        return_errno_with_message!(Errno::EINVAL, "the guest CID is invalid");
    }
    Ok(())
}

/// Runs the per-device data-plane worker.
fn worker_loop(backend: Arc<VhostVsockBackend>) {
    let inputs = &backend.inputs;
    let kick_tx = inputs.vring_kick[TX_VRING_INDEX].clone();
    let kick_rx = inputs.vring_kick[RX_VRING_INDEX].clone();
    let mut rx_blocked = false;

    while !inputs.stop.load(Ordering::Acquire) {
        inputs.rx_queue.take_worker_wake();

        if let Some(kick) = kick_tx.as_ref() {
            consume_eventfd(kick.as_ref());
        }
        if kick_rx
            .as_ref()
            .is_some_and(|kick| consume_eventfd(kick.as_ref()))
        {
            rx_blocked = false;
        }

        if backend.process_tx().is_err() {
            break;
        }

        if !rx_blocked && inputs.rx_queue.has_packets() {
            match backend.drain_rx_queue() {
                RxDrainResult::Blocked => rx_blocked = true,
                RxDrainResult::Drained => rx_blocked = false,
                RxDrainResult::Stopped => break,
            }
        }

        if inputs.stop.load(Ordering::Acquire) {
            break;
        }

        wait_for_worker_wake(inputs);
    }

    inputs.stop.store(true, Ordering::Release);
    let _ = backend.process_tx();
    let _ = backend.drain_rx_queue();
    unregister_backend_if_matches(&backend);
}

fn wait_for_worker_wake(inputs: &WorkerInputs) {
    let mut poller = Poller::new(None);
    if inputs
        .rx_queue
        .poll_worker_wake(Some(poller.as_handle_mut()))
        .is_empty()
    {
        let _ = poller.wait();
    }
}

struct VhostMemorySegment {
    userspace_addr: usize,
    len: usize,
}

fn read_gpa_bytes(
    vmar: &Vmar,
    regions: &[VhostMemoryRegion],
    gpa: u64,
    len: usize,
) -> Result<Vec<u8>> {
    let segments = gpa_to_uva_segments(regions, gpa, len).ok_or_else(|| {
        Error::with_message(
            Errno::EFAULT,
            "vhost-vsock GPA range not covered by mem table",
        )
    })?;
    let mut bytes = vec![0; len];
    read_gpa_segments(vmar, segments.as_slice(), bytes.as_mut_slice())?;
    Ok(bytes)
}

fn read_gpa_segments(vmar: &Vmar, segments: &[VhostMemorySegment], bytes: &mut [u8]) -> Result<()> {
    let mut offset = 0usize;
    for segment in segments {
        let end = offset.checked_add(segment.len).ok_or_else(|| {
            Error::with_message(Errno::EINVAL, "vhost-vsock segment offset overflow")
        })?;
        let mut writer = VmWriter::from(&mut bytes[offset..end]).to_fallible();
        vmar.read_alien(segment.userspace_addr, &mut writer)
            .map_err(|(e, _)| e)?;
        offset = end;
    }
    Ok(())
}

fn gpa_to_uva_segments(
    regions: &[VhostMemoryRegion],
    gpa: u64,
    len: usize,
) -> Option<Vec<VhostMemorySegment>> {
    let end = gpa.checked_add(u64::try_from(len).ok()?)?;
    let mut current = gpa;
    let mut segments = Vec::new();

    while current < end {
        let region = regions.iter().find(|region| {
            let Some(region_end) = region.guest_phys_addr.checked_add(region.memory_size) else {
                return false;
            };
            current >= region.guest_phys_addr && current < region_end
        })?;
        let region_end = region.guest_phys_addr.checked_add(region.memory_size)?;
        let segment_end = region_end.min(end);
        let segment_len = usize::try_from(segment_end - current).ok()?;
        let offset = current - region.guest_phys_addr;
        let userspace_addr = region.userspace_addr.checked_add(offset)?;
        segments.push(VhostMemorySegment {
            userspace_addr: usize::try_from(userspace_addr).ok()?,
            len: segment_len,
        });
        current = segment_end;
    }

    Some(segments)
}

/// Injects a host-to-guest virtio-vsock packet into the RX queue.
fn inject_packet(
    inputs: &WorkerInputs,
    last_avail: &mut u16,
    packet: VhostVsockPacket<'_>,
) -> Result<()> {
    let addr = inputs.vring_addr[RX_VRING_INDEX];
    let num = inputs.vring_num[RX_VRING_INDEX] as usize;
    if num == 0 {
        return_errno_with_message!(Errno::EINVAL, "vhost-vsock RX vring num is zero");
    }
    let guest_cid = inputs.guest_cid;
    let call = inputs.vring_call[RX_VRING_INDEX].clone();

    let vmar = inputs.owner_vmar.as_ref();

    let mut avail = VirtqAvailHeader::default();
    let mut writer = VmWriter::from(avail.as_mut_bytes()).to_fallible();
    vmar.read_alien(
        checked_user_addr(
            addr.avail_user_addr,
            0,
            "vhost-vsock RX avail address overflow",
        )?,
        &mut writer,
    )
    .map_err(|(e, _)| e)?;

    let avail_delta = avail.idx.wrapping_sub(*last_avail) as usize;
    if avail_delta == 0 {
        return_errno_with_message!(
            Errno::EAGAIN,
            "vhost-vsock RX has no buffer published by guest yet"
        );
    }
    if avail_delta > num {
        return_errno_with_message!(
            Errno::EINVAL,
            "vhost-vsock RX avail ring delta exceeds queue size"
        );
    }

    let avail_slot = *last_avail as usize % num;
    let mut head_le: u16 = 0;
    let mut writer = VmWriter::from(head_le.as_mut_bytes()).to_fallible();
    vmar.read_alien(
        checked_user_elem_addr(
            addr.avail_user_addr,
            VIRTQ_AVAIL_RING_OFFSET,
            avail_slot,
            size_of::<u16>(),
            "vhost-vsock RX avail ring address overflow",
        )?,
        &mut writer,
    )
    .map_err(|(e, _)| e)?;
    let head = head_le as usize;
    if head >= num {
        return_errno_with_message!(Errno::EINVAL, "vhost-vsock head index out of range");
    }

    let packet_len = VIRTIO_VSOCK_HDR_SIZE
        .checked_add(packet.payload.len())
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "vhost-vsock packet too large"))?;
    let hdr = VirtioVsockHdr {
        src_cid: HOST_CID,
        dst_cid: guest_cid,
        src_port: packet.src_port,
        dst_port: packet.dst_port,
        len: packet.payload.len() as u32,
        type_: VIRTIO_VSOCK_TYPE_STREAM,
        op: packet.op,
        flags: packet.flags,
        buf_alloc: packet.buf_alloc,
        fwd_cnt: packet.fwd_cnt,
    };
    let bytes = hdr.to_bytes();
    write_rx_chain(
        vmar,
        addr,
        &inputs.mem_regions,
        num,
        head,
        bytes.as_slice(),
        packet.payload,
    )?;

    let mut used = VirtqUsedHeader::default();
    let mut writer = VmWriter::from(used.as_mut_bytes()).to_fallible();
    vmar.read_alien(
        checked_user_addr(
            addr.used_user_addr,
            0,
            "vhost-vsock RX used address overflow",
        )?,
        &mut writer,
    )
    .map_err(|(e, _)| e)?;
    let used_slot = used.idx as usize % num;
    let used_elem = VirtqUsedElem {
        id: head as u32,
        len: packet_len as u32,
    };
    let mut reader = VmReader::from(used_elem.as_bytes()).to_fallible();
    vmar.write_alien(
        checked_user_elem_addr(
            addr.used_user_addr,
            VIRTQ_USED_RING_OFFSET,
            used_slot,
            size_of::<VirtqUsedElem>(),
            "vhost-vsock RX used ring address overflow",
        )?,
        &mut reader,
    )
    .map_err(|(e, _)| e)?;

    used.idx = used.idx.wrapping_add(1);
    let mut reader = VmReader::from(used.as_bytes()).to_fallible();
    vmar.write_alien(
        checked_user_addr(
            addr.used_user_addr,
            0,
            "vhost-vsock RX used address overflow",
        )?,
        &mut reader,
    )
    .map_err(|(e, _)| e)?;

    *last_avail = last_avail.wrapping_add(1);

    if let Some(call) = call.as_ref() {
        signal_eventfd(call.as_ref());
    }

    Ok(())
}

/// Drains new entries from the TX queue.
fn process_tx(backend: &VhostVsockBackend, last_avail: &mut u16) -> Result<()> {
    let inputs = &backend.inputs;
    let addr = inputs.vring_addr[TX_VRING_INDEX];
    let num = inputs.vring_num[TX_VRING_INDEX] as usize;
    if num == 0 {
        return_errno_with_message!(Errno::EINVAL, "vhost-vsock TX vring num is zero");
    }
    let call = inputs.vring_call[TX_VRING_INDEX].clone();

    let vmar = inputs.owner_vmar.as_ref();

    let mut avail_hdr = VirtqAvailHeader::default();
    let mut writer = VmWriter::from(avail_hdr.as_mut_bytes()).to_fallible();
    vmar.read_alien(
        checked_user_addr(
            addr.avail_user_addr,
            0,
            "vhost-vsock TX avail address overflow",
        )?,
        &mut writer,
    )
    .map_err(|(e, _)| e)?;

    let mut consumed_any = false;
    let result = (|| -> Result<()> {
        let avail_delta = avail_hdr.idx.wrapping_sub(*last_avail) as usize;
        if avail_delta > num {
            return_errno_with_message!(
                Errno::EINVAL,
                "vhost-vsock TX avail ring delta exceeds queue size"
            );
        }

        for _ in 0..avail_delta {
            let slot = *last_avail as usize % num;
            let mut head_le: u16 = 0;
            let mut writer = VmWriter::from(head_le.as_mut_bytes()).to_fallible();
            vmar.read_alien(
                checked_user_elem_addr(
                    addr.avail_user_addr,
                    VIRTQ_AVAIL_RING_OFFSET,
                    slot,
                    size_of::<u16>(),
                    "vhost-vsock TX avail ring address overflow",
                )?,
                &mut writer,
            )
            .map_err(|(e, _)| e)?;
            let head = head_le as usize;
            if head >= num {
                return_errno_with_message!(Errno::EINVAL, "vhost-vsock TX head out of range");
            }

            let chain = read_tx_chain(vmar, addr, &inputs.mem_regions, num, head)?;
            if chain.bytes.len() >= VIRTIO_VSOCK_HDR_SIZE {
                let hdr = VirtioVsockHdr::from_bytes(&chain.bytes[..VIRTIO_VSOCK_HDR_SIZE])?;
                let payload_len = hdr.len as usize;
                let packet_len =
                    VIRTIO_VSOCK_HDR_SIZE
                        .checked_add(payload_len)
                        .ok_or_else(|| {
                            Error::with_message(Errno::EINVAL, "vhost-vsock TX packet too large")
                        })?;
                if chain.bytes.len() < packet_len {
                    complete_tx_chain(vmar, &addr, num, head, last_avail)?;
                    consumed_any = true;
                    continue;
                }
                let payload = chain.bytes[VIRTIO_VSOCK_HDR_SIZE..packet_len].to_vec();
                if !validate_tx_header_for_backend(backend, &hdr) {
                    complete_tx_chain(vmar, &addr, num, head, last_avail)?;
                    consumed_any = true;
                    continue;
                }
                deliver_tx_packet(backend, hdr, payload)?;
                complete_tx_chain(vmar, &addr, num, head, last_avail)?;
                consumed_any = true;
                continue;
            }

            complete_tx_chain(vmar, &addr, num, head, last_avail)?;
            consumed_any = true;
        }

        Ok(())
    })();

    if consumed_any && let Some(call) = call.as_ref() {
        signal_eventfd(call.as_ref());
    }

    result
}

/// Best-effort drain of an eventfd counter (8 bytes).
fn consume_eventfd(file: &dyn FileLike) -> bool {
    if file.poll(IoEvents::IN, None).is_empty() {
        return false;
    }

    let mut buf = [0u8; 8];
    let mut writer = VmWriter::from(buf.as_mut_slice()).to_fallible();
    syscall::read_event_file_nonblocking(file, &mut writer).is_ok()
}

fn signal_eventfd(file: &dyn FileLike) {
    if file.poll(IoEvents::OUT, None).is_empty() {
        return;
    }
    let _ = syscall::write_event_file_nonblocking(file, 1);
}

pub(crate) fn send_packet(header: &TransportVsockHdr, payload: &[u8]) -> Result<bool> {
    let src_cid = header.src_cid;
    let dst_cid = header.dst_cid;
    let src_port = header.src_port;
    let dst_port = header.dst_port;
    let op = header.op;
    let flags = header.flags;
    let buf_alloc = header.buf_alloc;
    let fwd_cnt = header.fwd_cnt;

    // This path may run while the vsock socket-table and connection-state
    // spinlocks are held, so it must not wait for the vhost backend here.
    let backend = backend_for_guest(dst_cid);
    let Some(backend) = backend else {
        return Ok(false);
    };

    if backend.inputs.stop.load(Ordering::Acquire)
        || src_cid != HOST_CID
        || dst_cid != backend.inputs.guest_cid
    {
        return Ok(false);
    }

    backend.inject(VhostVsockPacket {
        dst_port,
        src_port,
        op,
        flags,
        payload,
        buf_alloc,
        fwd_cnt,
    })
}

struct TxChain {
    bytes: Vec<u8>,
}

fn read_tx_chain(
    vmar: &Vmar,
    addr: VhostVringAddr,
    mem_regions: &[VhostMemoryRegion],
    num: usize,
    head: usize,
) -> Result<TxChain> {
    let first_desc = read_vring_desc(vmar, addr.desc_user_addr, num, head)?;
    if first_desc.flags & VIRTQ_DESC_F_WRITE != 0 {
        return_errno_with_message!(Errno::EINVAL, "vhost-vsock TX descriptor is writable");
    }
    if first_desc.flags & VIRTQ_DESC_F_INDIRECT != 0 {
        return read_indirect_tx_chain(vmar, mem_regions, num, first_desc);
    }

    read_direct_tx_chain(vmar, addr, mem_regions, num, first_desc)
}

fn write_rx_chain(
    vmar: &Vmar,
    addr: VhostVringAddr,
    mem_regions: &[VhostMemoryRegion],
    num: usize,
    head: usize,
    header: &[u8],
    payload: &[u8],
) -> Result<()> {
    let first_desc = read_vring_desc(vmar, addr.desc_user_addr, num, head)?;
    if first_desc.flags & VIRTQ_DESC_F_INDIRECT != 0 {
        return write_indirect_rx_chain(vmar, mem_regions, num, first_desc, header, payload);
    }

    write_direct_rx_chain(vmar, addr, mem_regions, num, first_desc, header, payload)
}

fn write_direct_rx_chain(
    vmar: &Vmar,
    addr: VhostVringAddr,
    mem_regions: &[VhostMemoryRegion],
    num: usize,
    first_desc: VirtqDesc,
    header: &[u8],
    payload: &[u8],
) -> Result<()> {
    write_rx_desc_chain(
        vmar,
        mem_regions,
        first_desc,
        num,
        header,
        payload,
        |index| read_vring_desc(vmar, addr.desc_user_addr, num, index),
    )
}

fn write_indirect_rx_chain(
    vmar: &Vmar,
    mem_regions: &[VhostMemoryRegion],
    queue_num: usize,
    first_desc: VirtqDesc,
    header: &[u8],
    payload: &[u8],
) -> Result<()> {
    let table_len = first_desc.len as usize;
    if table_len == 0 || !table_len.is_multiple_of(size_of::<VirtqDesc>()) {
        return_errno_with_message!(
            Errno::EINVAL,
            "vhost-vsock RX indirect descriptor table has invalid length"
        );
    }
    let table_num = table_len / size_of::<VirtqDesc>();
    if table_num > queue_num {
        return_errno_with_message!(
            Errno::EINVAL,
            "vhost-vsock RX indirect descriptor table is too large"
        );
    }
    let table = read_gpa_bytes(vmar, mem_regions, first_desc.addr, table_len)?;

    let first_indirect_desc = read_indirect_desc(table.as_slice(), table_num, 0)?;
    write_rx_desc_chain(
        vmar,
        mem_regions,
        first_indirect_desc,
        table_num,
        header,
        payload,
        |index| read_indirect_desc(table.as_slice(), table_num, index),
    )
}

fn write_rx_desc_chain(
    vmar: &Vmar,
    mem_regions: &[VhostMemoryRegion],
    start_desc: VirtqDesc,
    num: usize,
    header: &[u8],
    payload: &[u8],
    mut read_desc: impl FnMut(usize) -> Result<VirtqDesc>,
) -> Result<()> {
    let mut remaining_header = header;
    let mut remaining_payload = payload;
    let mut desc = start_desc;

    for _ in 0..num {
        if desc.flags & VIRTQ_DESC_F_INDIRECT != 0 {
            return_errno_with_message!(
                Errno::EINVAL,
                "vhost-vsock RX nested indirect descriptor is unsupported"
            );
        }
        if desc.flags & VIRTQ_DESC_F_WRITE == 0 {
            return_errno_with_message!(Errno::EINVAL, "vhost-vsock RX descriptor is not writable");
        }

        let desc_len = desc.len as usize;
        let desc_segments =
            gpa_to_uva_segments(mem_regions, desc.addr, desc_len).ok_or_else(|| {
                Error::with_message(
                    Errno::EFAULT,
                    "vhost-vsock RX desc.addr not covered by mem table",
                )
            })?;
        let written = write_rx_desc_bytes(
            vmar,
            desc_segments.as_slice(),
            &mut remaining_header,
            &mut remaining_payload,
        )?;
        if remaining_header.is_empty() && remaining_payload.is_empty() {
            return Ok(());
        }
        if written == 0 {
            return_errno_with_message!(Errno::EINVAL, "vhost-vsock RX descriptor is empty");
        }

        if desc.flags & VIRTQ_DESC_F_NEXT == 0 {
            return_errno_with_message!(Errno::EINVAL, "vhost-vsock RX buffer too small");
        }

        let next = desc.next as usize;
        if next >= num {
            return_errno_with_message!(
                Errno::EINVAL,
                "vhost-vsock RX chain next index out of range"
            );
        }
        desc = read_desc(next)?;
    }

    return_errno_with_message!(Errno::EINVAL, "vhost-vsock RX descriptor chain loop");
}

fn write_rx_desc_bytes(
    vmar: &Vmar,
    segments: &[VhostMemorySegment],
    remaining_header: &mut &[u8],
    remaining_payload: &mut &[u8],
) -> Result<usize> {
    let mut written = 0;

    for segment in segments {
        let mut user_addr = segment.userspace_addr;
        let mut segment_len = segment.len;

        let header_len = segment_len.min(remaining_header.len());
        if header_len != 0 {
            let mut reader = VmReader::from(&remaining_header[..header_len]).to_fallible();
            vmar.write_alien(user_addr, &mut reader)
                .map_err(|(e, _)| e)?;
            *remaining_header = &remaining_header[header_len..];
            user_addr = user_addr.checked_add(header_len).ok_or_else(|| {
                Error::with_message(Errno::EINVAL, "vhost-vsock RX descriptor address overflow")
            })?;
            segment_len -= header_len;
            written += header_len;
        }

        let payload_len = segment_len.min(remaining_payload.len());
        if payload_len != 0 {
            let mut reader = VmReader::from(&remaining_payload[..payload_len]).to_fallible();
            vmar.write_alien(user_addr, &mut reader)
                .map_err(|(e, _)| e)?;
            *remaining_payload = &remaining_payload[payload_len..];
            written += payload_len;
        }

        if remaining_header.is_empty() && remaining_payload.is_empty() {
            break;
        }
    }

    Ok(written)
}

fn read_direct_tx_chain(
    vmar: &Vmar,
    addr: VhostVringAddr,
    mem_regions: &[VhostMemoryRegion],
    num: usize,
    first_desc: VirtqDesc,
) -> Result<TxChain> {
    read_tx_desc_chain(vmar, mem_regions, first_desc, num, |index| {
        read_vring_desc(vmar, addr.desc_user_addr, num, index)
    })
}

fn read_indirect_tx_chain(
    vmar: &Vmar,
    mem_regions: &[VhostMemoryRegion],
    queue_num: usize,
    first_desc: VirtqDesc,
) -> Result<TxChain> {
    let table_len = first_desc.len as usize;
    if table_len == 0 || !table_len.is_multiple_of(size_of::<VirtqDesc>()) {
        return_errno_with_message!(
            Errno::EINVAL,
            "vhost-vsock TX indirect descriptor table has invalid length"
        );
    }
    let table_num = table_len / size_of::<VirtqDesc>();
    if table_num > queue_num {
        return_errno_with_message!(
            Errno::EINVAL,
            "vhost-vsock TX indirect descriptor table is too large"
        );
    }
    let table = read_gpa_bytes(vmar, mem_regions, first_desc.addr, table_len)?;

    let first_indirect_desc = read_indirect_desc(table.as_slice(), table_num, 0)?;
    read_tx_desc_chain(vmar, mem_regions, first_indirect_desc, table_num, |index| {
        read_indirect_desc(table.as_slice(), table_num, index)
    })
}

fn read_tx_desc_chain(
    vmar: &Vmar,
    mem_regions: &[VhostMemoryRegion],
    start_desc: VirtqDesc,
    num: usize,
    mut read_desc: impl FnMut(usize) -> Result<VirtqDesc>,
) -> Result<TxChain> {
    let mut bytes = Vec::new();
    let mut desc = start_desc;

    for _ in 0..num {
        if desc.flags & VIRTQ_DESC_F_INDIRECT != 0 {
            return_errno_with_message!(
                Errno::EINVAL,
                "vhost-vsock TX nested indirect descriptor is unsupported"
            );
        }
        if desc.flags & VIRTQ_DESC_F_WRITE != 0 {
            return_errno_with_message!(Errno::EINVAL, "vhost-vsock TX descriptor is writable");
        }

        append_tx_desc_bytes(vmar, mem_regions, desc, &mut bytes)?;

        if desc.flags & VIRTQ_DESC_F_NEXT == 0 {
            return Ok(TxChain { bytes });
        }

        let next = desc.next as usize;
        if next >= num {
            return_errno_with_message!(
                Errno::EINVAL,
                "vhost-vsock TX chain next index out of range"
            );
        }
        desc = read_desc(next)?;
    }

    return_errno_with_message!(Errno::EINVAL, "vhost-vsock TX descriptor chain loop");
}

fn read_vring_desc(
    vmar: &Vmar,
    desc_user_addr: u64,
    num: usize,
    index: usize,
) -> Result<VirtqDesc> {
    if index >= num {
        return_errno_with_message!(Errno::EINVAL, "vhost-vsock TX chain index out of range");
    }

    read_desc_at(
        vmar,
        checked_user_elem_addr(
            desc_user_addr,
            0,
            index,
            size_of::<VirtqDesc>(),
            "vhost-vsock descriptor table address overflow",
        )?,
    )
}

fn read_indirect_desc(table: &[u8], table_num: usize, index: usize) -> Result<VirtqDesc> {
    if index >= table_num {
        return_errno_with_message!(
            Errno::EINVAL,
            "vhost-vsock TX indirect chain index out of range"
        );
    }

    let offset = index.checked_mul(size_of::<VirtqDesc>()).ok_or_else(|| {
        Error::with_message(
            Errno::EINVAL,
            "vhost-vsock indirect descriptor table overflow",
        )
    })?;
    let desc_bytes = table
        .get(offset..offset + size_of::<VirtqDesc>())
        .ok_or_else(|| {
            Error::with_message(
                Errno::EINVAL,
                "vhost-vsock indirect descriptor table overflow",
            )
        })?;
    VirtqDesc::from_bytes(desc_bytes)
}

fn read_desc_at(vmar: &Vmar, user_addr: usize) -> Result<VirtqDesc> {
    let mut desc = VirtqDesc::default();
    let mut writer = VmWriter::from(desc.as_mut_bytes()).to_fallible();
    vmar.read_alien(user_addr, &mut writer)
        .map_err(|(e, _)| e)?;
    Ok(desc)
}

fn append_tx_desc_bytes(
    vmar: &Vmar,
    mem_regions: &[VhostMemoryRegion],
    desc: VirtqDesc,
    bytes: &mut Vec<u8>,
) -> Result<()> {
    let desc_len = desc.len as usize;
    let new_len = bytes
        .len()
        .checked_add(desc_len)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "vhost-vsock TX chain too large"))?;
    if new_len > VHOST_VSOCK_MAX_TX_CHAIN_BYTES {
        return_errno_with_message!(Errno::EINVAL, "vhost-vsock TX chain exceeds size limit");
    }
    let desc_segments = gpa_to_uva_segments(mem_regions, desc.addr, desc_len).ok_or_else(|| {
        Error::with_message(
            Errno::EFAULT,
            "vhost-vsock TX desc.addr not covered by mem table",
        )
    })?;
    let old_len = bytes.len();
    bytes.resize(new_len, 0);
    read_gpa_segments(vmar, desc_segments.as_slice(), &mut bytes[old_len..])?;
    Ok(())
}

fn validate_tx_header_for_backend(backend: &VhostVsockBackend, hdr: &VirtioVsockHdr) -> bool {
    hdr.dst_cid == HOST_CID && hdr.src_cid == backend.inputs.guest_cid
}

fn deliver_tx_packet(
    backend: &VhostVsockBackend,
    hdr: VirtioVsockHdr,
    payload: Vec<u8>,
) -> Result<()> {
    if !validate_tx_header_for_backend(backend, &hdr) {
        return_errno_with_message!(Errno::EINVAL, "vhost-vsock TX CID mismatch");
    }
    let Some(op) = VirtioVsockOp::try_from(hdr.op).ok() else {
        return Ok(());
    };
    let transport_header = TransportVsockHdr::new(
        hdr.src_cid,
        hdr.dst_cid,
        hdr.src_port,
        hdr.dst_port,
        hdr.len,
        op,
        hdr.flags,
        hdr.buf_alloc,
        hdr.fwd_cnt,
    );

    crate::net::socket::vsock::handle_vhost_packet(transport_header, payload)?;

    Ok(())
}

fn complete_tx_chain(
    vmar: &Vmar,
    addr: &VhostVringAddr,
    num: usize,
    head: usize,
    last_avail: &mut u16,
) -> Result<()> {
    publish_tx_used(vmar, addr, num, head)?;
    *last_avail = last_avail.wrapping_add(1);
    Ok(())
}

fn publish_tx_used(vmar: &Vmar, addr: &VhostVringAddr, num: usize, head: usize) -> Result<()> {
    let mut used = VirtqUsedHeader::default();
    let mut writer = VmWriter::from(used.as_mut_bytes()).to_fallible();
    vmar.read_alien(
        checked_user_addr(
            addr.used_user_addr,
            0,
            "vhost-vsock TX used address overflow",
        )?,
        &mut writer,
    )
    .map_err(|(e, _)| e)?;
    let used_slot = used.idx as usize % num;
    let used_elem = VirtqUsedElem {
        id: head as u32,
        len: 0,
    };
    let mut reader = VmReader::from(used_elem.as_bytes()).to_fallible();
    vmar.write_alien(
        checked_user_elem_addr(
            addr.used_user_addr,
            VIRTQ_USED_RING_OFFSET,
            used_slot,
            size_of::<VirtqUsedElem>(),
            "vhost-vsock TX used ring address overflow",
        )?,
        &mut reader,
    )
    .map_err(|(e, _)| e)?;
    used.idx = used.idx.wrapping_add(1);
    let mut reader = VmReader::from(used.as_bytes()).to_fallible();
    vmar.write_alien(
        checked_user_addr(
            addr.used_user_addr,
            0,
            "vhost-vsock TX used address overflow",
        )?,
        &mut reader,
    )
    .map_err(|(e, _)| e)?;
    Ok(())
}

fn checked_userspace_range(base: u64, len: u64, message: &'static str) -> Result<()> {
    let base = usize::try_from(base).map_err(|_| Error::with_message(Errno::EINVAL, message))?;
    let len = usize::try_from(len).map_err(|_| Error::with_message(Errno::EINVAL, message))?;
    if base < VMAR_LOWEST_ADDR
        || VMAR_CAP_ADDR
            .checked_sub(base)
            .is_none_or(|remaining| remaining < len)
    {
        return_errno_with_message!(Errno::EINVAL, message);
    }
    Ok(())
}

fn checked_user_addr(base: u64, offset: usize, message: &'static str) -> Result<usize> {
    let base = usize::try_from(base).map_err(|_| Error::with_message(Errno::EINVAL, message))?;
    base.checked_add(offset)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, message))
}

fn checked_user_elem_addr(
    base: u64,
    header_len: usize,
    index: usize,
    elem_size: usize,
    message: &'static str,
) -> Result<usize> {
    let elem_offset = index
        .checked_mul(elem_size)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, message))?;
    let offset = header_len
        .checked_add(elem_offset)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, message))?;
    checked_user_addr(base, offset, message)
}

pub(super) fn init() -> Result<()> {
    char::register(VhostVsockDevice::new())
}

const VHOST_VSOCK_VRING_COUNT: usize = 2;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
struct VhostMemory {
    nregions: u32,
    padding: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
struct VhostMemoryRegion {
    guest_phys_addr: u64,
    memory_size: u64,
    userspace_addr: u64,
    flags_padding: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
struct VhostVringState {
    index: u32,
    num: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
struct VhostVringFile {
    index: u32,
    fd: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
struct VhostVringAddr {
    index: u32,
    flags: u32,
    desc_user_addr: u64,
    used_user_addr: u64,
    avail_user_addr: u64,
    log_guest_addr: u64,
}

// Virtio split-virtqueue layout structures, as specified by the OASIS Virtio
// split virtqueue format and mirrored by Linux UAPI `virtio_ring.h`.

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
struct VirtqDesc {
    addr: u64,
    len: u32,
    flags: u16,
    next: u16,
}

impl VirtqDesc {
    fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let bytes = bytes.get(..size_of::<Self>()).ok_or_else(|| {
            Error::with_message(Errno::EINVAL, "vhost-vsock descriptor is truncated")
        })?;

        Ok(Self {
            addr: u64::from_le_bytes(bytes[0..8].try_into().map_err(|_| {
                Error::with_message(Errno::EINVAL, "vhost-vsock descriptor addr is malformed")
            })?),
            len: u32::from_le_bytes(bytes[8..12].try_into().map_err(|_| {
                Error::with_message(Errno::EINVAL, "vhost-vsock descriptor len is malformed")
            })?),
            flags: u16::from_le_bytes(bytes[12..14].try_into().map_err(|_| {
                Error::with_message(Errno::EINVAL, "vhost-vsock descriptor flags are malformed")
            })?),
            next: u16::from_le_bytes(bytes[14..16].try_into().map_err(|_| {
                Error::with_message(Errno::EINVAL, "vhost-vsock descriptor next is malformed")
            })?),
        })
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
struct VirtqAvailHeader {
    flags: u16,
    idx: u16,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
struct VirtqUsedHeader {
    flags: u16,
    idx: u16,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
struct VirtqUsedElem {
    id: u32,
    len: u32,
}

/// virtio-vsock packet header (44 bytes, all little-endian).
///
/// We build/serialize this as a flat byte buffer
/// to side-step `Pod`'s no-padding requirement.
/// The natural `repr(C)` layout has 4 bytes of trailing padding
/// for `u64` alignment.
///
/// Reference: Linux `include/uapi/linux/virtio_vsock.h`.
#[derive(Clone, Copy, Debug, Default)]
struct VirtioVsockHdr {
    src_cid: u64,
    dst_cid: u64,
    src_port: u32,
    dst_port: u32,
    len: u32,
    type_: u16,
    op: u16,
    flags: u32,
    buf_alloc: u32,
    fwd_cnt: u32,
}

const VIRTIO_VSOCK_HDR_SIZE: usize = 44;

impl VirtioVsockHdr {
    fn to_bytes(self) -> [u8; VIRTIO_VSOCK_HDR_SIZE] {
        let mut b = [0u8; VIRTIO_VSOCK_HDR_SIZE];
        b[0..8].copy_from_slice(&self.src_cid.to_le_bytes());
        b[8..16].copy_from_slice(&self.dst_cid.to_le_bytes());
        b[16..20].copy_from_slice(&self.src_port.to_le_bytes());
        b[20..24].copy_from_slice(&self.dst_port.to_le_bytes());
        b[24..28].copy_from_slice(&self.len.to_le_bytes());
        b[28..30].copy_from_slice(&self.type_.to_le_bytes());
        b[30..32].copy_from_slice(&self.op.to_le_bytes());
        b[32..36].copy_from_slice(&self.flags.to_le_bytes());
        b[36..40].copy_from_slice(&self.buf_alloc.to_le_bytes());
        b[40..44].copy_from_slice(&self.fwd_cnt.to_le_bytes());
        b
    }
}

const VIRTIO_VSOCK_TYPE_STREAM: u16 = 1;
impl VirtioVsockHdr {
    fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let bytes = bytes.get(..VIRTIO_VSOCK_HDR_SIZE).ok_or_else(|| {
            Error::with_message(Errno::EINVAL, "vhost-vsock packet header is truncated")
        })?;

        Ok(Self {
            src_cid: u64::from_le_bytes(bytes[0..8].try_into().map_err(|_| {
                Error::with_message(Errno::EINVAL, "vhost-vsock packet src CID is malformed")
            })?),
            dst_cid: u64::from_le_bytes(bytes[8..16].try_into().map_err(|_| {
                Error::with_message(Errno::EINVAL, "vhost-vsock packet dst CID is malformed")
            })?),
            src_port: u32::from_le_bytes(bytes[16..20].try_into().map_err(|_| {
                Error::with_message(Errno::EINVAL, "vhost-vsock packet src port is malformed")
            })?),
            dst_port: u32::from_le_bytes(bytes[20..24].try_into().map_err(|_| {
                Error::with_message(Errno::EINVAL, "vhost-vsock packet dst port is malformed")
            })?),
            len: u32::from_le_bytes(bytes[24..28].try_into().map_err(|_| {
                Error::with_message(Errno::EINVAL, "vhost-vsock packet length is malformed")
            })?),
            type_: u16::from_le_bytes(bytes[28..30].try_into().map_err(|_| {
                Error::with_message(Errno::EINVAL, "vhost-vsock packet type is malformed")
            })?),
            op: u16::from_le_bytes(bytes[30..32].try_into().map_err(|_| {
                Error::with_message(Errno::EINVAL, "vhost-vsock packet op is malformed")
            })?),
            flags: u32::from_le_bytes(bytes[32..36].try_into().map_err(|_| {
                Error::with_message(Errno::EINVAL, "vhost-vsock packet flags are malformed")
            })?),
            buf_alloc: u32::from_le_bytes(bytes[36..40].try_into().map_err(|_| {
                Error::with_message(
                    Errno::EINVAL,
                    "vhost-vsock packet buffer alloc is malformed",
                )
            })?),
            fwd_cnt: u32::from_le_bytes(bytes[40..44].try_into().map_err(|_| {
                Error::with_message(
                    Errno::EINVAL,
                    "vhost-vsock packet forward count is malformed",
                )
            })?),
        })
    }
}

mod ioctl_defs {
    use super::{VhostMemory, VhostVringAddr, VhostVringFile, VhostVringState};
    use crate::util::ioctl::{InData, InOutData, NoData, OutData, ioc};

    // Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/linux/vhost.h>.
    pub(super) type GetFeatures        = ioc!(VHOST_GET_FEATURES,         0xaf, 0x00, OutData<u64>);
    pub(super) type SetFeatures        = ioc!(VHOST_SET_FEATURES,         0xaf, 0x00, InData<u64>);
    pub(super) type SetOwner           = ioc!(VHOST_SET_OWNER,            0xaf, 0x01, NoData);
    pub(super) type ResetOwner         = ioc!(VHOST_RESET_OWNER,          0xaf, 0x02, NoData);
    pub(super) type SetMemTable        = ioc!(VHOST_SET_MEM_TABLE,        0xaf, 0x03, InData<VhostMemory>);
    pub(super) type SetVringNum        = ioc!(VHOST_SET_VRING_NUM,        0xaf, 0x10, InData<VhostVringState>);
    pub(super) type SetVringAddr       = ioc!(VHOST_SET_VRING_ADDR,       0xaf, 0x11, InData<VhostVringAddr>);
    pub(super) type SetVringBase       = ioc!(VHOST_SET_VRING_BASE,       0xaf, 0x12, InData<VhostVringState>);
    pub(super) type GetVringBase       = ioc!(VHOST_GET_VRING_BASE,       0xaf, 0x12, InOutData<VhostVringState>);
    pub(super) type SetVringKick       = ioc!(VHOST_SET_VRING_KICK,       0xaf, 0x20, InData<VhostVringFile>);
    pub(super) type SetVringCall       = ioc!(VHOST_SET_VRING_CALL,       0xaf, 0x21, InData<VhostVringFile>);
    pub(super) type SetVringErr        = ioc!(VHOST_SET_VRING_ERR,        0xaf, 0x22, InData<VhostVringFile>);
    pub(super) type SetBackendFeatures = ioc!(VHOST_SET_BACKEND_FEATURES, 0xaf, 0x25, InData<u64>);
    pub(super) type GetBackendFeatures = ioc!(VHOST_GET_BACKEND_FEATURES, 0xaf, 0x26, OutData<u64>);
    pub(super) type SetGuestCid        = ioc!(VHOST_VSOCK_SET_GUEST_CID,  0xaf, 0x60, InData<u64>);
    pub(super) type SetRunning         = ioc!(VHOST_VSOCK_SET_RUNNING,    0xaf, 0x61, InData<i32>);
}
