// SPDX-License-Identifier: MPL-2.0

//! Persistent vhost configuration and exclusive access to split virtqueues.

#![short_vis_path::add(vhost)]

use core::{
    array,
    sync::atomic::{AtomicBool, Ordering},
};

use aster_util::field_ptr;
use ostd::{mm::VmIo, task::Task};

use super::{
    memory::{VhostMemory, VhostMemoryRegion, VhostMemorySpace},
    virtqueue::VhostVirtQueue,
    worker::{self, VhostWork},
};
use crate::{
    events::{IoEvents, KernelEventFile},
    fs::file::file_table::{FileDesc, RawFileDesc, get_file_fast},
    prelude::*,
    process::signal::Pollee,
    thread::{Thread, kernel_thread::ThreadOptions},
    util::ioctl::{RawIoctl, dispatch_ioctl},
    vm::vmar::Vmar,
};

#[cfg(ktest)]
mod tests;

/// `struct vhost_vring_state` in Linux, a queue index and its size or base.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/linux/vhost_types.h#L19>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(in vhost) struct VhostVringState {
    pub index: u32,
    pub num: u32,
}

/// `struct vhost_vring_file` in Linux, a queue index and its eventfd descriptor.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/linux/vhost_types.h#L24>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(in vhost) struct VhostVringFile {
    pub index: u32,
    pub fd: i32,
}

/// `struct vhost_vring_addr` in Linux, the owner virtual addresses of a queue.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/linux/vhost_types.h#L30>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(in vhost) struct VhostVringAddr {
    pub index: u32,
    pub flags: u32,
    pub desc_user_addr: u64,
    pub used_user_addr: u64,
    pub avail_user_addr: u64,
    pub log_guest_addr: u64,
}

pub(in vhost) mod ioctl_defs {
    use super::{VhostMemory, VhostVringAddr, VhostVringFile, VhostVringState};
    use crate::util::ioctl::{InData, InOutData, NoData, OutData, ioc};

    // Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/linux/vhost.h#L26>.
    pub(in vhost) type GetFeatures        = ioc!(VHOST_GET_FEATURES,         0xaf, 0x00, OutData<u64>);
    pub(in vhost) type SetFeatures        = ioc!(VHOST_SET_FEATURES,         0xaf, 0x00, InData<u64>);
    pub(in vhost) type SetOwner           = ioc!(VHOST_SET_OWNER,            0xaf, 0x01, NoData);
    pub(in vhost) type ResetOwner         = ioc!(VHOST_RESET_OWNER,          0xaf, 0x02, NoData);
    pub(in vhost) type SetMemTable        = ioc!(VHOST_SET_MEM_TABLE,        0xaf, 0x03, InData<VhostMemory>);
    // Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/linux/vhost.h#L71>.
    pub(in vhost) type SetVringNum        = ioc!(VHOST_SET_VRING_NUM,        0xaf, 0x10, InData<VhostVringState>);
    pub(in vhost) type SetVringAddr       = ioc!(VHOST_SET_VRING_ADDR,       0xaf, 0x11, InData<VhostVringAddr>);
    pub(in vhost) type SetVringBase       = ioc!(VHOST_SET_VRING_BASE,       0xaf, 0x12, InData<VhostVringState>);
    pub(in vhost) type GetVringBase       = ioc!(VHOST_GET_VRING_BASE,       0xaf, 0x12, InOutData<VhostVringState>);
    // Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/linux/vhost.h#L109>.
    pub(in vhost) type SetVringKick       = ioc!(VHOST_SET_VRING_KICK,       0xaf, 0x20, InData<VhostVringFile>);
    pub(in vhost) type SetVringCall       = ioc!(VHOST_SET_VRING_CALL,       0xaf, 0x21, InData<VhostVringFile>);
    pub(in vhost) type SetVringErr        = ioc!(VHOST_SET_VRING_ERR,        0xaf, 0x22, InData<VhostVringFile>);
    // Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/linux/vhost.h#L123>.
    pub(in vhost) type SetBackendFeatures = ioc!(VHOST_SET_BACKEND_FEATURES, 0xaf, 0x25, InData<u64>);
    pub(in vhost) type GetBackendFeatures = ioc!(VHOST_GET_BACKEND_FEATURES, 0xaf, 0x26, OutData<u64>);
}

/// Backend-specific feature masks and split-ring limits.
#[derive(Clone, Copy, Debug)]
pub(in vhost) struct VhostDeviceConfig {
    pub device_features: u64,
    pub backend_features: u64,
    pub max_queue_size: u32,
}

