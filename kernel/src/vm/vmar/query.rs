// SPDX-License-Identifier: MPL-2.0

use core::{array, num::NonZeroUsize, ops::Range};

use align_ext::AlignExt;
use aster_util::per_cpu_counter::PerCpuCounter;
use ostd::{
    cpu::CpuId,
    mm::{
        CachePolicy, HasSize, MAX_USERSPACE_VADDR, PageFlags, UFrame, VmSpace,
        io_util::HasVmReaderWriter,
        page_size, page_size_at,
        tlb::TlbFlushOp,
        vm_space::{Cursor, CursorMut, VmQueriedItem},
    },
    sync::RwMutexReadGuard,
    task::{atomic_mode::AsAtomicModeGuard, disable_preempt},
};

use super::{
    Interval, IntervalSet, MappedMemory, MappedVmo, PerCpuAllocator, RssDelta, RssType, VmMapping,
    Vmar, find_next_mapped, find_next_unmappable, propagate_if_needed, split_and_insert_rest,
};
use crate::{
    fs::{file_handle::Mappable, ramfs::memfd::MemfdInode},
    prelude::*,
    process::{Process, ProcessVm, ResourceType, posix_thread::last_tid},
    thread::exception::PageFaultInfo,
    vm::{self, perms::VmPerms, vmar::cursor_utils::unmap_count_rss, vmo::Vmo},
};

impl Vmar {
    /// Finds all the mapped regions that intersect with the specified range.
    pub fn query<'a, G: AsAtomicModeGuard>(
        &self,
        guard: &'a G,
        range: Range<usize>,
    ) -> VmarQueryGuard<'a> {
        VmarQueryGuard {
            cursor: self.vm_space.cursor(guard, &range).unwrap(),
        }
    }
}

/// A guard that allows querying a [`Vmar`] for its mappings.
pub struct VmarQueryGuard<'a> {
    cursor: Cursor<'a, PerPtMeta>,
}

impl VmarQueryGuard<'_> {
    /// Returns an iterator over the [`VmMapping`]s that intersect with the
    /// provided range when calling [`Vmar::query`].
    pub fn iter(&self) -> impl Iterator<Item = &VmMapping> {
        self.vmar.query(&self.range)
    }
}
