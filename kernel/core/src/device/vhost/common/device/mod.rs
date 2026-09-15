// SPDX-License-Identifier: MPL-2.0

//! Persistent vhost configuration and exclusive access to split virtqueues.

#![short_vis_path::add(vhost)]

use core::array;

use ostd::{mm::VmIo, task::Task};

use super::{
    memory::{self, VhostMemory, VhostMemorySpace},
    virtqueue::VhostVirtQueue,
    worker::{VhostWork, VhostWorker},
};
use crate::{
    events::{EventFile, EventFileFlags, KernelEventFile},
    fs::file::file_table::{FileDesc, RawFileDesc, get_file_fast},
    prelude::*,
    util::ioctl::{RawIoctl, dispatch_ioctl},
    vm::vmar::Vmar,
};

#[cfg(ktest)]
mod tests;

/// `struct vhost_vring_state` in Linux, a queue index and its size or base.
///
/// Reference: <https://github.com/torvalds/linux/blob/v6.18/include/uapi/linux/vhost_types.h#L19>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(in vhost) struct VhostVringState {
    pub index: u32,
    pub num: u32,
}

/// `struct vhost_vring_file` in Linux, a queue index and its eventfd descriptor.
///
/// Reference: <https://github.com/torvalds/linux/blob/v6.18/include/uapi/linux/vhost_types.h#L24>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(in vhost) struct VhostVringFile {
    pub index: u32,
    pub fd: i32,
}

/// `struct vhost_vring_addr` in Linux, the owner virtual addresses of a queue.
///
/// Reference: <https://github.com/torvalds/linux/blob/v6.18/include/uapi/linux/vhost_types.h#L30>.
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

    // Reference: <https://github.com/torvalds/linux/blob/v6.18/include/uapi/linux/vhost.h#L26-L38>.
    pub(in vhost) type GetFeatures        = ioc!(VHOST_GET_FEATURES,         0xaf, 0x00, OutData<u64>);
    pub(in vhost) type SetFeatures        = ioc!(VHOST_SET_FEATURES,         0xaf, 0x00, InData<u64>);
    pub(in vhost) type SetOwner           = ioc!(VHOST_SET_OWNER,            0xaf, 0x01, NoData);
    pub(in vhost) type ResetOwner         = ioc!(VHOST_RESET_OWNER,          0xaf, 0x02, NoData);
    pub(in vhost) type SetMemTable        = ioc!(VHOST_SET_MEM_TABLE,        0xaf, 0x03, InData<VhostMemory>);
    // Reference: <https://github.com/torvalds/linux/blob/v6.18/include/uapi/linux/vhost.h#L71-L77>.
    pub(in vhost) type SetVringNum        = ioc!(VHOST_SET_VRING_NUM,        0xaf, 0x10, InData<VhostVringState>);
    pub(in vhost) type SetVringAddr       = ioc!(VHOST_SET_VRING_ADDR,       0xaf, 0x11, InData<VhostVringAddr>);
    pub(in vhost) type SetVringBase       = ioc!(VHOST_SET_VRING_BASE,       0xaf, 0x12, InData<VhostVringState>);
    pub(in vhost) type GetVringBase       = ioc!(VHOST_GET_VRING_BASE,       0xaf, 0x12, InOutData<VhostVringState>);
    // Reference: <https://github.com/torvalds/linux/blob/v6.18/include/uapi/linux/vhost.h#L109-L113>.
    pub(in vhost) type SetVringKick       = ioc!(VHOST_SET_VRING_KICK,       0xaf, 0x20, InData<VhostVringFile>);
    pub(in vhost) type SetVringCall       = ioc!(VHOST_SET_VRING_CALL,       0xaf, 0x21, InData<VhostVringFile>);
    pub(in vhost) type SetVringErr        = ioc!(VHOST_SET_VRING_ERR,        0xaf, 0x22, InData<VhostVringFile>);
    // Reference: <https://github.com/torvalds/linux/blob/v6.18/include/uapi/linux/vhost.h#L123-L124>.
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

/// Owns one session's worker and persistent configuration and queues.
///
/// Control operations acquire `worker` before `data`. Worker callbacks only
/// acquire `data`, so the caller can join the thread after releasing the data lock.
/// Session close must explicitly stop work even if callbacks retain the backend.
/// `NUM_QUEUES` excludes queues handled only by the frontend.
pub(in vhost) struct VhostDeviceSession<const NUM_QUEUES: usize> {
    worker: Mutex<VhostWorker>,
    data: Arc<Mutex<VhostDeviceData<NUM_QUEUES>>>,
    wake: Arc<KernelEventFile>,
}

impl<const NUM_QUEUES: usize> VhostDeviceSession<NUM_QUEUES> {
    pub(in vhost) fn new(config: VhostDeviceConfig) -> Self {
        Self::from_data(VhostDeviceData::new(config))
    }

