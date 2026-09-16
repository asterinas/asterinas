// SPDX-License-Identifier: MPL-2.0

//! A vhost-vsock device backed by the common vhost split queues.
//!
//! Each open file owns a persistent device and endpoint. SET_OWNER creates a
//! common worker bound to the owner's VMAR; SET_RUNNING pauses or resumes its queues.
//! A sleeping mutex serializes guest accesses with configuration changes, and
//! borrowed descriptor chains cannot outlive that lock. Close wakes and joins
//! the worker without holding the queue, socket or pending locks.
//!
//! Queue 0 receives host packets; queue 1 transmits guest packets. CID routing
//! remains available while paused, so sockets can enqueue bounded pending data.
//! Owner-memory faults and malformed queues signal the affected error eventfd;
//! a later kick or configuration wake can retry after the owner repairs them.
//! Control-buffer exhaustion remains a fatal resource policy: it invalidates
//! reservations and resets sockets because control messages cannot be dropped.
//!
//! Supported features are VERSION_1 and indirect descriptors for stream packets.
//! Event-index, packed rings, logging, IOTLB and seqpacket are not implemented.
//! This backend requires the host transport, without an active virtio frontend.

use core::sync::atomic::{AtomicU32, Ordering};

use aster_virtio::{Feature, device::socket::header::VirtioVsockHdr};
use device_id::{DeviceId, MinorId};
use ostd::task::Task;

use super::common::device::{
    VhostDeviceConfig, VhostSession, VhostSharedData, ioctl_defs::SetOwner,
};
use crate::{
    device::{Device, DeviceType, registry::char},
    events::IoEvents,
    fs::{
        devtmpfs::DevtmpfsNodeMeta,
        file::{PerOpenFileOps, StatusFlags},
        vfs::{inode::FileOps, path::Path},
    },
    net::socket::vsock::{self, VMADDR_CID_HOST},
    prelude::*,
    process::signal::{PollHandle, Pollable},
    util::ioctl::{RawIoctl, dispatch_ioctl},
};

mod packet;
mod work;

#[cfg(ktest)]
mod tests;

use packet::{Packet, PendingPackets};

const VHOST_VSOCK_MINOR: u32 = 241;
const RX_QUEUE: usize = 0;
const TX_QUEUE: usize = 1;
const NUM_QUEUES: usize = 2;
pub(crate) const MAX_PAYLOAD_SIZE: usize = packet::MAX_PAYLOAD_LEN;

// Registry entries reserve a CID even while its device is stopped. The weak
// reference cannot keep a closed file or its worker alive.
static BACKENDS: SpinLock<BTreeMap<u32, Weak<Backend>>> = SpinLock::new(BTreeMap::new());

mod ioctl_defs {
    use crate::util::ioctl::{InData, ioc};

    // Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/linux/vhost.h#L146>.
    pub(super) type SetGuestCid = ioc!(VHOST_VSOCK_SET_GUEST_CID, 0xaf, 0x60, InData<u64>);
    pub(super) type SetRunning = ioc!(VHOST_VSOCK_SET_RUNNING, 0xaf, 0x61, InData<i32>);
}

#[derive(Debug)]
struct VhostVsockDevice {
    id: DeviceId,
}

impl Device for VhostVsockDevice {
    fn type_(&self) -> DeviceType {
        DeviceType::Char
    }

    fn id(&self) -> DeviceId {
        self.id
    }

    fn devtmpfs_meta(&self) -> Option<DevtmpfsNodeMeta> {
        Some(DevtmpfsNodeMeta::new("vhost-vsock").unwrap())
    }

    fn open(&self) -> Result<Box<dyn PerOpenFileOps>> {
        Ok(Box::new(VhostVsockFile {
            backend: Backend::new(),
        }))
    }
}

struct VhostVsockFile {
    backend: Arc<Backend>,
}

impl Pollable for VhostVsockFile {
    fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        mask & (IoEvents::IN | IoEvents::OUT)
    }
}

impl FileOps for VhostVsockFile {
    fn read_at(
        &self,
        _offset: usize,
        _writer: &mut VmWriter,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "the file does not support reading")
    }

    fn write_at(
        &self,
        _offset: usize,
        _reader: &mut VmReader,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "the file does not support writing")
    }
}

impl PerOpenFileOps for VhostVsockFile {
    fn check_seekable(&self) -> Result<()> {
        return_errno_with_message!(Errno::ESPIPE, "the file does not support seeking")
    }

