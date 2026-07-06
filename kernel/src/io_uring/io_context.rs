// SPDX-License-Identifier: MPL-2.0

use core::{
    cmp::min,
    fmt::Display,
    sync::atomic::{Ordering, fence},
};

use ostd::sync::WaitQueue;

use super::{
    c_types::{
        CqRingOffsets, IORING_OFF_CQ_RING, IORING_OFF_SQ_RING, IORING_OFF_SQES, IoRingMeta,
        IoUringCqe, IoUringFeatures, IoUringParams, IoUringSetupFlags, IoUringSqe,
        MAX_CQ_ENTRIES, MAX_SQ_ENTRIES, SqRingOffsets,
    },
    ops,
    utils::Completion,
};
use crate::{
    events::IoEvents,
    fs::{
        file::{AccessMode, CreationFlags, FileLike, Mappable, file_table::FdFlags},
        pseudofs::AnonInodeFs,
        vfs::path::Path,
    },
    prelude::*,
    process::signal::{PollHandle, Pollable, Pollee},
    vm::page_cache::{Vmo, VmoOptions},
};

/// Owns the file-facing state, rings, and wait state for one `io_uring` instance.
pub struct IoUringContext {
    sq_ring: Mutex<SqRing>,
    cq_ring: Mutex<CqRing>,
    ring_region: RingRegion,
    sqes_region: SqeRegion,

    sq_wait_queue: WaitQueue,
    pollee: Pollee,
    pseudo_path: Path,
}

struct SqRing {
    entries: u32,
    // Mirrors the kernel-owned SQ head. Userspace observes this value only
    // after `commit_head` publishes it into shared ring memory.
    cached_head: u32,
}

struct CqRing {
    entries: u32,
    // Mirrors the kernel-owned CQ tail. Userspace observes this value only
    // after `commit_tail` publishes it into shared ring memory.
    cached_tail: u32,
    // Temporarily holds completions when userspace has not freed CQ slots.
    // Draining is retried on submit, completion, wait, and poll paths.
    overflow_completions: VecDeque<Completion>,
}

/// Backs the shared SQ/CQ ring mapping visible to userspace.
///
/// The region contains the fixed `IoRingMeta` header first, followed by the
/// CQE array (`cqes`) and then the SQ array.
struct RingRegion {
    size: usize,
    sq_array_offset: usize,
    region: Arc<Vmo>,
}

struct SqeRegion {
    size: usize,
    region: Arc<Vmo>,
}

impl IoUringContext {
    pub fn new(config: &IoUringSetupConfig, _ctx: &Context) -> Result<Arc<Self>> {
        let ring_region = RingRegion::new(config.ring_size, config.sq_array_offset)?;
        let sqes_region = SqeRegion::new(config.sqes_size)?;
        let pseudo_path = AnonInodeFs::new_path(|_| "anon_inode:[io_uring]".to_string());
        let context = Arc::new(Self {
            sq_ring: Mutex::new(SqRing::new(config)),
            cq_ring: Mutex::new(CqRing::new(config)),
            ring_region,
            sqes_region,
            sq_wait_queue: WaitQueue::new(),
            pollee: Pollee::new(),
            pseudo_path,
        });

        let ring_meta = IoRingMeta::new(config.sq_entries, config.cq_entries);
        context.ring_region.write_meta(&ring_meta)?;

        Ok(context)
    }

    pub fn submit_sqes(&self, to_submit: u32) -> Result<u32> {
        if to_submit == 0 {
            return Ok(0);
        }

        let mut sq_ring = self.sq_ring.lock();
        let pending = sq_ring.pending_entry_count(&self.ring_region)?;
        let mut consumed = 0u32;
        let mut submitted = 0u32;

        // The SQ array contains user-provided indexes into the SQE table. The
        // kernel consumes them in ring order and advances its cached SQ head.
        for _ in 0..min(to_submit, pending) {
            if let Some(sqe) = self.get_sqe(&sq_ring)? {
                self.submit_sqe(sqe)?;
                sq_ring.cached_head = sq_ring.cached_head.wrapping_add(1);
                consumed += 1;
                submitted += 1;
            } else {
                self.ring_region.increment_sq_dropped()?;
                sq_ring.cached_head = sq_ring.cached_head.wrapping_add(1);
                consumed += 1;
                break;
            };
        }

        if consumed > 0 {
            sq_ring.commit_head(&self.ring_region, &self.sq_wait_queue)?;
        }

        Ok(submitted)
    }

