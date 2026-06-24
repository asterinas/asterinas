// SPDX-License-Identifier: MPL-2.0

//! Minimal `/dev/vhost-vsock` ABI surface.
//!
//! Kata's QEMU backend requires the Linux vhost-vsock misc device at major
//! `10`, minor `241`.  This module registers that device and implements the
//! control-plane ioctls plus the minimal data-plane bridge needed to exchange
//! packets between QEMU's vhost virtqueues and Asterinas' vsock stack.

use core::{
    hint::spin_loop,
    sync::atomic::{AtomicBool, AtomicU16, Ordering},
    time::Duration,
};

use aster_virtio::device::socket::header::{VirtioVsockHdr as TransportVsockHdr, VirtioVsockOp};
use device_id::{DeviceId, MinorId};
use ostd::{mm::VmIo, task::Task};
use spin::Once;

use crate::{
    device::{Device, DeviceType, DevtmpfsInodeMeta, registry::char::register},
    events::IoEvents,
    fs::{
        file::{
            FileLike, PerOpenFileOps, StatusFlags,
            file_table::{FileDesc, RawFileDesc, get_file_fast},
        },
        vfs::inode::FileOps,
    },
    prelude::*,
    process::{
        Process,
        posix_thread::AsPosixThread,
        signal::{PollHandle, Pollable, Poller},
    },
    thread::kernel_thread::ThreadOptions,
    util::ioctl::{RawIoctl, dispatch_ioctl},
    vm::vmar::Vmar,
};

const VHOST_VSOCK_MINOR: u32 = 241;
const VIRTIO_F_VERSION_1: u64 = 1 << 32;
const HOST_CID: u64 = 2;
const KATA_AGENT_SERVER_PORT: u32 = 1024;
const VIRTQ_DESC_F_NEXT: u16 = 1;
const VIRTQ_DESC_F_INDIRECT: u16 = 4;
const VHOST_VSOCK_MAX_TX_CHAIN_BYTES: usize = 1024 * 1024;

static ACTIVE_BACKEND: Once<Arc<Mutex<Option<Arc<VhostVsockBackend>>>>> = Once::new();

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
    /// QEMU process that opened this device, captured at `SET_OWNER`.
    /// Lets later worker code read QEMU's userspace via `Vmar::read_alien`.
    owner_process: Option<Arc<Process>>,
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
    /// Set when a worker has been spawned for this device.
    worker_started: bool,
}

impl VhostVsockState {
    fn snapshot_for_worker(&self, stop: Arc<AtomicBool>) -> Option<WorkerInputs> {
        let owner = self.owner_process.clone()?;
        let owner_vmar = self.owner_vmar.clone()?;
        Some(WorkerInputs {
            owner_process: owner,
            owner_vmar,
            guest_cid: self.guest_cid,
            mem_regions: self.memory_regions.clone(),
            vring_num: self.vring_num,
            vring_addr: self.vring_addr,
            vring_kick: self.vring_kick.clone(),
            vring_call: self.vring_call.clone(),
            stop,
        })
    }
}

#[derive(Clone)]
struct WorkerInputs {
    owner_process: Arc<Process>,
    owner_vmar: Arc<Vmar>,
    guest_cid: Option<u64>,
    mem_regions: Vec<VhostMemoryRegion>,
    vring_num: [u32; VHOST_VSOCK_VRING_COUNT],
    vring_addr: [Option<VhostVringAddr>; VHOST_VSOCK_VRING_COUNT],
    vring_kick: [Option<Arc<dyn FileLike>>; VHOST_VSOCK_VRING_COUNT],
    vring_call: [Option<Arc<dyn FileLike>>; VHOST_VSOCK_VRING_COUNT],
    stop: Arc<AtomicBool>,
}

/// State machine for an outbound (host → guest) AF_VSOCK connect.
///
/// Only the Kata agent server connect needs a retry window. The optional
/// agent log forwarder uses a separate port and should fail quickly if the
/// guest-side log listener is not ready.
///
/// Transitions:
/// - `Idle` → `Connecting(info)`  via `start_connect()`        (host queues OP_REQUEST)
/// - `Connecting(info)` → `Idle`  via `complete_connect(event)` when guest replies OP_RESPONSE
///
/// While in `Connecting(info)`:
/// - `retry_connect()` re-injects OP_REQUEST for the agent server until the
///   response arrives.
/// - early OP_RST from guest for that 4-tuple is suppressed (guest hasn't
///   bound the listener yet — RST would tear down the local pending socket).
enum ConnectState {
    Idle,
    Connecting(VhostConnectInfo),
}

#[derive(Clone)]
struct VhostConnectInfo {
    guest_cid: u64,
    guest_port: u32,
    host_port: u32,
    buf_alloc: u32,
    fwd_cnt: u32,
}

struct VhostVsockBackend {
    inputs: WorkerInputs,
    connect_state: Mutex<ConnectState>,
    rx_last_avail: AtomicU16,
    rx_inject_busy: AtomicBool,
    tx_last_avail: AtomicU16,
}

impl VhostVsockBackend {
    fn new(inputs: WorkerInputs) -> Self {
        Self {
            inputs,
            connect_state: Mutex::new(ConnectState::Idle),
            rx_last_avail: AtomicU16::new(0),
            rx_inject_busy: AtomicBool::new(false),
            tx_last_avail: AtomicU16::new(0),
        }
    }

    fn inject(&self, packet: VhostVsockPacket<'_>) -> Result<()> {
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
        process_tx(self, &mut tx_last_avail)?;
        self.tx_last_avail.store(tx_last_avail, Ordering::Relaxed);
        Ok(())
    }