/// Owns one session's configuration and worker lifecycle.
///
/// The backend protects this session with a mutex to serialize complete ioctls
/// and close. Worker callbacks use shared data without acquiring that mutex.
/// Close must stop work even if callbacks retain the backend.
/// `NUM_QUEUES` excludes queues handled only by the frontend.
pub(in vhost) struct VhostSession<const NUM_QUEUES: usize> {
    config: VhostDeviceConfig,
    backend_features: u64,
    // FIXME: Support multiple workers per session for devices such as vhost-scsi.
    // Most vhost devices use one worker per session; vhost-net traditionally
    // gets multiple workers by opening separate device sessions.
    worker_thread: Option<Arc<Thread>>,
    shared: Arc<VhostSharedData<NUM_QUEUES>>,
}

impl<const NUM_QUEUES: usize> VhostSession<NUM_QUEUES> {
    pub(in vhost) fn new(config: VhostDeviceConfig) -> Self {
        assert!(NUM_QUEUES > 0);
        Self {
            config,
            backend_features: 0,
            worker_thread: None,
            shared: Arc::new(VhostSharedData {
                runtime: Mutex::new(VhostRuntimeData::new()),
                stop_requested: AtomicBool::new(false),
                worker_pollee: Pollee::new(),
            }),
        }
    }

    /// Returns the state shared with this session's worker and event producers.
    pub(in vhost) fn shared(&self) -> &Arc<VhostSharedData<NUM_QUEUES>> {
        &self.shared
    }

    /// Handles common ioctls after backend-specific dispatch.
    ///
    /// The backend handles `SET_OWNER` by constructing its work and calling
    /// [`set_owner`](Self::set_owner); it also owns reset and close policy.
    pub(in vhost) fn handle_ioctl(&mut self, raw: RawIoctl) -> Result<i32> {
        use ioctl_defs::{
            GetBackendFeatures, GetFeatures, SetBackendFeatures, SetFeatures, SetVringNum,
        };

        let result = dispatch_ioctl!(match raw {
            cmd @ GetFeatures => {
                cmd.write(&self.config.device_features)?;
                Ok(0)
            }
            cmd @ GetBackendFeatures => {
                cmd.write(&self.config.backend_features)?;
                Ok(0)
            }
            cmd @ SetBackendFeatures => {
                let features = cmd.read()?;
                if features & !self.config.backend_features != 0 {
                    return_errno_with_message!(
                        Errno::EOPNOTSUPP,
                        "vhost backend feature bits are unsupported"
                    );
                }
                self.backend_features = features;
                Ok(0)
            }
            cmd @ SetFeatures => {
                let features = cmd.read()?;
                if features & !self.config.device_features != 0 {
                    return_errno_with_message!(
                        Errno::EOPNOTSUPP,
                        "vhost feature bits are unsupported"
                    );
                }
                self.shared.lock().negotiated_features = features;
                Ok(0)
            }
            cmd @ SetVringNum => {
                let mut runtime = self.shared.lock();
                runtime.check_owner()?;
                let state = cmd.read()?;
                runtime
                    .queue_mut(state.index)?
                    .set_num(state.num, self.config.max_queue_size)?;
                Ok(0)
            }
            _ => self.shared.lock().handle_ioctl(raw),
        });
        self.shared.worker_pollee.notify(IoEvents::IN);
        result
    }

    /// Assigns the owner and creates its worker, rejecting an existing owner.
    ///
    /// The caller must capture `vmar` in the owner's ioctl context and construct
    /// `work` with this session's shared data. The worker stays alive across queue
    /// errors and pauses until explicitly stopped.
    pub(in vhost) fn set_owner(&mut self, vmar: Arc<Vmar>, work: impl VhostWork) -> Result<()> {
        {
            let mut runtime = self.shared.lock();
            if runtime.is_owned() {
                return_errno_with_message!(Errno::EBUSY, "vhost owner is already set");
            }
            runtime.memory = Some(VhostMemorySpace::new(vmar));
        }
        self.start_worker(work)
    }

    /// Starts work bound to the stored owner VMAR, using this session's shared data.
    /// Any previous worker must have been stopped and joined.
    pub(in vhost) fn start_worker(&mut self, mut work: impl VhostWork) -> Result<()> {
        let vmar = self
            .shared
            .lock()
            .owner_vmar()
            .ok_or_else(|| Error::with_message(Errno::EPERM, "vhost owner is not set"))?
            .clone();
        assert!(self.worker_thread.is_none());
        self.shared.stop_requested.store(false, Ordering::Release);
        let shared = self.shared.clone();
        self.worker_thread = Some(
            ThreadOptions::new(move || worker::run(&shared, &mut work))
                .vmar(vmar)
                .spawn(),
        );
        Ok(())
    }

    pub(in vhost) fn enable_queues(&mut self) -> Result<()> {
        let result = self.shared.lock().enable_queues();
        self.shared.worker_pollee.notify(IoEvents::IN);
        result
    }

