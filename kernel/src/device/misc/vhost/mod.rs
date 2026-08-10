// SPDX-License-Identifier: MPL-2.0

//! Common state and data-plane primitives for Linux vhost backends.
//!
//! The module deliberately has no device, protocol, worker, or hypervisor
//! policy. A backend supplies its feature masks and queue count, then consumes
//! the validated split-virtqueue API exposed here.

use core::{
    array,
    sync::atomic::{AtomicU16, AtomicU64, Ordering},
};

use aster_virtio::virtio_ring::{
    AvailRing, UsedRing, avail_entry_offset, descriptor_offset, used_entry_offset,
};
use ostd::{mm::VmIo, task::Task};
use smallvec::SmallVec;

use crate::{
    events::KernelEventFile,
    fs::file::file_table::{FileDesc, RawFileDesc, get_file_fast},
    prelude::*,
    util::ioctl::{RawIoctl, dispatch_ioctl},
    vm::vmar::{VMAR_CAP_ADDR, VMAR_LOWEST_ADDR, Vmar},
};

mod virtqueue;

#[cfg(ktest)]
mod tests;

#[expect(
    unused_imports,
    reason = "the complete facade is consumed by the follow-up backend"
)]
pub(in crate::device::misc) use self::virtqueue::{
    VhostChainReader, VhostChainWriter, VhostDescriptorChain, VhostVirtQueue,
};

const VHOST_MAX_VRING_NUM: u32 = 32768;
const VHOST_MAX_MEMORY_REGIONS: usize = 64;
const VHOST_MAX_IOV: usize = 1024;