    pub fn wait_for_completions(&self, min_complete: u32) -> Result<()> {
        self.wait_events(IoEvents::IN, None, || {
            let mut cq_ring = self.cq_ring.lock();
            cq_ring.drain_overflow_completions(&self.ring_region, &self.pollee)?;
            if cq_ring.pending_entry_count(&self.ring_region)? >= min_complete {
                Ok(())
            } else {
                Err(Error::with_message(
                    Errno::EAGAIN,
                    "not enough io_uring completions",
                ))
            }
        })
    }

    pub(super) fn post_completion(&self, completion: Completion) -> Result<()> {
        let mut cq_ring = self.cq_ring.lock();
        cq_ring.post_completion(&self.ring_region, &self.pollee, completion)
    }

    fn get_sqe(&self, sq_ring: &SqRing) -> Result<Option<IoUringSqe>> {
        let sq_index = sq_ring.read_array_entry(&self.ring_region)?;
        if sq_index >= sq_ring.entry_count() {
            return Ok(None);
        }

        Ok(Some(self.sqes_region.read_sqe(sq_index)?))
    }

    fn submit_sqe(&self, sqe: IoUringSqe) -> Result<()> {
        match ops::build_op_request(self, &sqe) {
            Ok(request) => {
                if let Some(completion) = request.try_execute_nonblock() {
                    self.post_completion(completion)?;
                } else {
                    self.post_completion(request.execute())?;
                }
            }
            Err(err) => self.post_completion(Completion::with_error(sqe.user_data, err))?,
        }

        Ok(())
    }

    fn mmap_region(&self, offset: usize) -> Option<(Arc<Vmo>, usize)> {
        self.ring_region
            .mmap_region(offset)
            .or_else(|| self.sqes_region.mmap_region(offset))
    }

    fn check_io_events(&self) -> IoEvents {
        let mut cq_ring = self.cq_ring.lock();
        cq_ring.check_io_events(&self.ring_region, &self.pollee)
    }

    fn write_fdinfo(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let sq_ring = self.sq_ring.lock();
        let cq_ring = self.cq_ring.lock();
        let sq_head = self
            .ring_region
            .read_sq_head()
            .unwrap_or(sq_ring.cached_head);
        let sq_tail = self
            .ring_region
            .read_sq_tail()
            .unwrap_or(sq_ring.cached_head);
        let cq_head = self.ring_region.read_cq_head().unwrap_or(0);
        let cq_tail = self
            .ring_region
            .read_cq_tail()
            .unwrap_or(cq_ring.cached_tail);
        let sqes = sq_tail.wrapping_sub(sq_head).min(sq_ring.entry_count());
        let cqes = cq_tail.wrapping_sub(cq_head).min(cq_ring.entry_count());

        writeln!(
            f,
            "SqMask:\t0x{:x}",
            sq_ring.entry_count().saturating_sub(1)
        )?;
        writeln!(f, "SqHead:\t{}", sq_head)?;
        writeln!(f, "SqTail:\t{}", sq_tail)?;
        writeln!(f, "CachedSqHead:\t{}", sq_ring.cached_head)?;
        writeln!(
            f,
            "CqMask:\t0x{:x}",
            cq_ring.entry_count().saturating_sub(1)
        )?;
        writeln!(f, "CqHead:\t{}", cq_head)?;
        writeln!(f, "CqTail:\t{}", cq_tail)?;
        writeln!(f, "CachedCqTail:\t{}", cq_ring.cached_tail)?;
        writeln!(f, "SQEs:\t{}", sqes)?;
        writeln!(f, "CQEs:\t{}", cqes)?;
        writeln!(f, "SqThread:\t{}", -1)?;
        writeln!(f, "SqThreadCpu:\t{}", -1)?;
        writeln!(f, "SqTotalTime:\t{}", 0)?;
        writeln!(f, "SqWorkTime:\t{}", 0)?;
        writeln!(f, "UserFiles:\t{}", 0)?;
        writeln!(f, "UserBufs:\t{}", 0)?;
        writeln!(f, "PollList:")?;
        writeln!(f, "CqOverflowList:")
    }
}