    fn is_offset_aware(&self) -> bool {
        false
    }

    fn ioctl(&self, _path: &Path, raw: RawIoctl) -> Result<i32> {
        self.backend.ioctl(raw)
    }
}

impl Drop for VhostVsockFile {
    fn drop(&mut self) {
        // The worker and socket reservations can retain Backend after close.
        // Shutdown must therefore happen here, before the last Arc is dropped.
        self.backend.shutdown();
    }
}

// Lock order: session -> runtime -> socket/pending locks. The worker
// never takes the session mutex; socket callbacks take neither sleeping mutex.
// No guest copy holds a pending spinlock.
struct Backend {
    common: Mutex<VhostSession<NUM_QUEUES>>,
    shared: Arc<VhostSharedData<NUM_QUEUES>>,
    // Zero means that SET_GUEST_CID has not assigned a route yet.
    cid: AtomicU32,
    pending: SpinLock<PendingPackets>,
}

impl Backend {
    fn new() -> Arc<Self> {
        let common = VhostSession::new(VhostDeviceConfig {
            device_features: (Feature::VERSION_1 | Feature::RING_INDIRECT_DESC).bits(),
            backend_features: 0,
            max_queue_size: 32768,
        });
        let shared = common.shared().clone();
        Arc::new(Self {
            common: Mutex::new(common),
            shared,
            cid: AtomicU32::new(0),
            pending: SpinLock::new(PendingPackets::new()),
        })
    }

    fn ioctl(self: &Arc<Self>, raw: RawIoctl) -> Result<i32> {
        use ioctl_defs::*;

        let mut device = self.common.lock();
        dispatch_ioctl!(match raw {
            SetOwner => {
                let task = Task::current().unwrap();
                let thread_local = task.as_thread_local().unwrap();
                let vmar = thread_local.vmar().borrow().as_ref().unwrap().clone_arc();
                device.set_owner(vmar, self.clone())?;
                Ok(0)
            }
            cmd @ SetGuestCid => {
                // CID reservations belong to the file, independently of owner.
                self.set_guest_cid(cmd.read()?)?;
                Ok(0)
            }
            cmd @ SetRunning => {
                let running = cmd.read()?;
                self.shared.lock().check_owner()?;
                if running != 0 {
                    self.start(&mut device)?;
                } else {
                    device.disable_queues();
                }
                Ok(0)
            }
            _ => device.handle_ioctl(raw),
        })
    }

    fn set_guest_cid(self: &Arc<Self>, cid: u64) -> Result<()> {
        let cid = validate_guest_cid(cid)?;
        let _common = self.shared.lock();
        let mut backends = BACKENDS.lock();
        if let Some(other) = backends.get(&cid).and_then(Weak::upgrade)
            && !Arc::ptr_eq(&other, self)
        {
            return_errno_with_message!(Errno::EADDRINUSE, "the guest CID is already in use");
        }
        let old_cid = self.cid();
        if old_cid == cid {
            return Ok(());
        }
        if old_cid != 0 {
            backends.remove(&old_cid);
        }
        self.cid.store(cid, Ordering::Release);
        backends.insert(cid, Arc::downgrade(self));
        drop(backends);
        self.shared.worker_pollee().notify(IoEvents::IN);
        Ok(())
    }

    fn start(self: &Arc<Self>, device: &mut VhostSession<NUM_QUEUES>) -> Result<()> {
        vsock::ensure_vhost_backend()?;
        if !self.pending.lock().is_active {
            // Join before reactivation so deferred cleanup cannot reset new
            // connections. The stop request may precede the worker's cleanup.
            device.stop_worker();
            self.reset_failed_endpoint();
            {
                let mut pending = self.pending.lock();
                pending.is_active = true;
            }
            device.start_worker(self.clone())?;
        }
        device.enable_queues()
    }

    fn shutdown(&self) {
        let mut device = self.common.lock();
        device.disable_queues();
        {
            let mut pending = self.pending.lock();
            pending.is_active = false;
            pending.generation = pending.generation.wrapping_add(1);
        }
        device.stop_worker();
        let cid = self.cid();
        if cid != 0 {
            // Keep the CID reserved until old sockets are reset, so a new
            // session cannot inherit a reset intended for this session.
            vsock::reset_vhost_orphaned_connections();
            BACKENDS.lock().remove(&cid);
            self.cid.store(0, Ordering::Release);
        }
        self.pending.lock().discard();
        // Reservations may retain Backend; release owner resources on close.
        device.reset_owner();
    }

