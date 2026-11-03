// SPDX-License-Identifier: MPL-2.0

//! Common file state, shared queue state, and worker lifecycle for vhost backends.

#![short_vis_path::add(vhost)]

use core::{
    array,
    sync::atomic::{AtomicBool, Ordering},
};

use ostd::task::Task;

use super::{
    memory::{VhostMemory, VhostMemoryRegion, VhostMemorySpace},
    virtqueue::VhostVirtQueue,
    worker::{self, VhostWorkStatus},
};
use crate::{
    dispatch_ioctl,
    events::{IoEvents, KernelEventFile},
    fs::file::file_table::{FileDesc, RawFileDesc, get_file_fast},
    prelude::*,
    process::signal::Pollee,
    thread::{Thread, kernel_thread::ThreadOptions},
    util::ioctl::RawIoctl,
    vm::vmar::Vmar,
};

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
    use crate::{
        ioc,
        util::ioctl::{InData, InOutData, NoData, OutData},
    };

    // Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/linux/vhost.h#L26>.
    pub(in vhost) type GetFeatures        = ioc!(VHOST_GET_FEATURES,         0xaf, 0x00, OutData<u64>);
    pub(in vhost) type SetFeatures        = ioc!(VHOST_SET_FEATURES,         0xaf, 0x00, InData<u64>);
    pub(in vhost) type SetOwner           = ioc!(VHOST_SET_OWNER,            0xaf, 0x01, NoData);
    pub(in vhost) type SetMemTable        = ioc!(VHOST_SET_MEM_TABLE,        0xaf, 0x03, InData<VhostMemory>);
    pub(in vhost) type SetVringNum        = ioc!(VHOST_SET_VRING_NUM,        0xaf, 0x10, InData<VhostVringState>);
    pub(in vhost) type SetVringAddr       = ioc!(VHOST_SET_VRING_ADDR,       0xaf, 0x11, InData<VhostVringAddr>);
    pub(in vhost) type SetVringBase       = ioc!(VHOST_SET_VRING_BASE,       0xaf, 0x12, InData<VhostVringState>);
    pub(in vhost) type GetVringBase       = ioc!(VHOST_GET_VRING_BASE,       0xaf, 0x12, InOutData<VhostVringState>);
    pub(in vhost) type SetVringKick       = ioc!(VHOST_SET_VRING_KICK,       0xaf, 0x20, InData<VhostVringFile>);
    pub(in vhost) type SetVringCall       = ioc!(VHOST_SET_VRING_CALL,       0xaf, 0x21, InData<VhostVringFile>);
    pub(in vhost) type SetVringErr        = ioc!(VHOST_SET_VRING_ERR,        0xaf, 0x22, InData<VhostVringFile>);
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

/// The common configuration and worker lifecycle of an open vhost file.
///
/// A backend pairs this with a [`VhostSharedState`] for each open file.
/// It protects common state with a mutex to serialize ioctls and close,
/// acquiring that mutex before the shared state's runtime mutex.
/// Workers and socket producers do not acquire the common mutex.
///
/// [`set_owner`](Self::set_owner) binds the worker to the owner's address space.
/// The concrete file must call [`reset_owner`](Self::reset_owner) on close
/// to join the worker and release ownership, even if callbacks retain the backend.
pub(in vhost) struct VhostFileCommon {
    config: VhostDeviceConfig,
    backend_features: u64,
    // FIXME: Support multiple workers per file for devices such as vhost-scsi.
    worker_thread: Option<Arc<Thread>>,
}

impl VhostFileCommon {
    pub(in vhost) fn new(config: VhostDeviceConfig) -> Self {
        Self {
            config,
            backend_features: 0,
            worker_thread: None,
        }
    }

