// SPDX-License-Identifier: MPL-2.0

use core::{cmp::min, fmt::Display};

use super::c_types::{
    CqRingOffsets, IORING_OFF_CQ_RING, IORING_OFF_SQ_RING, IORING_OFF_SQES, IoRingMeta,
    IoUringCqe, IoUringFeatures, IoUringParams, IoUringSetupFlags, IoUringSqe, MAX_CQ_ENTRIES,
    MAX_SQ_ENTRIES, SqRingOffsets,
};
use crate::{
    events::IoEvents,
    fs::{
        file::{AccessMode, CreationFlags, FileLike, Mappable, file_table::FdFlags},
        pseudofs::AnonInodeFs,
        vfs::path::Path,
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
    vm::page_cache::{Vmo, VmoOptions},
};

/// Owns the file-facing state and shared rings for one `io_uring` instance.
pub struct IoUringContext {
    ring_region: RingRegion,
    sqes_region: SqeRegion,
    pseudo_path: Path,
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
            ring_region,
            sqes_region,
            pseudo_path,
        });

        let ring_meta = IoRingMeta::new(config.sq_entries, config.cq_entries);
        context.ring_region.write_meta(&ring_meta)?;

        Ok(context)
    }

    fn mmap_region(&self, offset: usize) -> Option<(Arc<Vmo>, usize)> {
        self.ring_region
            .mmap_region(offset)
            .or_else(|| self.sqes_region.mmap_region(offset))
    }
}

impl Pollable for IoUringContext {
    fn poll(&self, _mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        IoEvents::empty()
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
                writeln!(f, "SqThread:\t{}", -1)?;
                writeln!(f, "SqThreadCpu:\t{}", -1)?;
                writeln!(f, "UserFiles:\t{}", 0)?;
                writeln!(f, "UserBufs:\t{}", 0)
            }
        }

        Box::new(FdInfo {
            ring: self,
            fd_flags,
        })
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
