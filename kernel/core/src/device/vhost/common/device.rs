// SPDX-License-Identifier: MPL-2.0

//! Common vhost ioctl state and worker runtime snapshots.

#![short_vis_path::add(vhost)]

use core::{
    array,
    sync::atomic::{AtomicU16, AtomicU64, Ordering},
};

use aster_virtio::virtio_ring::{AvailRing, Descriptor, UsedRing};
use ostd::task::Task;

use super::{
    memory::{self, VhostMemory, VhostMemoryRegion, VhostMemorySpace},
    virtqueue::VhostVirtQueue,
};
use crate::{
    events::KernelEventFile,
    fs::file::file_table::{FileDesc, RawFileDesc, get_file_fast},
    prelude::*,
    util::ioctl::{RawIoctl, dispatch_ioctl},
    vm::vmar::Vmar,
};

#[cfg(ktest)]
mod tests;

const VHOST_MAX_VRING_NUM: u32 = 32768;

pub(in vhost) const VIRTIO_F_VERSION_1: u64 = 1 << 32;
pub(in vhost) const VIRTIO_RING_F_INDIRECT_DESC: u64 = 1 << 28;

/// `struct vhost_vring_state` in Linux, a queue index and its size or base.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/linux/vhost_types.h#L18>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(in vhost) struct VhostVringState {
    pub index: u32,
    pub num: u32,
}

/// `struct vhost_vring_file` in Linux, a queue index and its eventfd descriptor.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/linux/vhost_types.h#L23>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(in vhost) struct VhostVringFile {
    pub index: u32,
    pub fd: i32,
}

/// `struct vhost_vring_addr` in Linux, the owner virtual addresses of a queue.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/linux/vhost_types.h#L29>.
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

    // Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/linux/vhost.h>.
    pub(in vhost) type GetFeatures        = ioc!(VHOST_GET_FEATURES,         0xaf, 0x00, OutData<u64>);
    pub(in vhost) type SetFeatures        = ioc!(VHOST_SET_FEATURES,         0xaf, 0x00, InData<u64>);
    pub(in vhost) type SetOwner           = ioc!(VHOST_SET_OWNER,            0xaf, 0x01, NoData);
    pub(in vhost) type ResetOwner         = ioc!(VHOST_RESET_OWNER,          0xaf, 0x02, NoData);
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

/// Configuration state shared by a vhost device file and its backend worker.
///
/// The backend owns this value and forwards common vhost ioctls to
/// [`handle_ioctl`](Self::handle_ioctl). Once all generic state is configured,
/// [`build_runtime`](Self::build_runtime) creates a snapshot for data-plane use.
/// `NUM_QUEUES` is the number of queues handled by this backend session;
/// all must be configured before activation. Frontend-only queues are excluded.
pub(in vhost) struct VhostDeviceState<const NUM_QUEUES: usize> {
    config: VhostDeviceConfig,
    owner_vmar: Option<Arc<Vmar>>,
    negotiated_features: u64,
    backend_features: u64,
    memory_regions: Vec<VhostMemoryRegion>,
    queues: [VhostQueueState; NUM_QUEUES],
    generation: Arc<AtomicU64>,
}

pub(super) struct VhostQueueState {
    pub(super) num: u32,
    pub(super) base: Arc<AtomicU16>,
    pub(super) addr: Option<VhostVringAddr>,
    pub(super) kick: Option<Arc<KernelEventFile>>,
    pub(super) call: Option<Arc<KernelEventFile>>,
    pub(super) err: Option<Arc<KernelEventFile>>,
}

impl Default for VhostQueueState {
    fn default() -> Self {
        Self {
            num: 0,
            base: Arc::new(AtomicU16::new(0)),
            addr: None,
            kick: None,
            call: None,
            err: None,
        }
    }
}

impl<const NUM_QUEUES: usize> VhostDeviceState<NUM_QUEUES> {
    pub(in vhost) fn new(config: VhostDeviceConfig) -> Self {
        assert!(NUM_QUEUES > 0);
        Self {
            config,
            owner_vmar: None,
            negotiated_features: 0,
            backend_features: 0,
            memory_regions: Vec::new(),
            queues: array::from_fn(|_| VhostQueueState::default()),
            generation: Arc::new(AtomicU64::new(0)),
        }
    }

    pub(in vhost) fn is_owned(&self) -> bool {
        self.owner_vmar.is_some()
    }

    pub(in vhost) fn negotiated_features(&self) -> u64 {
        self.negotiated_features
    }

    pub(in vhost) fn is_fully_configured(&self) -> bool {
        self.owner_vmar.is_some()
            && !self.memory_regions.is_empty()
            && self
                .queues
                .iter()
                .all(|queue| queue.num != 0 && queue.addr.is_some())
    }

    pub(in vhost) fn queue_base(&self, index: u32) -> Result<u32> {
        Ok(u32::from(self.queue(index)?.base.load(Ordering::Acquire)))
    }