    pub(in vhost) fn disable_queues(&mut self) {
        self.shared.lock().disable_queues();
        self.shared.worker_pollee.notify(IoEvents::IN);
    }

    /// Disables queues and joins the worker without holding the runtime mutex.
    pub(in vhost) fn stop_worker(&mut self) {
        {
            let mut runtime = self.shared.lock();
            runtime.disable_queues();
            self.shared.stop_requested.store(true, Ordering::Release);
        }
        self.shared.worker_pollee.notify(IoEvents::IN);
        if let Some(thread) = self.worker_thread.take() {
            thread.join();
        }
    }

    /// Releases owner resources after the worker has been stopped and joined.
    pub(in vhost) fn reset_owner(&mut self) {
        assert!(self.worker_thread.is_none());
        self.shared.lock().reset_owner();
        self.backend_features = 0;
    }
}

impl<const NUM_QUEUES: usize> Drop for VhostSession<NUM_QUEUES> {
    fn drop(&mut self) {
        self.stop_worker();
    }
}

/// State shared by the session, its worker, and event producers.
pub(in vhost) struct VhostSharedData<const NUM_QUEUES: usize> {
    runtime: Mutex<VhostRuntimeData<NUM_QUEUES>>,
    stop_requested: AtomicBool,
    worker_pollee: Pollee,
}

impl<const NUM_QUEUES: usize> VhostSharedData<NUM_QUEUES> {
    /// Excludes queue configuration changes while a backend processes a batch.
    pub(in vhost) fn lock(&self) -> MutexGuard<'_, VhostRuntimeData<NUM_QUEUES>> {
        self.runtime.lock()
    }

    /// Returns the pollee for configuration changes and backend work arrivals.
    ///
    /// Producers update durable state before notifying it. The worker registers
    /// before checking that state; notifications themselves carry no work count.
    pub(in vhost) fn worker_pollee(&self) -> &Pollee {
        &self.worker_pollee
    }

    /// Returns whether the session has requested the worker to stop.
    /// A true result does not mean the worker has finished; joining waits for that.
    pub(super) fn stop_requested(&self) -> bool {
        self.stop_requested.load(Ordering::Acquire)
    }
}

/// Negotiated features, memory, and queue progress shared with the worker.
///
/// Queue operations and descriptor chains borrow this data, so reconfiguration waits
/// until guest-memory accesses and completion notifications have finished.
pub(in vhost) struct VhostRuntimeData<const NUM_QUEUES: usize> {
    negotiated_features: u64,
    memory: Option<VhostMemorySpace>,
    queues: [VhostVirtQueue; NUM_QUEUES],
}

impl<const NUM_QUEUES: usize> VhostRuntimeData<NUM_QUEUES> {
    fn new() -> Self {
        Self {
            negotiated_features: 0,
            memory: None,
            queues: array::from_fn(|_| VhostVirtQueue::default()),
        }
    }

    pub(in vhost) fn owner_vmar(&self) -> Option<&Arc<Vmar>> {
        self.memory.as_ref().map(VhostMemorySpace::vmar)
    }

    pub(in vhost) fn is_owned(&self) -> bool {
        self.memory.is_some()
    }

    pub(in vhost) fn is_running(&self) -> bool {
        self.queues.iter().all(|queue| queue.is_enabled())
    }

    pub(in vhost) fn negotiated_features(&self) -> u64 {
        self.negotiated_features
    }

    pub(in vhost) fn queue_base(&self, index: u32) -> Result<u32> {
        let queue = self.queues.get(index as usize).ok_or_else(|| {
            Error::with_message(Errno::ENOBUFS, "vhost queue index is out of range")
        })?;
        Ok(u32::from(queue.base()))
    }

    pub(in vhost) fn kick_event(&self, index: usize) -> Option<&Arc<KernelEventFile>> {
        self.queues.get(index).and_then(|queue| queue.kick_event())
    }

    /// Borrows the memory table and queues together for backend processing.
    ///
    /// The returned borrows keep the runtime locked for guest-memory access.
    /// Backends must check `is_running` before processing chains and unlock before socket
    /// notifications or other work that could acquire it again.
    pub(in vhost) fn memory_and_queues_mut(
        &mut self,
    ) -> Result<(&VhostMemorySpace, &mut [VhostVirtQueue; NUM_QUEUES])> {
        let memory = self
            .memory
            .as_ref()
            .ok_or_else(|| Error::with_message(Errno::EPERM, "vhost owner is not set"))?;
        Ok((memory, &mut self.queues))
    }