    /// Start tracking an outbound connect. Replaces any previous in-flight
    /// connect (the AF_VSOCK socket layer enforces sequencing, so this only
    /// fires once the previous connect has completed or timed out).
    fn start_connect(&self, info: VhostConnectInfo) {
        *self.connect_state.lock() = ConnectState::Connecting(info);
    }

    /// Re-inject OP_REQUEST while we're still in `Connecting`. No-op if the
    /// connect has already been completed or no connect is in flight.
    fn retry_connect(&self) {
        let info = match &*self.connect_state.lock() {
            ConnectState::Connecting(info) => info.clone(),
            ConnectState::Idle => return,
        };
        if info.guest_port != KATA_AGENT_SERVER_PORT {
            return;
        }
        if let Err(e) = self.inject_connect_request(&info) {
            error!("vhost-vsock pending connect retry failed: {:?}", e);
        }
    }

    fn inject_connect_request(&self, info: &VhostConnectInfo) -> Result<()> {
        self.inject(VhostVsockPacket {
            dst_port: info.guest_port,
            src_port: info.host_port,
            op: VIRTIO_VSOCK_OP_REQUEST,
            flags: 0,
            payload: &[],
            buf_alloc: info.buf_alloc,
            fwd_cnt: info.fwd_cnt,
        })
    }

    /// Returns true if the TX header is an early OP_RST for the in-flight
    /// connect's 4-tuple (guest hasn't bound the listener yet). Such RSTs
    /// would tear down our local pending socket prematurely; the AF_VSOCK
    /// `connect()` retries the OP_REQUEST until OP_RESPONSE arrives.
    fn should_suppress_tx_event(&self, hdr: &VirtioVsockHdr) -> bool {
        let state = self.connect_state.lock();
        let ConnectState::Connecting(info) = &*state else {
            return false;
        };
        if !matches_4tuple(hdr, info) {
            return false;
        }
        if hdr.op == VIRTIO_VSOCK_OP_RST && info.guest_port == KATA_AGENT_SERVER_PORT {
            debug!(
                "vhost-vsock suppress early RST for pending connect {}:{} -> {}:{}",
                HOST_CID, info.host_port, info.guest_cid, info.guest_port
            );
            return true;
        }
        false
    }

    /// Mark the in-flight connect as completed if the event matches its
    /// 4-tuple. Transitions `Connecting → Idle`.
    fn complete_connect(&self, hdr: &VirtioVsockHdr) {
        let mut state = self.connect_state.lock();
        let matches = match &*state {
            ConnectState::Connecting(info) => {
                hdr.src_cid == info.guest_cid
                    && hdr.src_port == info.guest_port
                    && hdr.dst_cid == HOST_CID
                    && hdr.dst_port == info.host_port
            }
            ConnectState::Idle => false,
        };
        if matches {
            *state = ConnectState::Idle;
        }
    }
}

