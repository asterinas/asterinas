// SPDX-License-Identifier: MPL-2.0

//! A vhost-vsock device backed by the common vhost split queues.
//!
//! Each open file owns its configuration, guest CID and worker. The socket
//! transport enqueues host packets without accessing the owner's memory.

use aster_virtio::device::socket::header::VirtioVsockHdr;
use device_id::{DeviceId, MinorId};

use super::vhost::{self, VhostDeviceConfig, VhostDeviceState};
use crate::{
    device::{Device, DeviceType, registry::char},
    events::{EventFile, EventFileFlags, IoEvents, KernelEventFile},
    fs::{
        devtmpfs::DevtmpfsNodeMeta,
        file::{PerOpenFileOps, StatusFlags},
        vfs::{inode::FileOps, path::Path},
    },
    net::socket::vsock::{self, VMADDR_CID_HOST},
    prelude::*,
    process::signal::{PollHandle, Pollable},
    thread::{Thread, kernel_thread::ThreadOptions},
    util::ioctl::{RawIoctl, dispatch_ioctl},
};

mod packet;
mod worker;

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

    // Reference: <https://github.com/torvalds/linux/blob/v6.18/include/uapi/linux/vhost.h>.
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
            control: Mutex::new(VhostVsockControl::new()),
        }))
    }
}

struct VhostVsockFile {
    control: Mutex<VhostVsockControl>,
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
        self.control.lock().ioctl(raw)
    }
}

struct VhostVsockControl {
    common: VhostDeviceState<NUM_QUEUES>,
    backend: Option<Arc<Backend>>,
    worker: Option<Arc<Thread>>,
}

impl VhostVsockControl {
    fn new() -> Self {
        Self {
            common: VhostDeviceState::new(VhostDeviceConfig {
                device_features: vhost::VIRTIO_F_VERSION_1 | vhost::VIRTIO_RING_F_INDIRECT_DESC,
                backend_features: 0,
                max_queue_size: 256,
            }),
            backend: None,
            worker: None,
        }
    }

    fn ioctl(&mut self, raw: RawIoctl) -> Result<i32> {
        use ioctl_defs::*;
        use vhost::ioctl_defs::*;

        dispatch_ioctl!(match raw {
            cmd @ SetGuestCid => {
                self.common.check_owner()?;
                self.set_guest_cid(cmd.read()?)?;
                Ok(0)
            }
            cmd @ SetRunning => {
                self.common.check_owner()?;
                match cmd.read()? {
                    0 => self.stop(),
                    1 => self.start()?,
                    _ => return_errno_with_message!(
                        Errno::EINVAL,
                        "invalid vhost-vsock running value"
                    ),
                }
                Ok(0)
            }
            ResetOwner => {
                self.common.check_owner()?;
                self.release_backend();
                self.common.reset_owner_after_quiesce();
                Ok(0)
            }
            SetFeatures | SetBackendFeatures | SetMemTable | SetVringNum | SetVringAddr
            | SetVringBase | SetVringKick | SetVringCall | SetVringErr => {
                self.common.check_owner()?;
                let was_running = self
                    .backend
                    .as_ref()
                    .is_some_and(|backend| backend.is_running());
                self.stop();
                let result = self.common.handle_ioctl(raw)?;
                if was_running {
                    self.start()?;
                }
                Ok(result)
            }
            GetVringBase => {
                self.common.check_owner()?;
                self.stop();
                self.common.handle_ioctl(raw)
            }
            _ => self.common.handle_ioctl(raw),
        })
    }

    fn set_guest_cid(&mut self, cid: u64) -> Result<()> {
        let cid = validate_guest_cid(cid)?;
        if self
            .backend
            .as_ref()
            .is_some_and(|backend| backend.cid == cid)
        {
            return Ok(());
        }

        let backend = Backend::new(cid)?;
        {
            let mut backends = BACKENDS.lock();
            if backends.get(&cid).and_then(Weak::upgrade).is_some() {
                return_errno_with_message!(Errno::EADDRINUSE, "the guest CID is already in use");
            }
            backends.insert(cid, Arc::downgrade(&backend));
        }
        self.release_backend();
        self.backend = Some(backend);
        Ok(())
    }