    pub(in vhost) fn handle_ioctl(&mut self, raw_ioctl: RawIoctl) -> Result<i32> {
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
                self.check_owner()?;
                let features = cmd.read()?;
                if features & !self.config.device_features != 0 {
                    return_errno_with_message!(Errno::EINVAL, "vhost feature bits are unsupported");
                }
                self.negotiated_features = features;
                self.invalidate_runtime();
                Ok(0)
            }
            SetOwner => {
                if self.owner_vmar.is_some() {
                    return_errno_with_message!(Errno::EBUSY, "vhost owner is already set");
                }
                self.owner_vmar = Some(capture_owner());
                self.invalidate_runtime();
                Ok(0)
            }
            cmd @ SetMemTable => {
                self.check_owner()?;
                let memory = cmd.read()?;
                let table_addr = raw_ioctl
                    .arg()
                    .checked_add(size_of::<VhostMemory>())
                    .ok_or_else(|| {
                        Error::with_message(Errno::EINVAL, "vhost memory table address overflow")
                    })?;
                let mut regions = memory::read_memory_regions(table_addr, memory)?;
                memory::sort_and_validate_memory_regions(&mut regions)?;
                self.memory_regions = regions;
                self.invalidate_runtime();
                Ok(0)
            }
            cmd @ SetVringNum => {
                self.check_owner()?;
                let state = cmd.read()?;
                let index = self.check_queue_index(state.index)?;
                validate_vring_num(state.num, self.config.max_queue_size)?;
                if let Some(addr) = self.queues[index].addr.as_ref() {
                    validate_vring_addr(addr, state.num)?;
                }
                self.queues[index].num = state.num;
                self.invalidate_runtime();
                Ok(0)
            }
            cmd @ SetVringAddr => {
                self.check_owner()?;
                let addr = cmd.read()?;
                let index = self.check_queue_index(addr.index)?;
                validate_vring_addr(&addr, self.queues[index].num)?;
                self.queues[index].addr = Some(addr);
                self.invalidate_runtime();
                Ok(0)
            }
            cmd @ SetVringBase => {
                self.check_owner()?;
                let state = cmd.read()?;
                let index = self.check_queue_index(state.index)?;
                validate_vring_base(state.num)?;
                self.queues[index]
                    .base
                    .store(state.num as u16, Ordering::Release);
                self.invalidate_runtime();
                Ok(0)
            }
            cmd @ GetVringBase => {
                self.check_owner()?;
                let mut state = cmd.read()?;
                let index = self.check_queue_index(state.index)?;
                state.num = u32::from(self.queues[index].base.load(Ordering::Acquire));
                cmd.write(&state)?;
                Ok(0)
            }
            cmd @ SetVringKick => {
                self.check_owner()?;
                let file = cmd.read()?;
                let index = self.check_queue_index(file.index)?;
                self.queues[index].kick = get_event_file(file.fd)?;
                self.invalidate_runtime();
                Ok(0)
            }
            cmd @ SetVringCall => {
                self.check_owner()?;
                let file = cmd.read()?;
                let index = self.check_queue_index(file.index)?;
                self.queues[index].call = get_event_file(file.fd)?;
                self.invalidate_runtime();
                Ok(0)
            }
            cmd @ SetVringErr => {
                self.check_owner()?;
                let file = cmd.read()?;
                let index = self.check_queue_index(file.index)?;
                self.queues[index].err = get_event_file(file.fd)?;
                self.invalidate_runtime();
                Ok(0)
            }
            cmd @ SetBackendFeatures => {
                self.check_owner()?;
                let features = cmd.read()?;
                if features & !self.config.backend_features != 0 {
                    return_errno_with_message!(
                        Errno::EINVAL,
                        "vhost backend feature bits are unsupported"
                    );
                }
                self.backend_features = features;
                self.invalidate_runtime();
                Ok(0)
            }
            _ => return_errno_with_message!(Errno::ENOTTY, "the vhost ioctl command is unknown"),
        })
    }

    /// Builds a snapshot in the owner process context, reading the used-ring headers.
    ///
    /// Stop and join existing workers before building a replacement. Move the snapshot
    /// to a worker bound to [`VhostRuntime::vmar`] before accessing queues there.
    pub(in vhost) fn build_runtime(&self) -> Result<VhostRuntime<NUM_QUEUES>> {
        self.check_owner()?;
        if !self.is_fully_configured() {
            return_errno_with_message!(Errno::EINVAL, "vhost device is not fully configured");
        }
        let owner = self.owner_vmar.as_ref().unwrap().clone();
        let memory = VhostMemorySpace::new(owner.clone(), self.memory_regions.clone())?;
        let allow_indirect = self.negotiated_features & VIRTIO_RING_F_INDIRECT_DESC != 0;
        let queues = self
            .queues
            .iter()
            .map(|queue| {
                let addr = queue.addr.as_ref().unwrap();
                validate_vring_addr(addr, queue.num)?;
                VhostVirtQueue::new(memory.clone(), queue, allow_indirect)
            })
            .collect::<Result<Vec<_>>>()?;
        // Exactly one queue was built for each entry of the fixed-size state array.
        let queues = queues.try_into().ok().unwrap();
        Ok(VhostRuntime {
            vmar: owner,
            generation: self.generation.load(Ordering::Acquire),
            current_generation: self.generation.clone(),
            queues,
        })
    }

    /// Clears ownership and all common configuration.
    ///
    /// A backend must stop and join every worker that can access a
    /// [`VhostRuntime`] before calling this method. Owner reset is deliberately
    /// not handled by [`handle_ioctl`](Self::handle_ioctl), because only the
    /// backend can enforce that lifecycle ordering.
    pub(in vhost) fn reset_owner_after_quiesce(&mut self) {
        self.owner_vmar = None;
        self.negotiated_features = 0;
        self.backend_features = 0;
        self.memory_regions.clear();
        for queue in &mut self.queues {
            *queue = VhostQueueState::default();
        }
        self.invalidate_runtime();
    }

    /// Checks that the caller owns this vhost session.
    pub(in vhost) fn check_owner(&self) -> Result<()> {
        let Some(owner) = self.owner_vmar.as_ref() else {
            return_errno_with_message!(Errno::EPERM, "vhost owner is not set");
        };
        let current = capture_owner();
        if !Arc::ptr_eq(owner, &current) {
            return_errno_with_message!(Errno::EPERM, "vhost caller is not the owner");
        }
        Ok(())
    }

    fn check_queue_index(&self, index: u32) -> Result<usize> {
        let index = index as usize;
        if index >= NUM_QUEUES {
            return_errno_with_message!(Errno::EINVAL, "vhost queue index is out of range");
        }
        Ok(index)
    }

    fn queue(&self, index: u32) -> Result<&VhostQueueState> {
        let index = self.check_queue_index(index)?;
        Ok(&self.queues[index])
    }

    fn invalidate_runtime(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
    }
}