fn matches_4tuple(hdr: &VirtioVsockHdr, info: &VhostConnectInfo) -> bool {
    hdr.src_cid == info.guest_cid
        && hdr.src_port == info.guest_port
        && hdr.dst_cid == HOST_CID
        && hdr.dst_port == info.host_port
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

struct VhostVsockFile {
    state: Mutex<VhostVsockState>,
    worker_stop: Arc<AtomicBool>,
}

impl VhostVsockFile {
    fn new() -> Self {
        Self {
            state: Mutex::new(VhostVsockState::default()),
            worker_stop: Arc::new(AtomicBool::new(false)),
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

    /// Logs a snapshot of each captured vring's descriptor head, avail ring
    /// header and used ring header.
    ///
    /// This is purely diagnostic: it does NOT process descriptors or pretend
    /// vhost-vsock is implemented. The intent is to verify, on the next probe
    /// run, that we can in fact reach QEMU's vring memory using the
    /// userspace addresses captured in `VHOST_SET_VRING_ADDR`.
    fn dump_vring_snapshot(state: &VhostVsockState) {
        let Some(current) = Task::current() else {
            error!("vhost-vsock dump_vring_snapshot: no current task");
            return;
        };
        let Some(thread_local) = current.as_thread_local() else {
            error!("vhost-vsock dump_vring_snapshot: no thread local");
            return;
        };
        let userspace = CurrentUserSpace::new(thread_local);

        for index in 0..VHOST_VSOCK_VRING_COUNT {
            let Some(addr) = state.vring_addr[index] else {
                continue;
            };
            let num = state.vring_num[index];
            let label = if index == 0 { "rx" } else { "tx" };

            // First 4 descriptors of the descriptor table.
            let mut descs: [VirtqDesc; 4] = Default::default();
            for slot in 0..descs.len() {
                let off = (slot * size_of::<VirtqDesc>()) as u64;
                let va = (addr.desc_user_addr + off) as usize;
                match userspace.read_val::<VirtqDesc>(va) {
                    Ok(d) => descs[slot] = d,
                    Err(e) => {
                        debug!(
                            "vhost-vsock {} desc[{}] read at {:#x} failed: {:?}",
                            label, slot, va, e
                        );
                        break;
                    }
                }
            }

            let avail_hdr: VirtqAvailHeader = userspace
                .read_val::<VirtqAvailHeader>(addr.avail_user_addr as usize)
                .unwrap_or_default();
            let used_hdr: VirtqUsedHeader = userspace
                .read_val::<VirtqUsedHeader>(addr.used_user_addr as usize)
                .unwrap_or_default();

            debug!(
                "vhost-vsock vring[{}] {}: num={}, desc[0..4]={:?}, avail={:?}, used={:?}",
                index, label, num, descs, avail_hdr, used_hdr
            );

            // Cross-check: read desc[0] using `Vmar::read_alien` (the same
            // primitive a future kernel-thread worker would have to use).
            if let Some(process) = state.owner_process.as_ref() {
                let vmar_guard = process.lock_vmar();
                if let Some(vmar) = vmar_guard.as_ref() {
                    let mut desc0_alien = VirtqDesc::default();
                    let mut writer = VmWriter::from(desc0_alien.as_mut_bytes()).to_fallible();
                    let result = vmar.read_alien(addr.desc_user_addr as usize, &mut writer);
                    debug!(
                        "vhost-vsock vring[{}] {}: alien_read_desc0={:?}, result={:?}",
                        index, label, desc0_alien, result
                    );
                }
            }
        }
    }

    /// Starts the data-plane worker once. The worker observes the guest's
    /// virtqueue activity via cross-process `Vmar::read_alien` reads.
    /// Returns `Ok(())` if the worker was started or is already running.
    fn ensure_worker_started(&self) -> Result<()> {
        let mut state = self.state.lock();
        if state.worker_started {
            return Ok(());
        }
        let Some(inputs) = state.snapshot_for_worker(self.worker_stop.clone()) else {
            return_errno_with_message!(Errno::EINVAL, "vhost-vsock owner process not captured yet");
        };
        let backend = Arc::new(VhostVsockBackend::new(inputs));
        *active_backend_slot().lock() = Some(backend.clone());
        ThreadOptions::new(move || worker_loop(backend)).spawn();
        state.worker_started = true;
        Ok(())
    }

    /// Captures the calling task's process. We rely on `SET_OWNER` running
    /// in QEMU's own thread, so the caller's process is the QEMU instance.
    fn capture_caller_owner() -> Option<(Arc<Process>, Arc<Vmar>)> {
        let task = Task::current()?;
        let posix_thread = task.as_posix_thread()?;
        let thread_local = task.as_thread_local()?;
        let vmar = thread_local.vmar().borrow().as_ref()?.clone_arc();
        Some((posix_thread.process(), vmar))
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
        Ok(Some(get_file_fast!(&mut file_table, fd).into_owned()))
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
                cmd.write(&VIRTIO_F_VERSION_1)?;
                Ok(0)
            }
            cmd @ SetFeatures => {
                self.state.lock().features = cmd.read()?;
                Ok(0)
            }
            SetOwner => {
                let mut state = self.state.lock();
                if state.owner_set {
                    return_errno_with_message!(Errno::EBUSY, "vhost-vsock owner is already set");
                }
                state.owner_set = true;
                if let Some((process, vmar)) = Self::capture_caller_owner() {
                    state.owner_process = Some(process);
                    state.owner_vmar = Some(vmar);
                }
                Ok(0)
            }
            ResetOwner => {
                *self.state.lock() = VhostVsockState::default();
                clear_active_backend();
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
                let memory_regions = Self::read_memory_regions(raw_ioctl, mem)?;
                debug!(
                    "vhost-vsock VHOST_SET_MEM_TABLE: nregions={}, regions={:?}",
                    mem.nregions, memory_regions
                );
                self.state.lock().memory_regions = memory_regions;
                Ok(0)
            }
            cmd @ SetVringNum => {
                let vring_state = cmd.read()?;
                let index = Self::check_vring_index(vring_state.index)?;
                self.state.lock().vring_num[index] = vring_state.num;
                Ok(0)
            }
            cmd @ SetVringAddr => {
                let vring_addr = cmd.read()?;
                let index = Self::check_vring_index(vring_addr.index)?;
                self.state.lock().vring_addr[index] = Some(vring_addr);
                debug!(
                    "vhost-vsock VHOST_SET_VRING_ADDR: index={}, addr={:?}",
                    index, vring_addr
                );
                Ok(0)
            }
            cmd @ SetVringBase => {
                let vring_state = cmd.read()?;
                let index = Self::check_vring_index(vring_state.index)?;
                self.state.lock().vring_base[index] = vring_state.num;
                Ok(0)
            }
            cmd @ GetVringBase => {
                let mut vring_state = cmd.read()?;
                let index = Self::check_vring_index(vring_state.index)?;
                vring_state.num = self.state.lock().vring_base[index];
                cmd.write(&vring_state)?;
                Ok(0)
            }
            cmd @ SetVringKick => {
                let vring_file = cmd.read()?;
                let index = Self::check_vring_index(vring_file.index)?;
                self.state.lock().vring_kick[index] = Self::get_event_file(vring_file.fd)?;
                debug!(
                    "vhost-vsock VHOST_SET_VRING_KICK: index={}, fd={}",
                    index, vring_file.fd
                );
                Ok(0)
            }
            cmd @ SetVringCall => {
                let vring_file = cmd.read()?;
                let index = Self::check_vring_index(vring_file.index)?;
                self.state.lock().vring_call[index] = Self::get_event_file(vring_file.fd)?;
                debug!(
                    "vhost-vsock VHOST_SET_VRING_CALL: index={}, fd={}",
                    index, vring_file.fd
                );
                Ok(0)
            }
            cmd @ SetVringErr => {
                let vring_file = cmd.read()?;
                let index = Self::check_vring_index(vring_file.index)?;
                self.state.lock().vring_err[index] = Self::get_event_file(vring_file.fd)?;
                debug!(
                    "vhost-vsock VHOST_SET_VRING_ERR: index={}, fd={}",
                    index, vring_file.fd
                );
                Ok(0)
            }
            cmd @ SetBackendFeatures => {
                self.state.lock().backend_features = cmd.read()?;
                Ok(0)
            }
            cmd @ GetBackendFeatures => {
                cmd.write(&self.state.lock().backend_features)?;
                Ok(0)
            }
            cmd @ SetGuestCid => {
                let guest_cid = cmd.read()?;
                if guest_cid <= 2 {
                    return_errno_with_message!(Errno::EINVAL, "the guest CID is reserved");
                }
                self.state.lock().guest_cid = Some(guest_cid);
                Ok(0)
            }
            cmd @ SetRunning => {
                let running = cmd.read()?;
                let state = self.state.lock();
                debug!(
                    "vhost-vsock VHOST_VSOCK_SET_RUNNING: \
                     running={}, guest_cid={:?}, features={:#x}, backend_features={:#x}, \
                     memory_regions={:?}, vring_num={:?}, vring_base={:?}, vring_addr={:?}",
                    running,
                    state.guest_cid,
                    state.features,
                    state.backend_features,
                    state.memory_regions,
                    state.vring_num,
                    state.vring_base,
                    state.vring_addr
                );

                // Diagnostic: prove we can read QEMU's userspace virtqueue memory.
                // We are still in QEMU's ioctl thread context here, so
                // `CurrentUserSpace` resolves to QEMU's address space.
                if running != 0 {
                    Self::dump_vring_snapshot(&state);
                }
                drop(state);

                if running != 0 {
                    self.ensure_worker_started()?;
                    Ok(0)
                } else {
                    // Best effort stop: signal worker (if any) to exit.
                    self.worker_stop.store(true, Ordering::Relaxed);
                    Ok(0)
                }
            }
            _ => return_errno_with_message!(Errno::ENOTTY, "the ioctl command is unknown"),
        })
    }
}