impl Pollable for IoUringContext {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.pollee.invalidate();
        self.pollee
            .poll_with(mask, poller, || self.check_io_events())
    }
}

impl FileLike for IoUringContext {
    fn mappable(&self) -> Result<Mappable> {
        let (mappable, _) = self.mappable_at(IORING_OFF_SQ_RING)?;
        Ok(mappable)
    }

    fn mappable_at(&self, offset: usize) -> Result<(Mappable, usize)> {
        let Some((region, region_offset)) = self.mmap_region(offset) else {
            return_errno_with_message!(Errno::EINVAL, "invalid io_uring mmap offset");
        };

        Ok((Mappable::Vmo(region), region_offset))
    }

    fn access_mode(&self) -> AccessMode {
        AccessMode::O_RDWR
    }

    fn path(&self) -> &Path {
        &self.pseudo_path
    }

    fn dump_proc_fdinfo(self: Arc<Self>, fd_flags: FdFlags) -> Box<dyn Display> {
        struct FdInfo {
            ring: Arc<IoUringContext>,
            fd_flags: FdFlags,
        }

        impl Display for FdInfo {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                let mut flags = self.ring.status_flags().bits() | self.ring.access_mode() as u32;
                if self.fd_flags.contains(FdFlags::CLOEXEC) {
                    flags |= CreationFlags::O_CLOEXEC.bits();
                }

                writeln!(f, "pos:\t{}", 0)?;
                writeln!(f, "flags:\t0{:o}", flags)?;
                writeln!(f, "mnt_id:\t{}", AnonInodeFs::mount_node().id())?;
                writeln!(f, "ino:\t{}", AnonInodeFs::shared_inode().ino())?;
                self.ring.write_fdinfo(f)
            }
        }

        Box::new(FdInfo {
            ring: self,
            fd_flags,
        })
    }
}

impl SqRing {
    fn new(config: &IoUringSetupConfig) -> Self {
        Self {
            entries: config.sq_entries,
            cached_head: 0,
        }
    }

    const fn entry_count(&self) -> u32 {
        self.entries
    }

    fn read_array_entry(&self, ring_region: &RingRegion) -> Result<u32> {
        let array_index = self.cached_head & (self.entries - 1);
        ring_region.read_sq_array_entry(array_index)
    }

    fn pending_entry_count(&self, ring_region: &RingRegion) -> Result<u32> {
        let sq_tail = ring_region.read_sq_tail()?;
        // Userspace publishes SQ array/SQE contents before updating `sq.tail`;
        // SQ array and SQE reads must stay after observing this tail value.
        fence(Ordering::Acquire);
        Ok(sq_tail.wrapping_sub(self.cached_head).min(self.entries))
    }

    fn commit_head(&self, ring_region: &RingRegion, sq_wait_queue: &WaitQueue) -> Result<()> {
        // The kernel must finish consuming SQEs before publishing `sq.head`;
        // userspace can reuse those SQ slots after observing the new head.
        fence(Ordering::Release);
        ring_region.write_sq_head(self.cached_head)?;
        sq_wait_queue.wake_all();
        Ok(())
    }
}

impl CqRing {
    fn new(config: &IoUringSetupConfig) -> Self {
        Self {
            entries: config.cq_entries,
            cached_tail: 0,
            overflow_completions: VecDeque::new(),
        }
    }

    const fn entry_count(&self) -> u32 {
        self.entries
    }

    fn post_completion(
        &mut self,
        ring_region: &RingRegion,
        pollee: &Pollee,
        completion: Completion,
    ) -> Result<()> {
        if self.try_post_cqe(ring_region, pollee, &completion)? {
            return Ok(());
        }

        self.overflow_completions.push_back(completion);
        self.increment_overflow(ring_region)
    }