    fn from_data(data: VhostDeviceData<NUM_QUEUES>) -> Self {
        let data = Arc::new(Mutex::new(data));
        let wake = KernelEventFile::from_file(&EventFile::new(0, EventFileFlags::empty())).unwrap();
        Self {
            worker: Mutex::new(VhostWorker::default()),
            data,
            wake,
        }
    }

    /// Serializes a complete ioctl or lifecycle operation, including backend work.
    pub(in vhost) fn lock(&self) -> VhostDeviceSessionGuard<'_, NUM_QUEUES> {
        VhostDeviceSessionGuard {
            device: self,
            worker: self.worker.lock(),
        }
    }

    /// Excludes configuration changes and queue access during a worker batch.
    pub(in vhost) fn lock_data(&self) -> MutexGuard<'_, VhostDeviceData<NUM_QUEUES>> {
        self.data.lock()
    }

    /// Returns the internal event used to wake the worker without locking.
    pub(in vhost) fn wake_event(&self) -> &KernelEventFile {
        &self.wake
    }
}

impl<const NUM_QUEUES: usize> Drop for VhostDeviceSession<NUM_QUEUES> {
    fn drop(&mut self) {
        self.data.lock().disable_queues();
        self.worker.get_mut().stop(&self.wake);
    }
}

/// A held worker mutex, serializing device control and worker lifecycle.
/// Worker callbacks must never acquire this lock.
pub(in vhost) struct VhostDeviceSessionGuard<'a, const NUM_QUEUES: usize> {
    device: &'a VhostDeviceSession<NUM_QUEUES>,
    worker: MutexGuard<'a, VhostWorker>,
}

impl<const NUM_QUEUES: usize> VhostDeviceSessionGuard<'_, NUM_QUEUES> {
    /// Handles common ioctls, creates the owner worker, and wakes it after updates.
    pub(in vhost) fn handle_ioctl(
        &mut self,
        raw: RawIoctl,
        work: impl VhostWork<NUM_QUEUES>,
    ) -> Result<i32> {
        use ioctl_defs::SetOwner;

        let result = dispatch_ioctl!(match raw {
            SetOwner => {
                self.set_owner(capture_owner(), work).map(|()| 0)
            }
            _ => self.device.lock_data().handle_ioctl(raw),
        });
        self.device.wake.signal();
        result
    }

    /// Assigns the owner and creates its worker, rejecting an existing owner.
    pub(in vhost) fn set_owner(
        &mut self,
        vmar: Arc<Vmar>,
        work: impl VhostWork<NUM_QUEUES>,
    ) -> Result<()> {
        {
            let mut data = self.device.lock_data();
            if data.is_owned() {
                return_errno_with_message!(Errno::EBUSY, "vhost owner is already set");
            }
            data.memory = Some(VhostMemorySpace::new(vmar, Vec::new()));
        }
        self.start_worker(work)
    }

    /// Starts a worker using the device's owner; any previous worker must be stopped.
    pub(in vhost) fn start_worker(&mut self, work: impl VhostWork<NUM_QUEUES>) -> Result<()> {
        let vmar = self
            .device
            .lock_data()
            .owner_vmar()
            .ok_or_else(|| Error::with_message(Errno::EPERM, "vhost owner is not set"))?
            .clone();
        self.worker.start(
            vmar,
            self.device.data.clone(),
            self.device.wake.clone(),
            work,
        );
        Ok(())
    }

    pub(in vhost) fn enable_queues(&self) -> Result<()> {
        let result = self.device.lock_data().enable_queues();
        self.device.wake.signal();
        result
    }

    pub(in vhost) fn disable_queues(&self) {
        self.device.lock_data().disable_queues();
        self.device.wake.signal();
    }

    /// Disables queues and joins the worker without holding the data mutex.
    pub(in vhost) fn stop_worker(&mut self) {
        self.device.lock_data().disable_queues();
        self.worker.stop(&self.device.wake);
    }
}

/// The device's single copy of configuration and queue progress under `data`.
///
/// Queue operations and descriptor chains borrow this data, so reconfiguration waits
/// until guest-memory accesses and completion notifications have finished.
pub(in vhost) struct VhostDeviceData<const NUM_QUEUES: usize> {
    config: VhostDeviceConfig,
    negotiated_features: u64,
    backend_features: u64,
    memory: Option<VhostMemorySpace>,
    queues: [VhostVirtQueue; NUM_QUEUES],
}