impl Drop for VhostVsockFile {
    fn drop(&mut self) {
        // Tell the worker to exit on its next polling cycle.
        self.worker_stop.store(true, Ordering::Relaxed);
        clear_active_backend();
    }
}

fn active_backend_slot() -> &'static Arc<Mutex<Option<Arc<VhostVsockBackend>>>> {
    ACTIVE_BACKEND.call_once(|| Arc::new(Mutex::new(None)))
}

fn active_backend() -> Option<Arc<VhostVsockBackend>> {
    active_backend_slot().lock().clone()
}

fn clear_active_backend() {
    *active_backend_slot().lock() = None;
}

/// The per-device data-plane worker.
///
/// The worker drains guest TX descriptors, injects host-to-guest packets into
/// the RX ring, and keeps periodic ring snapshots for bring-up diagnostics.
fn worker_loop(backend: Arc<VhostVsockBackend>) {
    let inputs = &backend.inputs;
    info!(
        "vhost-vsock worker started: guest_cid={:?}, vring_num={:?}",
        inputs.guest_cid, inputs.vring_num
    );

    // Initial dump of both rings so we have a known baseline.
    dump_ring(&inputs, 0, "rx", "init");
    dump_ring(&inputs, 1, "tx", "init");

    // Main event loop: wait on the TX kick eventfd. Whenever the guest
    // pushes anything onto vring[1], it writes to this eventfd and we wake.
    let kick_tx = inputs.vring_kick[1].clone();
    let kick_rx = inputs.vring_kick[0].clone();

    let timeout = Duration::from_millis(100);
    let mut tick: u64 = 0;
    while !inputs.stop.load(Ordering::Relaxed) {
        let mut poller = Poller::new(Some(&timeout));
        let mut tx_ready = false;
        let mut rx_ready = false;

        if let Some(k) = kick_tx.as_ref() {
            tx_ready = !k
                .poll(IoEvents::IN, Some(poller.as_handle_mut()))
                .is_empty();
        }
        if let Some(k) = kick_rx.as_ref() {
            rx_ready = !k
                .poll(IoEvents::IN, Some(poller.as_handle_mut()))
                .is_empty();
        }

        if !tx_ready && !rx_ready {
            // Block until any registered eventfd fires or timeout elapses.
            // ETIME is fine; we'll re-check the stop flag and loop.
            let _ = poller.wait();
        }

        // Drain the eventfd counters so subsequent poll() reflects new kicks.
        if let Some(k) = kick_tx.as_ref() {
            consume_eventfd(k.as_ref());
        }
        if let Some(k) = kick_rx.as_ref() {
            consume_eventfd(k.as_ref());
        }

        // Drain TX every loop as well as on explicit kicks. This makes the
        // debug path resilient to missed eventfd readiness while the runtime
        // may tear QEMU down quickly on connect failure.
        if let Err(e) = backend.process_tx() {
            if tx_ready {
                error!("vhost-vsock TX process failed: {:?}", e);
            }
        }
        if rx_ready {
            dump_ring(&inputs, 0, "rx", "kick");
        }

        tick += 1;
        if tick % 20 == 0 {
            // Periodic snapshot even if no kicks fired, so we can see slow
            // changes (e.g. guest filling RX buffers in the background).
            dump_ring(&inputs, 0, "rx", "tick");
            dump_ring(&inputs, 1, "tx", "tick");
        }
        if tick % 10 == 0 {
            backend.retry_connect();
        }
    }

    // Final state snapshot so we can see what the guest left behind even if
    // QEMU torn down the device before any TX kick reached our poll loop.
    dump_ring(&inputs, 0, "rx", "exit");
    dump_ring(&inputs, 1, "tx", "exit");
    if let Err(e) = backend.process_tx() {
        debug!("vhost-vsock final TX drain skipped after teardown: {:?}", e);
    }
    info!("vhost-vsock worker exiting after {} ticks", tick);
}