    /// Handles common ioctls after backend-specific dispatch.
    ///
    /// The backend handles `SET_OWNER` by constructing its work and calling
    /// [`set_owner`](Self::set_owner); it also owns reset and close policy.
    pub(in vhost) fn handle_ioctl<const NUM_QUEUES: usize>(
        &mut self,
        raw: RawIoctl,
        shared: &VhostSharedState<NUM_QUEUES>,
    ) -> Result<i32> {
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
                shared.runtime().lock().negotiated_features = features;
                Ok(0)
            }
            cmd @ SetVringNum => {
                let mut runtime = shared.runtime().lock();
                runtime.check_owner()?;
                let state = cmd.read()?;
                runtime
                    .queue_mut(state.index)?
                    .set_num(state.num, self.config.max_queue_size)?;
                Ok(0)
            }
            _ => shared.runtime().lock().handle_ioctl(raw),
        });
        shared.worker_pollee.notify(IoEvents::IN);
        result
    }

    /// Assigns the owner and starts its worker, rejecting an existing owner.
    ///
    /// The caller captures `vmar` in the owner's ioctl context.
    /// `work_fn` may run while queues are paused;
    /// it must lock the runtime and check that queues are enabled before using them.
    ///
    /// The callback handles queue errors and chooses its batch budget. Before
    /// returning `Idle`, it must restore kicks and check for pending requests;
    /// `Pending` continues without waiting. Queue faults and pauses do not stop
    /// the worker; [`reset_owner`](Self::reset_owner) stops and joins it.
    pub(in vhost) fn set_owner<const NUM_QUEUES: usize>(
        &mut self,
        shared: &Arc<VhostSharedState<NUM_QUEUES>>,
        vmar: Arc<Vmar>,
        work_fn: impl FnMut(&VhostSharedState<NUM_QUEUES>) -> VhostWorkStatus + Send + 'static,
    ) -> Result<()> {
        {
            let mut runtime = shared.runtime().lock();
            if runtime.is_owned() {
                return_errno_with_message!(Errno::EBUSY, "vhost owner is already set");
            }
            runtime.memory = Some(VhostMemorySpace::new(vmar));
        }
        self.start_worker(shared, work_fn);
        Ok(())
    }

    fn start_worker<const NUM_QUEUES: usize>(
        &mut self,
        shared: &Arc<VhostSharedState<NUM_QUEUES>>,
        mut work_fn: impl FnMut(&VhostSharedState<NUM_QUEUES>) -> VhostWorkStatus + Send + 'static,
    ) {
        let vmar = shared.runtime().lock().owner_vmar().unwrap().clone();
        debug_assert!(self.worker_thread.is_none());
        shared.stop_worker_requested.store(false, Ordering::Release);
        let shared = shared.clone();
        self.worker_thread = Some(
            ThreadOptions::new(move || worker::run(&shared, &mut work_fn))
                .vmar(vmar)
                .spawn(),
        );
    }

    /// Disables queues and joins the worker.
    fn stop_worker<const NUM_QUEUES: usize>(&mut self, shared: &VhostSharedState<NUM_QUEUES>) {
        {
            let mut runtime = shared.runtime().lock();
            runtime.disable_queues();
            shared.stop_worker_requested.store(true, Ordering::Release);
        }
        shared.worker_pollee.notify(IoEvents::IN);
        if let Some(thread) = self.worker_thread.take() {
            // The worker needs the runtime lock to observe the stop request.
            thread.join();
        }
    }

    /// Stops and joins the worker, then releases ownership and configuration.
    pub(in vhost) fn reset_owner<const NUM_QUEUES: usize>(
        &mut self,
        shared: &VhostSharedState<NUM_QUEUES>,
    ) {
        self.stop_worker(shared);
        shared.runtime().lock().reset_owner();
        self.backend_features = 0;
    }
}

/// Queue state and notifications shared by a vhost file and its worker.
///
/// `NUM_QUEUES` counts the queues processed by the backend worker,
/// excluding queues handled entirely by the frontend.
pub(in vhost) struct VhostSharedState<const NUM_QUEUES: usize> {
    runtime: Mutex<VhostRuntimeState<NUM_QUEUES>>,
    stop_worker_requested: AtomicBool,
    worker_pollee: Pollee,
}

impl<const NUM_QUEUES: usize> VhostSharedState<NUM_QUEUES> {
    pub(in vhost) fn new() -> Self {
        assert!(NUM_QUEUES > 0);
        Self {
            runtime: Mutex::new(VhostRuntimeState::new()),
            stop_worker_requested: AtomicBool::new(false),
            worker_pollee: Pollee::new(),
        }
    }

    #[cfg_attr(not(ktest), expect(dead_code))]
    pub(in vhost) fn enable_queues(&self) -> Result<()> {
        let result = self.runtime().lock().enable_queues();
        self.worker_pollee.notify(IoEvents::IN);
        result
    }

    pub(in vhost) fn disable_queues(&self) {
        self.runtime().lock().disable_queues();
        self.worker_pollee.notify(IoEvents::IN);
    }

    /// Returns the runtime state.
    pub(in vhost) fn runtime(&self) -> &Mutex<VhostRuntimeState<NUM_QUEUES>> {
        &self.runtime
    }

    /// Returns the pollee.
    pub(in vhost) fn worker_pollee(&self) -> &Pollee {
        &self.worker_pollee
    }

    /// Returns whether the file has requested the worker to stop.
    pub(super) fn stop_worker_requested(&self) -> bool {
        self.stop_worker_requested.load(Ordering::Acquire)
    }
}

/// Negotiated features, guest memory regions, and queues shared with the worker.
pub(in vhost) struct VhostRuntimeState<const NUM_QUEUES: usize> {
    negotiated_features: u64,
    memory: Option<VhostMemorySpace>,
    queues: [VhostVirtQueue; NUM_QUEUES],
}

impl<const NUM_QUEUES: usize> VhostRuntimeState<NUM_QUEUES> {
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

    /// Borrows guest memory and queues together for backend processing.
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

    /// Disables queue processing.
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
                // FIXME: For many vhost ioctls, Linux reads and validates the queue
                // index before reading the entire payload. Reading it all upfront
                // may cause error code discrepancies in rare corner cases.
                let mut state = cmd.read()?;
                state.num = self.queue_base(state.index)?;
                cmd.write(&state)?;
                Ok(0)
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
            _ => return_errno_with_message!(Errno::ENOTTY, "the vhost ioctl command is unknown"),
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

// Kernel-thread fixtures have no POSIX ioctl context.
#[cfg(ktest)]
impl VhostFileCommon {
    pub(in vhost) fn configure_for_test<const NUM_QUEUES: usize>(
        &mut self,
        shared: &VhostSharedState<NUM_QUEUES>,
        regions: Vec<VhostMemoryRegion>,
        queue_size: u32,
        addresses: [VhostVringAddr; NUM_QUEUES],
    ) -> Result<()> {
        let mut runtime = shared.runtime().lock();
        runtime.memory.as_mut().unwrap().set_regions(regions)?;
        runtime.negotiated_features = self.config.device_features;
        for (queue, addr) in runtime.queues.iter_mut().zip(addresses) {
            queue.set_num(queue_size, self.config.max_queue_size)?;
            queue.set_addr(addr)?;
        }
        Ok(())
    }
}