    fn drain_overflow_completions(
        &mut self,
        ring_region: &RingRegion,
        pollee: &Pollee,
    ) -> Result<()> {
        while self.has_avaliable_entries(ring_region)? {
            let Some(completion) = self.overflow_completions.pop_front() else {
                break;
            };
            self.commit_tail(ring_region, pollee, &completion)?;
        }

        Ok(())
    }

    fn pending_entry_count(&self, ring_region: &RingRegion) -> Result<u32> {
        let cq_head = ring_region.read_cq_head()?;
        // Userspace advances `cq.head` after consuming CQEs; CQ space checks and
        // later CQE writes must stay after observing reclaimed CQ slots.
        fence(Ordering::Acquire);
        Ok(self.cached_tail.wrapping_sub(cq_head).min(self.entries))
    }

    fn has_pending_entries(&self, ring_region: &RingRegion) -> Result<bool> {
        Ok(self.pending_entry_count(ring_region)? != 0)
    }

    fn check_io_events(&mut self, ring_region: &RingRegion, pollee: &Pollee) -> IoEvents {
        let _ = self.drain_overflow_completions(ring_region, pollee);

        let mut events = IoEvents::empty();
        if self.has_pending_entries(ring_region).is_ok_and(|has| has) {
            events |= IoEvents::IN;
        }
        if self.has_avaliable_entries(ring_region).is_ok_and(|has| has) {
            events |= IoEvents::OUT;
        }
        events
    }

    fn try_post_cqe(
        &mut self,
        ring_region: &RingRegion,
        pollee: &Pollee,
        completion: &Completion,
    ) -> Result<bool> {
        self.drain_overflow_completions(ring_region, pollee)?;
        if !self.has_avaliable_entries(ring_region)? {
            return Ok(false);
        }

        self.commit_tail(ring_region, pollee, completion)?;
        Ok(true)
    }

    fn commit_tail(
        &mut self,
        ring_region: &RingRegion,
        pollee: &Pollee,
        completion: &Completion,
    ) -> Result<()> {
        let cq_tail = self.cached_tail;
        let cq_index = cq_tail & (self.entries - 1);
        let cqe = IoUringCqe {
            user_data: completion.user_data,
            res: completion.res,
            flags: completion.flags,
        };

        ring_region.write_cqe(cq_index, &cqe)?;
        let new_cq_tail = cq_tail.wrapping_add(1);
        // The CQE payload must be visible before publishing `cq.tail`;
        // userspace treats the tail update as permission to read the CQE.
        fence(Ordering::Release);
        ring_region.write_cq_tail(new_cq_tail)?;
        self.cached_tail = new_cq_tail;
        // Poll waiters are notified only after the tail is visible.
        pollee.notify(IoEvents::IN);

        Ok(())
    }

    fn has_avaliable_entries(&self, ring_region: &RingRegion) -> Result<bool> {
        Ok(self.pending_entry_count(ring_region)? < self.entries)
    }

    fn increment_overflow(&self, ring_region: &RingRegion) -> Result<()> {
        let overflow = ring_region.read_cq_overflow()?;
        ring_region.write_cq_overflow(overflow.wrapping_add(1))
    }
}

impl RingRegion {
    fn new(size: usize, sq_array_offset: usize) -> Result<Self> {
        Ok(Self {
            size,
            sq_array_offset,
            region: VmoOptions::new(size).alloc()?,
        })
    }

    fn write_meta(&self, meta: &IoRingMeta) -> Result<()> {
        self.write_val(0, meta)
    }

    fn read_sq_head(&self) -> Result<u32> {
        self.read_val(core::mem::offset_of!(IoRingMeta, sq_head))
    }

    fn write_sq_head(&self, sq_head: u32) -> Result<()> {
        self.write_val(core::mem::offset_of!(IoRingMeta, sq_head), &sq_head)
    }

    fn read_sq_tail(&self) -> Result<u32> {
        self.read_val(core::mem::offset_of!(IoRingMeta, sq_tail))
    }