pub(in crate::device::misc) const VIRTIO_F_VERSION_1: u64 = 1 << 32;
pub(in crate::device::misc) const VIRTIO_RING_F_INDIRECT_DESC: u64 = 1 << 28;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(in crate::device::misc) struct VhostMemory {
    pub nregions: u32,
    pub padding: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(in crate::device::misc) struct VhostMemoryRegion {
    pub guest_phys_addr: u64,
    pub memory_size: u64,
    pub userspace_addr: u64,
    pub flags_padding: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(in crate::device::misc) struct VhostVringState {
    pub index: u32,
    pub num: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(in crate::device::misc) struct VhostVringFile {
    pub index: u32,
    pub fd: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(in crate::device::misc) struct VhostVringAddr {
    pub index: u32,
    pub flags: u32,
    pub desc_user_addr: u64,
    pub used_user_addr: u64,
    pub avail_user_addr: u64,
    pub log_guest_addr: u64,
}

pub(in crate::device::misc) mod ioctl_defs {
    use super::{VhostMemory, VhostVringAddr, VhostVringFile, VhostVringState};
    use crate::util::ioctl::{InData, InOutData, NoData, OutData, ioc};

    // Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/linux/vhost.h>.
    pub(in crate::device::misc) type GetFeatures        = ioc!(VHOST_GET_FEATURES,         0xaf, 0x00, OutData<u64>);
    pub(in crate::device::misc) type SetFeatures        = ioc!(VHOST_SET_FEATURES,         0xaf, 0x00, InData<u64>);
    pub(in crate::device::misc) type SetOwner           = ioc!(VHOST_SET_OWNER,            0xaf, 0x01, NoData);
    pub(in crate::device::misc) type ResetOwner         = ioc!(VHOST_RESET_OWNER,          0xaf, 0x02, NoData);
    pub(in crate::device::misc) type SetMemTable        = ioc!(VHOST_SET_MEM_TABLE,        0xaf, 0x03, InData<VhostMemory>);
    pub(in crate::device::misc) type SetVringNum        = ioc!(VHOST_SET_VRING_NUM,        0xaf, 0x10, InData<VhostVringState>);
    pub(in crate::device::misc) type SetVringAddr       = ioc!(VHOST_SET_VRING_ADDR,       0xaf, 0x11, InData<VhostVringAddr>);
    pub(in crate::device::misc) type SetVringBase       = ioc!(VHOST_SET_VRING_BASE,       0xaf, 0x12, InData<VhostVringState>);
    pub(in crate::device::misc) type GetVringBase       = ioc!(VHOST_GET_VRING_BASE,       0xaf, 0x12, InOutData<VhostVringState>);
    pub(in crate::device::misc) type SetVringKick       = ioc!(VHOST_SET_VRING_KICK,       0xaf, 0x20, InData<VhostVringFile>);
    pub(in crate::device::misc) type SetVringCall       = ioc!(VHOST_SET_VRING_CALL,       0xaf, 0x21, InData<VhostVringFile>);
    pub(in crate::device::misc) type SetVringErr        = ioc!(VHOST_SET_VRING_ERR,        0xaf, 0x22, InData<VhostVringFile>);
    pub(in crate::device::misc) type SetBackendFeatures = ioc!(VHOST_SET_BACKEND_FEATURES, 0xaf, 0x25, InData<u64>);
    pub(in crate::device::misc) type GetBackendFeatures = ioc!(VHOST_GET_BACKEND_FEATURES, 0xaf, 0x26, OutData<u64>);
}

/// Accesses userspace addresses in the process that owns this vhost session.
///
/// The queue and GPA translation logic depend only on this boundary. A future
/// MM implementation can replace alien copies with an activated owner address
/// space without changing the vhost data plane.
trait OwnerMemory: Send + Sync {
    fn read(&self, addr: usize, dst: &mut [u8]) -> Result<()>;
    fn write(&self, addr: usize, src: &[u8]) -> Result<()>;
}

struct AlienOwnerMemory(Arc<Vmar>);

// FIXME: `read_alien`/`write_alien` resolve owner pages on every access and are the
// dominant cost in large-payload vhost-vsock profiles. Add an MM-provided fast path that
// lets a worker safely access its owner's address space, as Linux vhost workers do by
// adopting `dev->mm`, or that retains suitable shared frames. Keep that policy in MM so
// the vhost data plane can continue to depend only on `OwnerMemory`.
impl OwnerMemory for AlienOwnerMemory {
    fn read(&self, addr: usize, dst: &mut [u8]) -> Result<()> {
        let mut writer = VmWriter::from(dst).to_fallible();
        self.0.read_alien(addr, &mut writer).map_err(|(e, _)| e)?;
        Ok(())
    }

    fn write(&self, addr: usize, src: &[u8]) -> Result<()> {
        let mut reader = VmReader::from(src).to_fallible();
        self.0.write_alien(addr, &mut reader).map_err(|(e, _)| e)?;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MemorySegment {
    addr: usize,
    len: usize,
}

type MemorySegments = SmallVec<[MemorySegment; 4]>;

#[derive(Clone)]
struct VhostMemorySpace {
    owner_memory: Arc<dyn OwnerMemory>,
    regions: Arc<[VhostMemoryRegion]>,
}

impl VhostMemorySpace {
    fn new(owner_memory: Arc<dyn OwnerMemory>, regions: Vec<VhostMemoryRegion>) -> Result<Self> {
        let regions = validate_memory_regions(regions)?;
        if regions.is_empty() {
            return_errno_with_message!(Errno::EINVAL, "vhost memory table is empty");
        }
        Ok(Self {
            owner_memory,
            regions: regions.into(),
        })
    }

    fn read_owner(&self, addr: usize, dst: &mut [u8]) -> Result<()> {
        self.owner_memory.read(addr, dst)
    }

    fn write_owner(&self, addr: usize, src: &[u8]) -> Result<()> {
        self.owner_memory.write(addr, src)
    }

    fn read_owner_obj<T: Default + Pod>(&self, addr: usize) -> Result<T> {
        let mut value = T::default();
        self.read_owner(addr, value.as_mut_bytes())?;
        Ok(value)
    }

    fn write_owner_obj<T: Pod>(&self, addr: usize, value: &T) -> Result<()> {
        self.write_owner(addr, value.as_bytes())
    }

    fn translate(&self, guest_addr: u64, len: usize) -> Result<MemorySegments> {
        let mut segments = MemorySegments::new();
        self.translate_into(guest_addr, len, &mut segments)?;
        Ok(segments)
    }

    fn translate_into(
        &self,
        guest_addr: u64,
        len: usize,
        segments: &mut MemorySegments,
    ) -> Result<()> {
        let end = guest_addr
            .checked_add(u64::try_from(len).map_err(|_| {
                Error::with_message(Errno::EINVAL, "vhost guest range length is invalid")
            })?)
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "vhost guest range overflow"))?;
        let mut current = guest_addr;
        let mut region_index = self
            .regions
            .partition_point(|region| region.guest_phys_addr <= current)
            .checked_sub(1);

        while current < end {
            let index = region_index.ok_or_else(|| {
                Error::with_message(Errno::EFAULT, "vhost guest range is not mapped")
            })?;
            let region = self.regions.get(index).ok_or_else(|| {
                Error::with_message(Errno::EFAULT, "vhost guest range is not mapped")
            })?;
            let region_end = region.guest_phys_addr + region.memory_size;
            if current < region.guest_phys_addr || current >= region_end {
                return_errno_with_message!(Errno::EFAULT, "vhost guest range is not mapped");
            }
            let segment_end = end.min(region_end);
            let segment_len = usize::try_from(segment_end - current).map_err(|_| {
                Error::with_message(Errno::EINVAL, "vhost memory segment is too large")
            })?;
            let userspace_addr = region
                .userspace_addr
                .checked_add(current - region.guest_phys_addr)
                .ok_or_else(|| {
                    Error::with_message(Errno::EINVAL, "vhost userspace address overflow")
                })?;
            segments.push(MemorySegment {
                addr: usize::try_from(userspace_addr).map_err(|_| {
                    Error::with_message(Errno::EINVAL, "vhost userspace address is invalid")
                })?,
                len: segment_len,
            });
            current = segment_end;
            region_index = Some(index + 1);
        }
        Ok(())
    }

    fn read_guest(&self, guest_addr: u64, dst: &mut [u8]) -> Result<()> {
        let segments = self.translate(guest_addr, dst.len())?;
        let mut offset = 0usize;
        for segment in segments {
            let end = offset.checked_add(segment.len).ok_or_else(|| {
                Error::with_message(Errno::EINVAL, "vhost memory segment offset overflow")
            })?;
            self.owner_memory
                .read(segment.addr, &mut dst[offset..end])?;
            offset = end;
        }
        Ok(())
    }
}

/// Backend-specific feature masks and split-ring limits.
#[derive(Clone, Copy, Debug)]
pub(in crate::device::misc) struct VhostDeviceConfig {
    pub device_features: u64,
    pub backend_features: u64,
    pub max_queue_size: u32,
}

/// Configuration state shared by a vhost device file and its backend worker.
///
/// The backend owns this value and forwards common vhost ioctls to
/// [`handle_ioctl`](Self::handle_ioctl). Once all generic state is configured,
/// [`build_runtime`](Self::build_runtime) creates a snapshot for data-plane use.
pub(in crate::device::misc) struct VhostDeviceState<const NUM_QUEUES: usize> {
    config: VhostDeviceConfig,
    owner_vmar: Option<Arc<Vmar>>,
    features: u64,
    backend_features: u64,
    memory_regions: Vec<VhostMemoryRegion>,
    queues: [VhostQueueState; NUM_QUEUES],
    generation: Arc<AtomicU64>,
}

struct VhostQueueState {
    num: u32,
    base: Arc<AtomicU16>,
    addr: Option<VhostVringAddr>,
    kick: Option<Arc<KernelEventFile>>,
    call: Option<Arc<KernelEventFile>>,
    err: Option<Arc<KernelEventFile>>,
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
    pub(in crate::device::misc) fn new(config: VhostDeviceConfig) -> Self {
        assert!(NUM_QUEUES > 0);
        Self {
            config,
            owner_vmar: None,
            features: 0,
            backend_features: 0,
            memory_regions: Vec::new(),
            queues: array::from_fn(|_| VhostQueueState::default()),
            generation: Arc::new(AtomicU64::new(0)),
        }
    }

    pub(in crate::device::misc) fn is_owned(&self) -> bool {
        self.owner_vmar.is_some()
    }

    pub(in crate::device::misc) fn negotiated_features(&self) -> u64 {
        self.features
    }

    pub(in crate::device::misc) fn is_fully_configured(&self) -> bool {
        self.owner_vmar.is_some()
            && !self.memory_regions.is_empty()
            && self
                .queues
                .iter()
                .all(|queue| queue.num != 0 && queue.addr.is_some())
    }

    pub(in crate::device::misc) fn queue_base(&self, index: u32) -> Result<u32> {
        Ok(u32::from(self.queue(index)?.base.load(Ordering::Acquire)))
    }

    pub(in crate::device::misc) fn handle_ioctl(&mut self, raw_ioctl: RawIoctl) -> Result<i32> {
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
                self.ensure_configurable()?;
                let features = cmd.read()?;
                if features & !self.config.device_features != 0 {
                    return_errno_with_message!(Errno::EINVAL, "vhost feature bits are unsupported");
                }
                self.features = features;
                self.invalidate_runtime();
                Ok(0)
            }
            SetOwner => {
                if self.owner_vmar.is_some() {
                    return_errno_with_message!(Errno::EBUSY, "vhost owner is already set");
                }
                self.owner_vmar = Some(capture_owner()?);
                self.invalidate_runtime();
                Ok(0)
            }
            cmd @ SetMemTable => {
                self.ensure_configurable()?;
                let memory = cmd.read()?;
                if memory.padding != 0 {
                    return_errno_with_message!(
                        Errno::EINVAL,
                        "vhost memory table padding must be zero"
                    );
                }
                let regions = read_memory_regions(raw_ioctl, memory)?;
                self.memory_regions = validate_memory_regions(regions)?;
                self.invalidate_runtime();
                Ok(0)
            }
            cmd @ SetVringNum => {
                self.ensure_configurable()?;
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
                self.ensure_configurable()?;
                let addr = cmd.read()?;
                let index = self.check_queue_index(addr.index)?;
                validate_vring_addr(&addr, self.queues[index].num)?;
                self.queues[index].addr = Some(addr);
                self.invalidate_runtime();
                Ok(0)
            }
            cmd @ SetVringBase => {
                self.ensure_configurable()?;
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
                self.ensure_owner()?;
                let mut state = cmd.read()?;
                let index = self.check_queue_index(state.index)?;
                state.num = u32::from(self.queues[index].base.load(Ordering::Acquire));
                cmd.write(&state)?;
                Ok(0)
            }
            cmd @ SetVringKick => {
                self.ensure_configurable()?;
                let file = cmd.read()?;
                let index = self.check_queue_index(file.index)?;
                self.queues[index].kick = get_event_file(file.fd)?;
                self.invalidate_runtime();
                Ok(0)
            }
            cmd @ SetVringCall => {
                self.ensure_configurable()?;
                let file = cmd.read()?;
                let index = self.check_queue_index(file.index)?;
                self.queues[index].call = get_event_file(file.fd)?;
                self.invalidate_runtime();
                Ok(0)
            }
            cmd @ SetVringErr => {
                self.ensure_configurable()?;
                let file = cmd.read()?;
                let index = self.check_queue_index(file.index)?;
                self.queues[index].err = get_event_file(file.fd)?;
                self.invalidate_runtime();
                Ok(0)
            }
            cmd @ SetBackendFeatures => {
                self.ensure_configurable()?;
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

    pub(in crate::device::misc) fn build_runtime(&self) -> Result<VhostRuntime<NUM_QUEUES>> {
        self.ensure_owner()?;
        if !self.is_fully_configured() {
            return_errno_with_message!(Errno::EINVAL, "vhost device is not fully configured");
        }
        let owner = self.owner_vmar.as_ref().unwrap().clone();
        let memory = VhostMemorySpace::new(
            Arc::new(AlienOwnerMemory(owner)),
            self.memory_regions.clone(),
        )?;
        let allow_indirect = self.features & VIRTIO_RING_F_INDIRECT_DESC != 0;
        let queues = self
            .queues
            .iter()
            .map(|queue| {
                let addr = queue.addr.as_ref().ok_or_else(|| {
                    Error::with_message(Errno::EINVAL, "vhost vring address is not set")
                })?;
                validate_vring_addr(addr, queue.num)?;
                VhostVirtQueue::new(memory.clone(), queue, allow_indirect)
            })
            .collect::<Result<Vec<_>>>()?;
        let queues: [VhostVirtQueue; NUM_QUEUES] = queues.try_into().map_err(|_| {
            Error::with_message(Errno::EINVAL, "vhost queue count changed during setup")
        })?;
        Ok(VhostRuntime {
            generation: self.generation.load(Ordering::Acquire),
            current_generation: self.generation.clone(),
            queues,
        })
    }

    /// Checks that the caller owns this vhost session.
    pub(in crate::device::misc) fn check_owner(&self) -> Result<()> {
        self.ensure_owner()
    }

    /// Clears ownership and all common configuration.
    ///
    /// A backend must stop and join every worker that can access a
    /// [`VhostRuntime`] before calling this method. Owner reset is deliberately
    /// not handled by [`handle_ioctl`](Self::handle_ioctl), because only the
    /// backend can enforce that lifecycle ordering.
    pub(in crate::device::misc) fn reset_owner_after_quiesce(&mut self) {
        self.owner_vmar = None;
        self.features = 0;
        self.backend_features = 0;
        self.memory_regions.clear();
        for queue in &mut self.queues {
            *queue = VhostQueueState::default();
        }
        self.invalidate_runtime();
    }

    fn ensure_configurable(&self) -> Result<()> {
        self.ensure_owner()
    }

    fn ensure_owner(&self) -> Result<()> {
        let Some(owner) = self.owner_vmar.as_ref() else {
            return_errno_with_message!(Errno::EPERM, "vhost owner is not set");
        };
        let current = capture_owner()?;
        if !Arc::ptr_eq(owner, &current) {
            return_errno_with_message!(Errno::EPERM, "vhost caller is not the owner");
        }
        Ok(())
    }

    fn check_queue_index(&self, index: u32) -> Result<usize> {
        let index = usize::try_from(index)
            .map_err(|_| Error::with_message(Errno::EINVAL, "vhost queue index is invalid"))?;
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
/// stale; workers must stop using it and build a new one. Backends must also
/// quiesce workers before mutations that release owner resources.
pub(in crate::device::misc) struct VhostRuntime<const NUM_QUEUES: usize> {
    generation: u64,
    current_generation: Arc<AtomicU64>,
    queues: [VhostVirtQueue; NUM_QUEUES],
}

impl<const NUM_QUEUES: usize> VhostRuntime<NUM_QUEUES> {
    pub(in crate::device::misc) fn is_current(&self) -> bool {
        self.current_generation.load(Ordering::Acquire) == self.generation
    }

    pub(in crate::device::misc) fn queue_mut(
        &mut self,
        index: usize,
    ) -> Result<&mut VhostVirtQueue> {
        if !self.is_current() {
            return_errno_with_message!(Errno::EBUSY, "vhost runtime configuration is stale");
        }
        self.queues.get_mut(index).ok_or_else(|| {
            Error::with_message(Errno::EINVAL, "vhost runtime queue index is out of range")
        })
    }
}

fn capture_owner() -> Result<Arc<Vmar>> {
    let task =
        Task::current().ok_or_else(|| Error::with_message(Errno::ESRCH, "no current task"))?;
    let thread_local = task
        .as_thread_local()
        .ok_or_else(|| Error::with_message(Errno::EFAULT, "current task has no thread local"))?;
    thread_local
        .vmar()
        .borrow()
        .as_ref()
        .map(|vmar| vmar.clone_arc())
        .ok_or_else(|| Error::with_message(Errno::ESRCH, "current task has no VMAR"))
}

fn read_memory_regions(raw_ioctl: RawIoctl, memory: VhostMemory) -> Result<Vec<VhostMemoryRegion>> {
    let count = usize::try_from(memory.nregions)
        .map_err(|_| Error::with_message(Errno::EINVAL, "vhost memory region count is invalid"))?;
    if count > VHOST_MAX_MEMORY_REGIONS {
        return_errno_with_message!(Errno::EINVAL, "vhost memory region count is invalid");
    }
    let base = raw_ioctl
        .arg()
        .checked_add(size_of::<VhostMemory>())
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "vhost memory table address overflow"))?;
    let task =
        Task::current().ok_or_else(|| Error::with_message(Errno::ESRCH, "no current task"))?;
    let thread_local = task
        .as_thread_local()
        .ok_or_else(|| Error::with_message(Errno::EFAULT, "current task has no thread local"))?;
    let userspace = CurrentUserSpace::new(thread_local);
    let mut regions = Vec::with_capacity(count);
    for index in 0..count {
        let address = base
            .checked_add(
                index
                    .checked_mul(size_of::<VhostMemoryRegion>())
                    .ok_or_else(|| {
                        Error::with_message(Errno::EINVAL, "vhost memory table address overflow")
                    })?,
            )
            .ok_or_else(|| {
                Error::with_message(Errno::EINVAL, "vhost memory table address overflow")
            })?;
        regions.push(userspace.read_val(address)?);
    }
    Ok(regions)
}

fn get_event_file(fd: RawFileDesc) -> Result<Option<Arc<KernelEventFile>>> {
    if fd == -1 {
        return Ok(None);
    }
    let fd = FileDesc::try_from(fd)?;
    let task =
        Task::current().ok_or_else(|| Error::with_message(Errno::ESRCH, "no current task"))?;
    let thread_local = task
        .as_thread_local()
        .ok_or_else(|| Error::with_message(Errno::EFAULT, "current task has no thread local"))?;
    let mut file_table = thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, fd).into_owned();
    KernelEventFile::from_file(file).map(Some)
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
    validate_owner_range(addr.desc_user_addr, 0)?;
    validate_owner_range(addr.avail_user_addr, 0)?;
    validate_owner_range(addr.used_user_addr, 0)?;

    // Linux permits setting addresses before the queue size. The complete
    // ranges are validated when a size is available and again at activation.
    if num == 0 {
        return Ok(());
    }
    let num = usize::try_from(num)
        .map_err(|_| Error::with_message(Errno::EINVAL, "vhost vring size is invalid"))?;
    let desc_len = descriptor_offset(num).ok_or_else(|| {
        Error::with_message(Errno::EINVAL, "vhost descriptor table size overflow")
    })?;
    let avail_len = avail_entry_offset(num)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "vhost available ring size overflow"))?;
    let used_len = used_entry_offset(num)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "vhost used ring size overflow"))?;
    validate_owner_range(addr.desc_user_addr, desc_len)?;
    validate_owner_range(addr.avail_user_addr, avail_len)?;
    validate_owner_range(addr.used_user_addr, used_len)?;
    Ok(())
}