fn dump_ring(inputs: &WorkerInputs, index: usize, label: &str, why: &str) {
    let Some(addr) = inputs.vring_addr[index] else {
        return;
    };
    let vmar = inputs.owner_vmar.as_ref();
    let mut avail = VirtqAvailHeader::default();
    let mut writer = VmWriter::from(avail.as_mut_bytes()).to_fallible();
    let _ = vmar.read_alien(addr.avail_user_addr as usize, &mut writer);
    let mut used = VirtqUsedHeader::default();
    let mut writer = VmWriter::from(used.as_mut_bytes()).to_fallible();
    let _ = vmar.read_alien(addr.used_user_addr as usize, &mut writer);
    debug!(
        "vhost-vsock vring[{}] {} {}: avail={:?}, used={:?}",
        index, label, why, avail, used
    );
}

/// Translate a guest physical address to QEMU's userspace address using the
/// mem table from `VHOST_SET_MEM_TABLE`.
fn gpa_to_uva(regions: &[VhostMemoryRegion], gpa: u64, len: usize) -> Option<usize> {
    let end = gpa.checked_add(len as u64)?;
    for r in regions {
        let region_end = r.guest_phys_addr.checked_add(r.memory_size)?;
        if gpa >= r.guest_phys_addr && end <= region_end {
            let offset = gpa - r.guest_phys_addr;
            return Some((r.userspace_addr + offset) as usize);
        }
    }
    None
}

/// Synthesizes a single host->guest virtio-vsock packet on the RX queue
/// (`vring[0]`). This is a stand-in for a future AF_VSOCK socket bridge:
/// build the header, find an avail-ring head, write it into the head's
/// descriptor buffer, publish a used-ring entry, and signal the call eventfd.
fn inject_packet(
    inputs: &WorkerInputs,
    last_avail: &mut u16,
    packet: VhostVsockPacket<'_>,
) -> Result<()> {
    const RX_RING: usize = 0;

    let addr = inputs.vring_addr[RX_RING]
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "vhost-vsock RX vring addr not set"))?;
    let num = inputs.vring_num[RX_RING] as usize;
    if num == 0 {
        return_errno_with_message!(Errno::EINVAL, "vhost-vsock RX vring num is zero");
    }
    let guest_cid = inputs
        .guest_cid
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "guest CID not set"))?;
    let call = inputs.vring_call[RX_RING]
        .clone()
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "RX call eventfd not set"))?;

    let vmar = inputs.owner_vmar.as_ref();

    // 1) read avail header (flags + idx)
    let mut avail = VirtqAvailHeader::default();
    let mut writer = VmWriter::from(avail.as_mut_bytes()).to_fallible();
    vmar.read_alien(addr.avail_user_addr as usize, &mut writer)
        .map_err(|(e, _)| e)?;

    if *last_avail == avail.idx {
        return_errno_with_message!(
            Errno::EAGAIN,
            "vhost-vsock RX has no buffer published by guest yet"
        );
    }

    // 2) read the next available RX descriptor head.
    let avail_slot = *last_avail as usize % num;
    let mut head_le: u16 = 0;
    let mut writer = VmWriter::from(head_le.as_mut_bytes()).to_fallible();
    vmar.read_alien(
        addr.avail_user_addr as usize + 4 + avail_slot * 2,
        &mut writer,
    )
    .map_err(|(e, _)| e)?;
    let head = head_le as usize;
    if head >= num {
        return_errno_with_message!(Errno::EINVAL, "vhost-vsock head index out of range");
    }

    // 3) read desc[head]
    let mut desc = VirtqDesc::default();
    let mut writer = VmWriter::from(desc.as_mut_bytes()).to_fallible();
    vmar.read_alien(
        addr.desc_user_addr as usize + head * size_of::<VirtqDesc>(),
        &mut writer,
    )
    .map_err(|(e, _)| e)?;

    let packet_len = VIRTIO_VSOCK_HDR_SIZE
        .checked_add(packet.payload.len())
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "vhost-vsock packet too large"))?;
    if (desc.len as usize) < packet_len {
        error!(
            "vhost-vsock RX buffer too small op={}({}) payload_len={} desc_len={} packet_len={}",
            packet.op,
            vsock_op_name(packet.op),
            packet.payload.len(),
            desc.len,
            packet_len
        );
        return_errno_with_message!(Errno::EINVAL, "vhost-vsock RX buffer too small");
    }

    // 4) build packet header and write into the descriptor buffer
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
    let buf_uva = gpa_to_uva(&inputs.mem_regions, desc.addr, packet_len).ok_or_else(|| {
        Error::with_message(
            Errno::EFAULT,
            "vhost-vsock RX desc.addr not covered by mem table",
        )
    })?;
    let bytes = hdr.to_bytes();
    let mut reader = VmReader::from(bytes.as_slice()).to_fallible();
    vmar.write_alien(buf_uva, &mut reader).map_err(|(e, _)| e)?;
    if !packet.payload.is_empty() {
        let mut reader = VmReader::from(packet.payload).to_fallible();
        vmar.write_alien(buf_uva + VIRTIO_VSOCK_HDR_SIZE, &mut reader)
            .map_err(|(e, _)| e)?;
    }
    if hdr.op == VIRTIO_VSOCK_OP_RW {
        debug!(
            "vhost-vsock RX inject RW head={} desc_len={} payload_len={} src={}:{} dst={}:{} buf_alloc={} fwd_cnt={}",
            head,
            desc.len,
            packet.payload.len(),
            hdr.src_cid,
            hdr.src_port,
            hdr.dst_cid,
            hdr.dst_port,
            hdr.buf_alloc,
            hdr.fwd_cnt
        );
    } else {
        debug!(
            "vhost-vsock inject: head={}, desc.addr={:#x} -> uva={:#x}, desc.len={}, \
             op={}({}) payload_len={}, hdr={:?}",
            head,
            desc.addr,
            buf_uva,
            desc.len,
            hdr.op,
            vsock_op_name(hdr.op),
            packet.payload.len(),
            hdr
        );
    }

    // 5) read current used.idx, write used.ring[idx] = {id: head, len: 44}
    let mut used = VirtqUsedHeader::default();
    let mut writer = VmWriter::from(used.as_mut_bytes()).to_fallible();
    vmar.read_alien(addr.used_user_addr as usize, &mut writer)
        .map_err(|(e, _)| e)?;
    let used_slot = used.idx as usize % num;
    let used_elem = VirtqUsedElem {
        id: head as u32,
        len: packet_len as u32,
    };
    let mut reader = VmReader::from(used_elem.as_bytes()).to_fallible();
    vmar.write_alien(
        addr.used_user_addr as usize + 4 + used_slot * size_of::<VirtqUsedElem>(),
        &mut reader,
    )
    .map_err(|(e, _)| e)?;

    // 6) increment used.idx
    used.idx = used.idx.wrapping_add(1);
    let mut reader = VmReader::from(used.as_bytes()).to_fallible();
    vmar.write_alien(addr.used_user_addr as usize, &mut reader)
        .map_err(|(e, _)| e)?;

    *last_avail = last_avail.wrapping_add(1);

    // 7) signal call eventfd: write 1u64 LE
    let counter: u64 = 1;
    let mut reader = VmReader::from(counter.as_bytes()).to_fallible();
    call.write(&mut reader)?;

    Ok(())
}