/// A data-plane snapshot. Any later control-plane mutation makes this snapshot
/// stale. Generation checks do not synchronize in-flight operations or chains
/// already handed to a worker. Backends must quiesce workers before changing
/// configuration, then build a replacement in the owner process context.
pub(in vhost) struct VhostRuntime<const NUM_QUEUES: usize> {
    vmar: Arc<Vmar>,
    generation: u64,
    current_generation: Arc<AtomicU64>,
    queues: [VhostVirtQueue; NUM_QUEUES],
}

impl<const NUM_QUEUES: usize> VhostRuntime<NUM_QUEUES> {
    /// Returns the VMAR to associate with each worker using `ThreadOptions::vmar`.
    ///
    /// This reference does not keep the owner's mappings alive after owner exit.
    pub(in vhost) fn vmar(&self) -> &Arc<Vmar> {
        &self.vmar
    }

    pub(in vhost) fn is_current(&self) -> bool {
        self.current_generation.load(Ordering::Acquire) == self.generation
    }

    pub(in vhost) fn queue_mut(&mut self, index: usize) -> Result<&mut VhostVirtQueue> {
        if !self.is_current() {
            return_errno_with_message!(Errno::EBUSY, "vhost runtime configuration is stale");
        }
        self.queues.get_mut(index).ok_or_else(|| {
            Error::with_message(Errno::EINVAL, "vhost runtime queue index is out of range")
        })
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

fn validate_vring_num(num: u32, max: u32) -> Result<()> {
    if num == 0 || num > max || num > VHOST_MAX_VRING_NUM || !num.is_power_of_two() {
        return_errno_with_message!(Errno::EINVAL, "vhost vring size is invalid");
    }
    Ok(())
}

fn validate_vring_base(base: u32) -> Result<()> {
    if base > u32::from(u16::MAX) {
        return_errno_with_message!(Errno::EINVAL, "vhost vring base is too large");
    }
    Ok(())
}

fn validate_vring_addr(addr: &VhostVringAddr, num: u32) -> Result<()> {
    if addr.flags != 0 || addr.log_guest_addr != 0 {
        return_errno_with_message!(Errno::EINVAL, "vhost vring address flags are invalid");
    }
    if !addr
        .avail_user_addr
        .is_multiple_of(align_of::<AvailRing>() as u64)
        || !addr
            .used_user_addr
            .is_multiple_of(align_of::<UsedRing>() as u64)
    {
        return_errno_with_message!(Errno::EINVAL, "vhost vring address is misaligned");
    }
    memory::validate_owner_range(addr.desc_user_addr as usize, 0)?;
    memory::validate_owner_range(addr.avail_user_addr as usize, 0)?;
    memory::validate_owner_range(addr.used_user_addr as usize, 0)?;

    // Linux permits setting addresses before the queue size. The complete
    // ranges are validated when a size is available and again at activation.
    if num == 0 {
        return Ok(());
    }
    let num = num as usize;
    let desc_len = num * size_of::<Descriptor>();
    let avail_len = AvailRing::entry_offset(num).unwrap();
    let used_len = UsedRing::entry_offset(num).unwrap();
    memory::validate_owner_range(addr.desc_user_addr as usize, desc_len)?;
    memory::validate_owner_range(addr.avail_user_addr as usize, avail_len)?;
    memory::validate_owner_range(addr.used_user_addr as usize, used_len)?;
    Ok(())
}