fn validate_owner_range(base: u64, len: usize) -> Result<usize> {
    let base = usize::try_from(base)
        .map_err(|_| Error::with_message(Errno::EINVAL, "vhost owner address is invalid"))?;
    if base < VMAR_LOWEST_ADDR
        || VMAR_CAP_ADDR
            .checked_sub(base)
            .is_none_or(|remaining| remaining < len)
    {
        return_errno_with_message!(Errno::EINVAL, "vhost owner address range is invalid");
    }
    Ok(base)
}

fn validate_memory_regions(mut regions: Vec<VhostMemoryRegion>) -> Result<Vec<VhostMemoryRegion>> {
    if regions.len() > VHOST_MAX_MEMORY_REGIONS {
        return_errno_with_message!(Errno::EINVAL, "vhost memory region count is invalid");
    }
    for region in &regions {
        if region.memory_size == 0 {
            return_errno_with_message!(Errno::EINVAL, "vhost memory region size is zero");
        }
        if region.flags_padding != 0 {
            return_errno_with_message!(Errno::EINVAL, "vhost memory region flags must be zero");
        }
        region
            .guest_phys_addr
            .checked_add(region.memory_size)
            .ok_or_else(|| {
                Error::with_message(Errno::EINVAL, "vhost guest memory range overflow")
            })?;
        validate_owner_range(
            region.userspace_addr,
            usize::try_from(region.memory_size).map_err(|_| {
                Error::with_message(Errno::EINVAL, "vhost memory region is too large")
            })?,
        )?;
    }
    regions.sort_unstable_by_key(|region| region.guest_phys_addr);
    for pair in regions.windows(2) {
        if pair[0].guest_phys_addr + pair[0].memory_size > pair[1].guest_phys_addr {
            return_errno_with_message!(Errno::EINVAL, "vhost guest memory regions overlap");
        }
    }
    Ok(regions)
}