/// Drain new entries off the TX (`vring[1]`) avail ring. For each new head,
/// read the descriptor's buffer (44-byte vsock header + optional payload),
/// log the parsed header, then publish the descriptor on the used ring and
/// signal the call eventfd so the guest can recycle it.
fn process_tx(backend: &VhostVsockBackend, last_avail: &mut u16) -> Result<()> {
    const TX_RING: usize = 1;
    let inputs = &backend.inputs;
    let addr = inputs.vring_addr[TX_RING]
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "vhost-vsock TX vring addr not set"))?;
    let num = inputs.vring_num[TX_RING] as usize;
    if num == 0 {
        return_errno_with_message!(Errno::EINVAL, "vhost-vsock TX vring num is zero");
    }
    let call = inputs.vring_call[TX_RING].clone();

    let vmar = inputs.owner_vmar.as_ref();

    let mut avail_hdr = VirtqAvailHeader::default();
    let mut writer = VmWriter::from(avail_hdr.as_mut_bytes()).to_fallible();
    vmar.read_alien(addr.avail_user_addr as usize, &mut writer)
        .map_err(|(e, _)| e)?;

    let mut consumed_any = false;
    while *last_avail != avail_hdr.idx {
        let slot = *last_avail as usize % num;
        let mut head_le: u16 = 0;
        let mut writer = VmWriter::from(head_le.as_mut_bytes()).to_fallible();
        vmar.read_alien(addr.avail_user_addr as usize + 4 + slot * 2, &mut writer)
            .map_err(|(e, _)| e)?;
        let head = head_le as usize;
        if head >= num {
            return_errno_with_message!(Errno::EINVAL, "vhost-vsock TX head out of range");
        }

        let chain = read_tx_chain(vmar, addr, &inputs.mem_regions, num, head)?;
        if chain.bytes.len() >= VIRTIO_VSOCK_HDR_SIZE {
            let hdr = VirtioVsockHdr::from_bytes(&chain.bytes[..VIRTIO_VSOCK_HDR_SIZE]);
            let payload_len = hdr.len as usize;
            let packet_len = VIRTIO_VSOCK_HDR_SIZE
                .checked_add(payload_len)
                .ok_or_else(|| {
                    Error::with_message(Errno::EINVAL, "vhost-vsock TX packet too large")
                })?;
            if chain.bytes.len() < packet_len {
                warn!(
                    "vhost-vsock TX incomplete packet head={} chain_len={} packet_len={} first_flags={:#x}",
                    head,
                    chain.bytes.len(),
                    packet_len,
                    chain.first_desc.flags
                );
                publish_tx_used(vmar, &addr, num, head, chain.total_len)?;
                *last_avail = last_avail.wrapping_add(1);
                consumed_any = true;
                continue;
            }
            let payload = chain.bytes[VIRTIO_VSOCK_HDR_SIZE..packet_len].to_vec();
            if hdr.op == VIRTIO_VSOCK_OP_RW {
                debug!(
                    "vhost-vsock TX RW head={} chain_len={} src={}:{} dst={}:{} payload_len={} buf_alloc={} fwd_cnt={}",
                    head,
                    chain.bytes.len(),
                    hdr.src_cid,
                    hdr.src_port,
                    hdr.dst_cid,
                    hdr.dst_port,
                    hdr.len,
                    hdr.buf_alloc,
                    hdr.fwd_cnt
                );
            } else {
                debug!(
                    "vhost-vsock TX packet head={} desc.addr={:#x} desc.len={} flags={:#x} chain_len={} \
                     op={}({}) type={} src={}:{} dst={}:{} payload_len={} buf_alloc={} fwd_cnt={}",
                    head,
                    chain.first_desc.addr,
                    chain.first_desc.len,
                    chain.first_desc.flags,
                    chain.bytes.len(),
                    hdr.op,
                    vsock_op_name(hdr.op),
                    hdr.type_,
                    hdr.src_cid,
                    hdr.src_port,
                    hdr.dst_cid,
                    hdr.dst_port,
                    hdr.len,
                    hdr.buf_alloc,
                    hdr.fwd_cnt,
                );
            }
            if !backend.should_suppress_tx_event(&hdr) {
                if let Err(e) = deliver_tx_packet(hdr, &payload) {
                    error!("vhost-vsock deliver TX packet failed: {:?}", e);
                    return Err(e);
                }
            }
            publish_tx_used(vmar, &addr, num, head, chain.total_len)?;
            *last_avail = last_avail.wrapping_add(1);
            consumed_any = true;
            continue;
        } else {
            warn!(
                "vhost-vsock TX short chain head={} len={} first_flags={:#x} (skipping parse)",
                head,
                chain.bytes.len(),
                chain.first_desc.flags
            );
        }

        publish_tx_used(vmar, &addr, num, head, chain.total_len)?;

        *last_avail = last_avail.wrapping_add(1);
        consumed_any = true;
    }

    if consumed_any {
        if let Some(call) = call {
            let counter: u64 = 1;
            let mut reader = VmReader::from(counter.as_bytes()).to_fallible();
            let _ = call.write(&mut reader);
        }
    }

    Ok(())
}