    fn read_cq_head(&self) -> Result<u32> {
        self.read_val(core::mem::offset_of!(IoRingMeta, cq_head))
    }

    fn read_cq_tail(&self) -> Result<u32> {
        self.read_val(core::mem::offset_of!(IoRingMeta, cq_tail))
    }

    fn write_cq_tail(&self, cq_tail: u32) -> Result<()> {
        self.write_val(core::mem::offset_of!(IoRingMeta, cq_tail), &cq_tail)
    }

    fn increment_sq_dropped(&self) -> Result<()> {
        let dropped: u32 = self.read_val(core::mem::offset_of!(IoRingMeta, sq_dropped))?;
        self.write_val(
            core::mem::offset_of!(IoRingMeta, sq_dropped),
            &dropped.wrapping_add(1),
        )
    }

    fn read_cq_overflow(&self) -> Result<u32> {
        self.read_val(core::mem::offset_of!(IoRingMeta, cq_overflow))
    }

    fn write_cq_overflow(&self, cq_overflow: u32) -> Result<()> {
        self.write_val(core::mem::offset_of!(IoRingMeta, cq_overflow), &cq_overflow)
    }

    fn read_sq_array_entry(&self, array_index: u32) -> Result<u32> {
        let array_offset = (array_index as usize)
            .checked_mul(size_of::<u32>())
            .and_then(|offset| self.sq_array_offset.checked_add(offset))
            .ok_or_else(|| {
                Error::with_message(Errno::EOVERFLOW, "the SQ array offset overflows")
            })?;

        self.read_val(array_offset)
    }

    fn write_cqe(&self, cq_index: u32, cqe: &IoUringCqe) -> Result<()> {
        let cqe_offset = (cq_index as usize)
            .checked_mul(size_of::<IoUringCqe>())
            .and_then(|offset| size_of::<IoRingMeta>().checked_add(offset))
            .ok_or_else(|| Error::with_message(Errno::EOVERFLOW, "the CQE offset overflows"))?;

        self.write_val(cqe_offset, cqe)
    }

    fn read_val<T: FromZeros + Pod>(&self, offset: usize) -> Result<T> {
        let mut value = T::new_zeroed();
        let mut writer = VmWriter::from(value.as_mut_bytes()).to_fallible();
        self.region.read(offset, &mut writer)?;
        Ok(value)
    }

    fn write_val<T: Pod>(&self, offset: usize, value: &T) -> Result<()> {
        let mut reader = VmReader::from(value.as_bytes()).to_fallible();
        self.region.write(offset, &mut reader)
    }

    fn mmap_region(&self, offset: usize) -> Option<(Arc<Vmo>, usize)> {
        if let Some(region_offset) = offset_in_region(offset, IORING_OFF_SQ_RING, self.size) {
            return Some((self.region.clone(), region_offset));
        }
        if let Some(region_offset) = offset_in_region(offset, IORING_OFF_CQ_RING, self.size) {
            return Some((self.region.clone(), region_offset));
        }

        None
    }
}

impl SqeRegion {
    fn new(size: usize) -> Result<Self> {
        Ok(Self {
            size,
            region: VmoOptions::new(size).alloc()?,
        })
    }

    fn read_sqe(&self, sq_index: u32) -> Result<IoUringSqe> {
        let offset = (sq_index as usize)
            .checked_mul(size_of::<IoUringSqe>())
            .ok_or_else(|| Error::with_message(Errno::EOVERFLOW, "the SQE offset overflows"))?;
        let mut sqe = IoUringSqe::new_zeroed();
        let mut writer = VmWriter::from(sqe.as_mut_bytes()).to_fallible();
        self.region.read(offset, &mut writer)?;
        Ok(sqe)
    }

    fn mmap_region(&self, offset: usize) -> Option<(Arc<Vmo>, usize)> {
        let region_offset = offset_in_region(offset, IORING_OFF_SQES, self.size)?;
        Some((self.region.clone(), region_offset))
    }
}