    fn start(&mut self) -> Result<()> {
        vsock::ensure_vhost_backend()?;
        if self
            .backend
            .as_ref()
            .is_some_and(|backend| backend.is_running())
        {
            return Ok(());
        }
        // Join a worker which may have stopped itself after a queue error.
        self.stop();
        let backend = self
            .backend
            .as_ref()
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "the guest CID is not configured"))?;
        let mut runtime = self.common.build_runtime()?;
        let kicks = [
            runtime.queue_mut(RX_QUEUE)?.kick_event(),
            runtime.queue_mut(TX_QUEUE)?.kick_event(),
        ];
        if kicks.iter().any(Option::is_none) {
            return_errno_with_message!(Errno::EINVAL, "vhost-vsock queues need kick eventfds");
        }
        backend.wake.consume();
        let mut pending = backend.pending.lock();
        if pending.failed {
            pending.discard();
            pending.generation = pending.generation.wrapping_add(1);
            drop(pending);
            vsock::reset_vhost_connections(backend.cid);
            pending = backend.pending.lock();
            pending.failed = false;
        }
        pending.is_active = true;
        pending.is_running = true;
        drop(pending);
        let vmar = runtime.vmar().clone();
        let backend = backend.clone();
        let cid = backend.cid;
        self.worker = Some(
            ThreadOptions::new(move || worker::run(runtime, backend, kicks.map(Option::unwrap)))
                .vmar(vmar)
                .spawn(),
        );
        vsock::notify_vhost_writable(cid);
        Ok(())
    }

    fn stop(&mut self) {
        if let Some(backend) = &self.backend {
            backend.pending.lock().is_running = false;
            backend.wake.signal();
        }
        if let Some(worker) = self.worker.take() {
            worker.join();
        }
    }

    fn release_backend(&mut self) {
        if let Some(backend) = &self.backend {
            let mut pending = backend.pending.lock();
            pending.is_active = false;
            pending.generation = pending.generation.wrapping_add(1);
        }
        self.stop();
        if let Some(backend) = self.backend.take() {
            backend.pending.lock().discard();
            vsock::reset_vhost_connections(backend.cid);
            BACKENDS.lock().remove(&backend.cid);
        }
    }
}

impl Drop for VhostVsockControl {
    fn drop(&mut self) {
        self.release_backend();
    }
}

struct Backend {
    cid: u32,
    pending: SpinLock<PendingPackets>,
    wake: Arc<KernelEventFile>,
}

impl Backend {
    fn new(cid: u32) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            cid,
            pending: SpinLock::new(PendingPackets::new()),
            wake: KernelEventFile::from_file(&EventFile::new(0, EventFileFlags::empty()))?,
        }))
    }

    fn is_active(&self) -> bool {
        self.pending.lock().is_active
    }

    fn is_running(&self) -> bool {
        self.pending.lock().is_running
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
            pending.failed = true;
            pending.is_active = false;
            pending.is_running = false;
            drop(pending);
            backend.wake.signal();
            return_errno_with_message!(Errno::ENOBUFS, "the vsock control queue is full");
        }
        return Ok(false);
    }
    drop(pending);
    backend.wake.signal();
    Ok(true)
}

pub(super) fn init_in_first_kthread() {
    let id = DeviceId::new(
        super::MISC_MAJOR.get().unwrap().get(),
        MinorId::new(VHOST_VSOCK_MINOR),
    );
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
        self.backend.wake.signal();
        Ok(true)
    }
}

impl Drop for VhostTxReservation {
    fn drop(&mut self) {
        if !self.committed {
            self.backend.pending.lock().release(self.len);
            self.backend.wake.signal();
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