/// Best-effort drain of an eventfd counter (8 bytes). Errors are intentionally
/// ignored: an empty eventfd just yields `EAGAIN` and that's fine.
fn consume_eventfd(file: &dyn FileLike) {
    let mut buf = [0u8; 8];
    let mut writer = VmWriter::from(buf.as_mut_slice()).to_fallible();
    let _ = file.read(&mut writer);
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

    let is_request = op == VirtioVsockOp::Request as u16;
    // This path may run while the vsock socket-table and connection-state
    // spinlocks are held, so it must not wait for the vhost backend here.
    let backend = active_backend();
    let Some(backend) = backend else {
        return Ok(false);
    };

    if src_cid != HOST_CID || Some(dst_cid) != backend.inputs.guest_cid {
        return Ok(false);
    }

    let connect_info = VhostConnectInfo {
        guest_cid: dst_cid,
        guest_port: dst_port,
        host_port: src_port,
        buf_alloc,
        fwd_cnt,
    };
    if is_request {
        backend.start_connect(connect_info);
    }
    backend.inject(VhostVsockPacket {
        dst_port,
        src_port,
        op,
        flags,
        payload,
        buf_alloc,
        fwd_cnt,
    })?;
    debug!(
        "vhost-vsock injected {}:{} -> {}:{} op={} payload_len={} buf_alloc={} fwd_cnt={}",
        HOST_CID,
        src_port,
        dst_cid,
        dst_port,
        op,
        payload.len(),
        buf_alloc,
        fwd_cnt
    );
    Ok(true)
}

struct TxChain {
    first_desc: VirtqDesc,
    bytes: Vec<u8>,
    total_len: u32,
}

fn read_tx_chain(
    vmar: &Vmar,
    addr: VhostVringAddr,
    mem_regions: &[VhostMemoryRegion],
    num: usize,
    head: usize,
) -> Result<TxChain> {
    let first_desc = read_vring_desc(vmar, addr.desc_user_addr as usize, num, head)?;
    if first_desc.flags & VIRTQ_DESC_F_INDIRECT != 0 {
        return read_indirect_tx_chain(vmar, mem_regions, first_desc);
    }

    read_direct_tx_chain(vmar, addr, mem_regions, num, first_desc)
}

fn read_direct_tx_chain(
    vmar: &Vmar,
    addr: VhostVringAddr,
    mem_regions: &[VhostMemoryRegion],
    num: usize,
    first_desc: VirtqDesc,
) -> Result<TxChain> {
    read_tx_desc_chain(vmar, mem_regions, first_desc, first_desc, num, |index| {
        read_vring_desc(vmar, addr.desc_user_addr as usize, num, index)
    })
}

fn read_indirect_tx_chain(
    vmar: &Vmar,
    mem_regions: &[VhostMemoryRegion],
    first_desc: VirtqDesc,
) -> Result<TxChain> {
    let table_len = first_desc.len as usize;
    if table_len == 0 || table_len % size_of::<VirtqDesc>() != 0 {
        return_errno_with_message!(
            Errno::EINVAL,
            "vhost-vsock TX indirect descriptor table has invalid length"
        );
    }
    let table_num = table_len / size_of::<VirtqDesc>();
    let table_uva = gpa_to_uva(mem_regions, first_desc.addr, table_len).ok_or_else(|| {
        Error::with_message(
            Errno::EFAULT,
            "vhost-vsock TX indirect table not covered by mem table",
        )
    })?;

    let first_indirect_desc = read_indirect_desc(vmar, table_uva, table_num, 0)?;
    read_tx_desc_chain(
        vmar,
        mem_regions,
        first_desc,
        first_indirect_desc,
        table_num,
        |index| read_indirect_desc(vmar, table_uva, table_num, index),
    )
}