impl<const NUM_QUEUES: usize> VhostDeviceData<NUM_QUEUES> {
    fn new(config: VhostDeviceConfig) -> Self {
        assert!(NUM_QUEUES > 0);
        Self {
            config,
            negotiated_features: 0,
            backend_features: 0,
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
        let index = self.check_queue_index(index)?;
        Ok(u32::from(self.queues[index].base()))
    }

    pub(in vhost) fn kick_event(&self, index: usize) -> Option<&Arc<KernelEventFile>> {
        self.queues.get(index).and_then(|queue| queue.kick_event())
    }

    /// Borrows the current memory table and queues from the same locked device.
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
        let (memory, queues) = self.memory_and_queues_mut()?;
        for queue in queues {
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
            cmd @ GetFeatures => {
                cmd.write(&self.config.device_features)?;
                Ok(0)
            }
            cmd @ GetBackendFeatures => {
                cmd.write(&self.config.backend_features)?;
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
                self.negotiated_features = features;
                Ok(0)
            }
            cmd @ SetMemTable => {
                self.check_owner()?;
                let header = cmd.read()?;
                let table_addr = raw_ioctl
                    .arg()
                    .checked_add(size_of::<VhostMemory>())
                    .ok_or_else(|| {
                        Error::with_message(Errno::EINVAL, "vhost memory table address overflow")
                    })?;
                let regions = memory::read_memory_regions(table_addr, header)?;
                self.memory.as_mut().unwrap().set_regions(regions)?;
                Ok(0)
            }
            cmd @ SetVringNum => {
                self.check_owner()?;
                let index = self.read_queue_index(raw_ioctl)?;
                // Linux rejects a running queue before reading the remaining input.
                self.queues[index].check_stopped()?;
                self.queues[index].set_num(cmd.read()?.num, self.config.max_queue_size)?;
                Ok(0)
            }
            cmd @ SetVringAddr => {
                self.check_owner()?;
                let index = self.read_queue_index(raw_ioctl)?;
                self.queues[index].set_addr(cmd.read()?)?;
                Ok(0)
            }
            cmd @ SetVringBase => {
                self.check_owner()?;
                let index = self.read_queue_index(raw_ioctl)?;
                // Linux rejects a running queue before reading the remaining input.
                self.queues[index].check_stopped()?;
                self.queues[index].set_base(cmd.read()?.num)?;
                Ok(0)
            }
            cmd @ GetVringBase => {
                self.check_owner()?;
                let index = self.read_queue_index(raw_ioctl)?;
                cmd.write(&VhostVringState {
                    index: index as u32,
                    num: u32::from(self.queues[index].base()),
                })?;
                Ok(0)
            }
            cmd @ SetVringKick => {
                self.check_owner()?;
                let index = self.read_queue_index(raw_ioctl)?;
                self.queues[index].set_kick(get_event_file(cmd.read()?.fd)?);
                Ok(0)
            }
            cmd @ SetVringCall => {
                self.check_owner()?;
                let index = self.read_queue_index(raw_ioctl)?;
                self.queues[index].set_call(get_event_file(cmd.read()?.fd)?);
                Ok(0)
            }
            cmd @ SetVringErr => {
                self.check_owner()?;
                let index = self.read_queue_index(raw_ioctl)?;
                self.queues[index].set_err(get_event_file(cmd.read()?.fd)?);
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
            _ => {
                // Linux vsock falls back to vring dispatch, which reads the
                // queue index even for unsupported commands such as RESET_OWNER.
                self.check_owner()?;
                self.read_queue_index(raw_ioctl)?;
                return_errno_with_message!(Errno::ENOTTY, "the vhost ioctl command is unknown");
            }
        })
    }

    /// Clears ownership and configuration after the backend's worker has exited.
    pub(in vhost) fn reset_owner(&mut self) {
        self.memory = None;
        self.negotiated_features = 0;
        self.backend_features = 0;
        self.queues = array::from_fn(|_| VhostVirtQueue::default());
    }

    pub(in vhost) fn check_owner(&self) -> Result<()> {
        let Some(owner) = self.owner_vmar() else {
            return_errno_with_message!(Errno::EPERM, "vhost owner is not set");
        };
        if !Arc::ptr_eq(owner, &capture_owner()) {
            return_errno_with_message!(Errno::EPERM, "vhost caller is not the owner");
        }
        Ok(())
    }

    fn read_queue_index(&self, raw: RawIoctl) -> Result<usize> {
        let task = Task::current().unwrap();
        let userspace = CurrentUserSpace::new(task.as_thread_local().unwrap());
        self.check_queue_index(userspace.read_val(raw.arg())?)
    }

    fn check_queue_index(&self, index: u32) -> Result<usize> {
        if index as usize >= NUM_QUEUES {
            return_errno_with_message!(Errno::ENOBUFS, "vhost queue index is out of range");
        }
        Ok(index as usize)
    }
}

// Called from a POSIX thread handling a vhost ioctl.
fn capture_owner() -> Arc<Vmar> {
    let task = Task::current().unwrap();
    let thread_local = task.as_thread_local().unwrap();
    thread_local.vmar().borrow().as_ref().unwrap().clone_arc()
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