fn offset_in_region(offset: usize, base: usize, size: usize) -> Option<usize> {
    let region_offset = offset.checked_sub(base)?;
    (region_offset < size).then_some(region_offset)
}

pub struct IoUringSetupConfig {
    sq_entries: u32,
    cq_entries: u32,
    ring_size: usize,
    sq_array_offset: usize,
    sqes_size: usize,
}

impl IoUringSetupConfig {
    pub fn new(entries: u32, params: &IoUringParams) -> Result<Self> {
        if params.resv.iter().any(|reserved| *reserved != 0) {
            return_errno_with_message!(
                Errno::EINVAL,
                "io_uring_setup reserved fields are not zero"
            );
        }

        let flags = IoUringSetupFlags::from_user_bits(params.flags)?;
        let is_clamp = flags.contains(IoUringSetupFlags::CLAMP);
        let sq_entries = calculate_entries(entries, MAX_SQ_ENTRIES, is_clamp)?;
        let cq_entries = if flags.contains(IoUringSetupFlags::CQSIZE) {
            let cq_entries = calculate_entries(params.cq_entries, MAX_CQ_ENTRIES, is_clamp)?;
            if cq_entries < sq_entries {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "CQ ring must have at least as many entries as the SQ ring"
                );
            }
            cq_entries
        } else {
            sq_entries * 2
        };

        let sq_array_offset =
            size_of::<IoRingMeta>() + cq_entries as usize * size_of::<IoUringCqe>();
        let ring_size = sq_array_offset + sq_entries as usize * size_of::<u32>();
        let sqes_size = sq_entries as usize * size_of::<IoUringSqe>();

        Ok(Self {
            sq_entries,
            cq_entries,
            ring_size,
            sq_array_offset,
            sqes_size,
        })
    }

    pub fn write_params(&self, params: &mut IoUringParams) {
        params.sq_entries = self.sq_entries;
        params.cq_entries = self.cq_entries;
        let supported_features = IoUringFeatures::empty();
        params.features = supported_features.bits();
        params.wq_fd = 0;
        params.resv = [0; 3];

        params.sq_off = SqRingOffsets {
            head: core::mem::offset_of!(IoRingMeta, sq_head) as u32,
            tail: core::mem::offset_of!(IoRingMeta, sq_tail) as u32,
            ring_mask: core::mem::offset_of!(IoRingMeta, sq_ring_mask) as u32,
            ring_entries: core::mem::offset_of!(IoRingMeta, sq_ring_entries) as u32,
            flags: core::mem::offset_of!(IoRingMeta, sq_flags) as u32,
            dropped: core::mem::offset_of!(IoRingMeta, sq_dropped) as u32,
            array: self.sq_array_offset as u32,
            resv1: 0,
            user_addr: 0,
        };
        params.cq_off = CqRingOffsets {
            head: core::mem::offset_of!(IoRingMeta, cq_head) as u32,
            tail: core::mem::offset_of!(IoRingMeta, cq_tail) as u32,
            ring_mask: core::mem::offset_of!(IoRingMeta, cq_ring_mask) as u32,
            ring_entries: core::mem::offset_of!(IoRingMeta, cq_ring_entries) as u32,
            overflow: core::mem::offset_of!(IoRingMeta, cq_overflow) as u32,
            cqes: size_of::<IoRingMeta>() as u32,
            flags: core::mem::offset_of!(IoRingMeta, cq_flags) as u32,
            resv1: 0,
            user_addr: 0,
        };
    }
}

fn calculate_entries(entries: u32, max_entries: u32, is_clamp: bool) -> Result<u32> {
    if entries == 0 {
        return_errno_with_message!(Errno::EINVAL, "the ring entry count is zero");
    }

    let Some(rounded_entries) = entries.checked_next_power_of_two() else {
        return_errno_with_message!(Errno::EINVAL, "the ring entry count overflows");
    };
    if rounded_entries <= max_entries {
        return Ok(rounded_entries);
    }
    if is_clamp {
        Ok(max_entries)
    } else {
        return_errno_with_message!(Errno::EINVAL, "the ring entry count is too large");
    }
}