fn read_tx_desc_chain(
    vmar: &Vmar,
    mem_regions: &[VhostMemoryRegion],
    reported_first_desc: VirtqDesc,
    start_desc: VirtqDesc,
    num: usize,
    mut read_desc: impl FnMut(usize) -> Result<VirtqDesc>,
) -> Result<TxChain> {
    let mut bytes = Vec::new();
    let mut total_len: u32 = 0;
    let mut desc = start_desc;

    for _ in 0..num {
        if desc.flags & VIRTQ_DESC_F_INDIRECT != 0 {
            return_errno_with_message!(
                Errno::EINVAL,
                "vhost-vsock TX nested indirect descriptor is unsupported"
            );
        }

        append_tx_desc_bytes(vmar, mem_regions, desc, &mut bytes)?;
        total_len = total_len.wrapping_add(desc.len);

        if desc.flags & VIRTQ_DESC_F_NEXT == 0 {
            return Ok(TxChain {
                first_desc: reported_first_desc,
                bytes,
                total_len,
            });
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
    desc_user_addr: usize,
    num: usize,
    index: usize,
) -> Result<VirtqDesc> {
    if index >= num {
        return_errno_with_message!(Errno::EINVAL, "vhost-vsock TX chain index out of range");
    }

    read_desc_at(vmar, desc_user_addr + index * size_of::<VirtqDesc>())
}

fn read_indirect_desc(
    vmar: &Vmar,
    table_uva: usize,
    table_num: usize,
    index: usize,
) -> Result<VirtqDesc> {
    if index >= table_num {
        return_errno_with_message!(
            Errno::EINVAL,
            "vhost-vsock TX indirect chain index out of range"
        );
    }

    read_desc_at(vmar, table_uva + index * size_of::<VirtqDesc>())
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
    let desc_uva = gpa_to_uva(mem_regions, desc.addr, desc_len).ok_or_else(|| {
        Error::with_message(
            Errno::EFAULT,
            "vhost-vsock TX desc.addr not covered by mem table",
        )
    })?;
    let old_len = bytes.len();
    bytes.resize(new_len, 0);
    let mut writer = VmWriter::from(&mut bytes[old_len..]).to_fallible();
    vmar.read_alien(desc_uva, &mut writer).map_err(|(e, _)| e)?;
    Ok(())
}

fn deliver_tx_packet(hdr: VirtioVsockHdr, payload: &[u8]) -> Result<()> {
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

    if hdr.op == VIRTIO_VSOCK_OP_RESPONSE
        && let Some(backend) = active_backend()
    {
        backend.complete_connect(&hdr);
    }

    Ok(())
}

fn publish_tx_used(
    vmar: &Vmar,
    addr: &VhostVringAddr,
    num: usize,
    head: usize,
    len: u32,
) -> Result<()> {
    let mut used = VirtqUsedHeader::default();
    let mut writer = VmWriter::from(used.as_mut_bytes()).to_fallible();
    vmar.read_alien(addr.used_user_addr as usize, &mut writer)
        .map_err(|(e, _)| e)?;
    let used_slot = used.idx as usize % num;
    let used_elem = VirtqUsedElem {
        id: head as u32,
        len,
    };
    let mut reader = VmReader::from(used_elem.as_bytes()).to_fallible();
    vmar.write_alien(
        addr.used_user_addr as usize + 4 + used_slot * size_of::<VirtqUsedElem>(),
        &mut reader,
    )
    .map_err(|(e, _)| e)?;
    used.idx = used.idx.wrapping_add(1);
    let mut reader = VmReader::from(used.as_bytes()).to_fallible();
    vmar.write_alien(addr.used_user_addr as usize, &mut reader)
        .map_err(|(e, _)| e)?;
    Ok(())
}

pub(super) fn init() -> Result<()> {
    register(VhostVsockDevice::new())
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

// virtio split-virtqueue layout structures, used only for diagnostic snapshot.

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
struct VirtqDesc {
    addr: u64,
    len: u32,
    flags: u16,
    next: u16,
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
/// We build/serialize this as a flat byte buffer to side-step `Pod`'s
/// no-padding requirement (the natural `repr(C)` layout has 4 bytes of
/// trailing padding for u64 alignment).
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
    fn to_bytes(&self) -> [u8; VIRTIO_VSOCK_HDR_SIZE] {
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
const VIRTIO_VSOCK_OP_REQUEST: u16 = 1;
const VIRTIO_VSOCK_OP_RESPONSE: u16 = 2;
const VIRTIO_VSOCK_OP_RST: u16 = 3;
const VIRTIO_VSOCK_OP_SHUTDOWN: u16 = 4;
const VIRTIO_VSOCK_OP_RW: u16 = 5;
const VIRTIO_VSOCK_OP_CREDIT_UPDATE: u16 = 6;
const VIRTIO_VSOCK_OP_CREDIT_REQUEST: u16 = 7;

fn vsock_op_name(op: u16) -> &'static str {
    match op {
        1 => "REQUEST",
        2 => "RESPONSE",
        3 => "RST",
        4 => "SHUTDOWN",
        5 => "RW",
        6 => "CREDIT_UPDATE",
        7 => "CREDIT_REQUEST",
        _ => "UNKNOWN",
    }
}

impl VirtioVsockHdr {
    fn from_bytes(b: &[u8]) -> Self {
        Self {
            src_cid: u64::from_le_bytes(b[0..8].try_into().unwrap()),
            dst_cid: u64::from_le_bytes(b[8..16].try_into().unwrap()),
            src_port: u32::from_le_bytes(b[16..20].try_into().unwrap()),
            dst_port: u32::from_le_bytes(b[20..24].try_into().unwrap()),
            len: u32::from_le_bytes(b[24..28].try_into().unwrap()),
            type_: u16::from_le_bytes(b[28..30].try_into().unwrap()),
            op: u16::from_le_bytes(b[30..32].try_into().unwrap()),
            flags: u32::from_le_bytes(b[32..36].try_into().unwrap()),
            buf_alloc: u32::from_le_bytes(b[36..40].try_into().unwrap()),
            fwd_cnt: u32::from_le_bytes(b[40..44].try_into().unwrap()),
        }
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