    /// Enables all queues, or leaves all queues disabled if activation fails.
    /// Repeated activation checks access without resetting active cursors.
    pub(in vhost) fn enable_queues(&mut self) -> Result<()> {
        let memory = self
            .memory
            .as_ref()
            .ok_or_else(|| Error::with_message(Errno::EPERM, "vhost owner is not set"))?;
        for queue in &mut self.queues {
            if let Err(error) = queue.enable(memory) {
                self.disable_queues();
                return Err(error);
            }
        }
        Ok(())
    }

    /// Disables queue processing while preserving configuration and progress.
    pub(in vhost) fn disable_queues(&mut self) {
        for queue in &mut self.queues {
            queue.disable();
        }
    }

    fn handle_ioctl(&mut self, raw_ioctl: RawIoctl) -> Result<i32> {
        use ioctl_defs::*;

        dispatch_ioctl!(match raw_ioctl {
            cmd @ SetMemTable => {
                self.check_owner()?;
                let table_addr = raw_ioctl
                    .arg()
                    .checked_add(size_of::<VhostMemory>())
                    .ok_or_else(|| {
                        Error::with_message(Errno::EINVAL, "vhost memory table address overflow")
                    })?;
                let header = cmd.read()?;
                let regions = VhostMemoryRegion::read_from_user(table_addr, header)?;
                self.memory.as_mut().unwrap().set_regions(regions)?;
                Ok(0)
            }
            cmd @ SetVringAddr => {
                self.check_owner()?;
                let addr = cmd.read()?;
                self.queue_mut(addr.index)?.set_addr(addr)?;
                Ok(0)
            }
            cmd @ SetVringBase => {
                self.check_owner()?;
                let state = cmd.read()?;
                self.queue_mut(state.index)?.set_base(state.num)?;
                Ok(0)
            }
            cmd @ GetVringBase => {
                self.check_owner()?;
                // Only index is input; num may reside in write-only memory.
                cmd.with_data_ptr(|ptr| {
                    let index = field_ptr!(&ptr, VhostVringState, index).read()?;
                    ptr.write(&VhostVringState {
                        index,
                        num: self.queue_base(index)?,
                    })?;
                    Ok(0)
                })
            }
            cmd @ SetVringKick => {
                self.check_owner()?;
                let file = cmd.read()?;
                self.queue_mut(file.index)?
                    .set_kick(get_event_file(file.fd)?);
                Ok(0)
            }
            cmd @ SetVringCall => {
                self.check_owner()?;
                let file = cmd.read()?;
                self.queue_mut(file.index)?
                    .set_call(get_event_file(file.fd)?);
                Ok(0)
            }
            cmd @ SetVringErr => {
                self.check_owner()?;
                let file = cmd.read()?;
                self.queue_mut(file.index)?
                    .set_err(get_event_file(file.fd)?);
                Ok(0)
            }
            _ => {
                // Linux vsock falls back to vring dispatch, which reads the
                // queue index even for unsupported commands such as RESET_OWNER.
                self.check_owner()?;
                // Unknown commands have no typed argument to decode. Preserve
                // Linux's index-first fallback only on this compatibility path.
                let task = Task::current().unwrap();
                let userspace = CurrentUserSpace::new(task.as_thread_local().unwrap());
                let index: u32 = userspace.read_val(raw_ioctl.arg())?;
                self.queue_mut(index)?;
                return_errno_with_message!(Errno::ENOTTY, "the vhost ioctl command is unknown");
            }
        })
    }

    /// Clears ownership and configuration after the backend's worker has exited.
    fn reset_owner(&mut self) {
        self.memory = None;
        self.negotiated_features = 0;
        self.queues = array::from_fn(|_| VhostVirtQueue::default());
    }

    pub(in vhost) fn check_owner(&self) -> Result<()> {
        let Some(owner) = self.owner_vmar() else {
            return_errno_with_message!(Errno::EPERM, "vhost owner is not set");
        };
        let task = Task::current().unwrap();
        let thread_local = task.as_thread_local().unwrap();
        let caller = thread_local.vmar().borrow();
        if !Arc::ptr_eq(owner, &caller.as_ref().unwrap().clone_arc()) {
            return_errno_with_message!(Errno::EPERM, "vhost caller is not the owner");
        }
        Ok(())
    }

    fn queue_mut(&mut self, index: u32) -> Result<&mut VhostVirtQueue> {
        self.queues
            .get_mut(index as usize)
            .ok_or_else(|| Error::with_message(Errno::ENOBUFS, "vhost queue index is out of range"))
    }
}

fn get_event_file(fd: RawFileDesc) -> Result<Option<Arc<KernelEventFile>>> {
    if fd == -1 {
        return Ok(None);
    }
    let fd = FileDesc::try_from(fd)?;
    let task = Task::current().unwrap();
    let thread_local = task.as_thread_local().unwrap();
    let mut file_table = thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, fd).into_owned();
    KernelEventFile::from_file(file.as_ref()).map(Some)
}