    fn cid(&self) -> u32 {
        self.cid.load(Ordering::Acquire)
    }

    fn is_active(&self) -> bool {
        self.pending.lock().is_active
    }
}

fn validate_guest_cid(cid: u64) -> Result<u32> {
    if cid <= u64::from(VMADDR_CID_HOST) || cid >= u64::from(u32::MAX) {
        return_errno_with_message!(Errno::EINVAL, "the guest CID is reserved or out of range");
    }
    Ok(cid as u32)
}

pub(crate) fn can_connect_remote_cid(cid: u32) -> bool {
    let backend = BACKENDS.lock().get(&cid).and_then(Weak::upgrade);
    backend.is_some_and(|backend| backend.is_active())
}

/// Queues a host packet without sleeping or touching guest memory.
pub(crate) fn send_packet(header: &VirtioVsockHdr, payload: &[u8]) -> Result<bool> {
    let Ok(cid) = u32::try_from(header.dst_cid) else {
        return Ok(false);
    };
    let backend = BACKENDS.lock().get(&cid).and_then(Weak::upgrade);
    let Some(backend) = backend else {
        return Ok(false);
    };
    let packet = Packet::new(*header, payload)?;
    let mut pending = backend.pending.lock();
    if !pending.push(packet) {
        if pending.is_active && payload.is_empty() {
            // The transport cannot retry every control operation. Fail the
            // endpoint asynchronously instead of silently losing a shutdown,
            // connection response or credit update.
            pending.needs_reset = true;
            pending.is_active = false;
            drop(pending);
            backend.shared.worker_pollee().notify(IoEvents::IN);
            return_errno_with_message!(Errno::ENOBUFS, "the vsock control queue is full");
        }
        return Ok(false);
    }
    drop(pending);
    backend.shared.worker_pollee().notify(IoEvents::IN);
    Ok(true)
}

pub(super) fn init_in_first_kthread(major: device_id::MajorId) {
    let id = DeviceId::new(major, MinorId::new(VHOST_VSOCK_MINOR));
    char::register(Arc::new(VhostVsockDevice { id })).unwrap();
}

pub(crate) struct VhostTxReservation {
    backend: Arc<Backend>,
    len: usize,
    committed: bool,
    generation: u64,
}

impl VhostTxReservation {
    pub(crate) fn send(mut self, header: &VirtioVsockHdr, payload: &[u8]) -> Result<bool> {
        if payload.len() != self.len {
            return_errno_with_message!(Errno::EINVAL, "the reserved vsock payload size changed");
        }
        let packet = Packet::new(*header, payload)?;
        let mut pending = self.backend.pending.lock();
        if !pending.is_active || pending.generation != self.generation {
            return Ok(false);
        }
        pending.push_reserved(packet);
        self.committed = true;
        drop(pending);
        self.backend.shared.worker_pollee().notify(IoEvents::IN);
        Ok(true)
    }
}

impl Drop for VhostTxReservation {
    fn drop(&mut self) {
        if !self.committed {
            self.backend.pending.lock().release(self.len);
            self.backend.shared.worker_pollee().notify(IoEvents::IN);
        }
    }
}

pub(crate) fn reserve_data_packet(cid: u32, len: usize) -> Result<Option<VhostTxReservation>> {
    if len > packet::MAX_PAYLOAD_LEN {
        return_errno_with_message!(Errno::EINVAL, "the vsock payload is too large");
    }
    let backend = BACKENDS.lock().get(&cid).and_then(Weak::upgrade);
    let Some(backend) = backend else {
        return_errno_with_message!(Errno::ENETUNREACH, "the guest CID is not registered");
    };
    let mut pending = backend.pending.lock();
    if !pending.is_active {
        return_errno_with_message!(Errno::ENETUNREACH, "the guest CID is stopped");
    }
    if !pending.reserve(len) {
        return Ok(None);
    }
    let generation = pending.generation;
    drop(pending);
    Ok(Some(VhostTxReservation {
        backend,
        len,
        committed: false,
        generation,
    }))
}

pub(crate) fn can_send_data(cid: u32) -> bool {
    let backend = BACKENDS.lock().get(&cid).and_then(Weak::upgrade);
    backend.is_some_and(|backend| backend.pending.lock().has_data_room())
}
