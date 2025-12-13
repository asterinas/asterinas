// SPDX-License-Identifier: MPL-2.0

//! Virtual Memory Address Regions (VMARs).

mod vm_mapping;

// Utility modules.
mod cursor_utils;
mod interval_set;
mod vm_allocator;

// Implementation modules.
mod access_remote;
mod fork;
mod map;
mod page_fault;
mod protect;
mod query;
mod remap;
mod unmap;

use core::{array, ops::Range};

use align_ext::AlignExt;
use aster_util::per_cpu_counter::PerCpuCounter;
use ostd::{
    cpu::CpuId,
    mm::{MAX_USERSPACE_VADDR, VmSpace},
};

use self::{
    cursor_utils::{
        find_next_mapped, find_next_unmappable, propagate_if_needed, split_and_insert_rest,
    },
    interval_set::{Interval, IntervalSet},
    vm_allocator::PerCpuAllocator,
    vm_mapping::{MappedMemory, MappedVmo, VmMapping},
};
use crate::{
    prelude::*,
    process::{Process, ProcessVm, ResourceType},
};

/// Virtual Memory Address Regions (VMARs) are a type of capability that manages
/// user address spaces.
pub struct Vmar {
    /// The allocator for map/unmap operations.
    allocator: PerCpuAllocator,
    /// The attached `VmSpace`.
    vm_space: Arc<VmSpace<PerPtMeta>>,
    /// The RSS counters.
    rss_counters: [PerCpuCounter; NUM_RSS_COUNTERS],
    /// The process VM.
    process_vm: ProcessVm,
}

impl Vmar {
    /// Creates a new VMAR.
    pub fn new(process_vm: ProcessVm) -> Arc<Self> {
        let allocator = PerCpuAllocator::new().unwrap();
        let vm_space = VmSpace::<PerPtMeta>::new();
        let rss_counters = array::from_fn(|_| PerCpuCounter::new());
        Arc::new(Vmar {
            allocator,
            vm_space: Arc::new(vm_space),
            rss_counters,
            process_vm,
        })
    }

    /// Returns the current RSS count for the given RSS type.
    pub fn get_rss_counter(&self, rss_type: RssType) -> usize {
        self.rss_counters[rss_type as usize].sum_all_cpus()
    }

    /// Returns the total size of the mappings in bytes.
    pub fn get_mappings_total_size(&self) -> usize {
        self.inner.read().total_vm
    }

    fn add_rss_counter(&self, rss_type: RssType, val: isize) {
        // There are races but updating a remote counter won't cause any problems.
        let cpu_id = CpuId::current_racy();
        self.rss_counters[rss_type as usize].add_on_cpu(cpu_id, val);
    }
}

impl Vmar {
    /// Returns the attached `VmSpace`.
    pub fn vm_space(&self) -> &Arc<VmSpace<PerPtMeta>> {
        &self.vm_space
    }

    /// Returns the attached `ProcessVm`.
    pub fn process_vm(&self) -> &ProcessVm {
        &self.process_vm
    }
}

struct PerPtMeta {
    inner: IntervalSet<Vaddr, PteRangeMeta>,
}

type VmarCursor<'a> = CursorMut<'a, PerPtMeta>;

enum PteRangeMeta {
    ChildPt(Range<Vaddr>),
    VmMapping(VmMapping),
}

impl Interval<Vaddr> for PteRangeMeta {
    fn range(&self) -> Range<Vaddr> {
        match self {
            PteRangeMeta::ChildPt(range) => range.clone(),
            PteRangeMeta::VmMapping(vm_mapping) => vm_mapping.range(),
        }
    }
}

ostd::check_aux_pt_meta_layout!(PerPtMeta);
impl AuxPageTableMeta for PerPtMeta {
    fn new_root_page_table() -> Self {
        PerPtMeta {
            inner: IntervalSet::new(),
        }
    }

    fn alloc_child_page_table(&mut self, va: Vaddr, level: ostd::mm::PagingLevel) -> Self {
        todo!()
    }
}

impl PerPtMeta {
    const fn new() -> Self {
        Self {
            inner: IntervalSet::new(),
        }
    }

    /// Returns `Ok` if the calling process may expand its mapped
    /// memory by the passed size.
    fn check_extra_size_fits_rlimit(&self, expand_size: usize) -> Result<()> {
        let Some(process) = Process::current() else {
            // When building a `Process`, the kernel task needs to build
            // some `VmMapping`s, in which case this branch is reachable.
            return Ok(());
        };

        let rlimt_as = process
            .resource_limits()
            .get_rlimit(ResourceType::RLIMIT_AS)
            .get_cur();

        let new_total_vm = self
            .total_vm
            .checked_add(expand_size)
            .ok_or(Errno::ENOMEM)?;
        if new_total_vm > rlimt_as as usize {
            return_errno_with_message!(Errno::ENOMEM, "address space limit overflow");
        }
        Ok(())
    }

    /// Checks whether `addr..addr + size` is covered by a single `VmMapping`,
    /// and returns the address of the single `VmMapping` if successful.
    fn check_lies_in_single_mapping(&self, addr: Vaddr, size: usize) -> Result<Vaddr> {
        if let Some(vm_mapping) = self
            .inner
            .find_one(&addr)
            .filter(|vm_mapping| vm_mapping.map_end() - addr >= size)
        {
            Ok(vm_mapping.map_to_addr())
        } else {
            return_errno_with_message!(Errno::EFAULT, "the range must lie in a single mapping");
        }
    }

    /// Inserts a `VmMapping` into the `Vmar`, without attempting to merge with
    /// neighboring mappings.
    ///
    /// The caller must ensure that the given `VmMapping` is not mergeable with
    /// any neighboring mappings.
    ///
    /// Make sure the insertion doesn't exceed address space limit.
    fn insert_without_try_merge(&mut self, vm_mapping: VmMapping) {
        self.inner.insert(vm_mapping);
    }

    /// Inserts a `VmMapping` into the `Vmar`, and attempts to merge it with
    /// neighboring mappings.
    ///
    /// This method will try to merge the `VmMapping` with neighboring mappings
    /// that are adjacent and compatible, in order to reduce fragmentation.
    ///
    /// Make sure the insertion doesn't exceed address space limit.
    fn insert_try_merge(&mut self, vm_mapping: VmMapping) {
        let mut vm_mapping = vm_mapping;
        let addr = vm_mapping.map_to_addr();

        if let Some(prev) = self.inner.find_prev(&addr) {
            let (new_mapping, to_remove) = vm_mapping.try_merge_with(prev);
            vm_mapping = new_mapping;
            if let Some(addr) = to_remove {
                self.inner.remove(&addr);
            }
        }

        if let Some(next) = self.inner.find_next(&addr) {
            let (new_mapping, to_remove) = vm_mapping.try_merge_with(next);
            vm_mapping = new_mapping;
            if let Some(addr) = to_remove {
                self.inner.remove(&addr);
            }
        }

        self.inner.insert(vm_mapping);
    }

    /// Removes a `VmMapping` based on the provided key from the `Vmar`.
    fn remove(&mut self, key: &Vaddr) -> Option<VmMapping> {
        let vm_mapping = self.inner.remove(key)?;
        Some(vm_mapping)
    }

    /// Finds a set of [`VmMapping`]s that intersect with the provided range.
    fn query(&self, range: &Range<Vaddr>) -> impl Iterator<Item = &VmMapping> {
        self.inner.find(range)
    }

    /// Calculates the total amount of overlap between `VmMapping`s
    /// and the provided range.
    fn count_overlap_size(&self, range: Range<Vaddr>) -> usize {
        let mut sum_overlap_size = 0;
        for vm_mapping in self.inner.find(&range) {
            let vm_mapping_range = vm_mapping.range();
            let intersected_range = get_intersected_range(&range, &vm_mapping_range);
            sum_overlap_size += intersected_range.end - intersected_range.start;
        }
        sum_overlap_size
    }

    /// Splits and unmaps the found mapping if the new size is smaller.
    /// Enlarges the last mapping if the new size is larger.
    fn resize_mapping(
        &mut self,
        vm_space: &VmSpace,
        map_addr: Vaddr,
        old_size: usize,
        new_size: usize,
        rss_delta: &mut RssDelta,
    ) -> Result<()> {
        debug_assert_eq!(map_addr % PAGE_SIZE, 0);
        debug_assert_eq!(old_size % PAGE_SIZE, 0);
        debug_assert_eq!(new_size % PAGE_SIZE, 0);

        if new_size == 0 {
            return_errno_with_message!(Errno::EINVAL, "cannot resize a mapping to 0 size");
        }

        if new_size == old_size {
            return Ok(());
        }

        let old_map_end = map_addr.checked_add(old_size).ok_or(Errno::EINVAL)?;
        let new_map_end = map_addr.checked_add(new_size).ok_or(Errno::EINVAL)?;
        if !is_userspace_vaddr(new_map_end - 1) {
            return_errno_with_message!(Errno::EINVAL, "resize to an invalid new size");
        }

        if new_size < old_size {
            self.alloc_free_region_exact_truncate(
                vm_space,
                new_map_end,
                old_map_end - new_map_end,
                rss_delta,
            )?;
            return Ok(());
        }

        self.alloc_free_region_exact(old_map_end, new_map_end - old_map_end)?;

        let last_mapping = self.vm_mappings.find_one(&(old_map_end - 1)).unwrap();
        let last_mapping_addr = last_mapping.map_to_addr();
        debug_assert_eq!(last_mapping.map_end(), old_map_end);

        self.check_extra_size_fits_rlimit(new_map_end - old_map_end)?;
        let last_mapping = self.remove(&last_mapping_addr).unwrap();
        let last_mapping = last_mapping.enlarge(new_map_end - old_map_end);
        self.insert_try_merge(last_mapping);
        Ok(())
    }
}

pub const VMAR_LOWEST_ADDR: Vaddr = 0x001_0000; // 64 KiB is the Linux configurable default
const VMAR_CAP_ADDR: Vaddr = MAX_USERSPACE_VADDR;

/// Returns whether the input `vaddr` is a legal user space virtual address.
pub fn is_userspace_vaddr(vaddr: Vaddr) -> bool {
    (VMAR_LOWEST_ADDR..VMAR_CAP_ADDR).contains(&vaddr)
}

/// Returns the full user space virtual address range.
pub fn userspace_range() -> Range<Vaddr> {
    VMAR_LOWEST_ADDR..VMAR_CAP_ADDR
}

/// Determines whether two ranges are intersected.
/// returns false if one of the ranges has a length of 0
pub fn is_intersected(range1: &Range<usize>, range2: &Range<usize>) -> bool {
    range1.start.max(range2.start) < range1.end.min(range2.end)
}

/// Gets the intersection range of two ranges.
/// The two ranges should be ensured to be intersected.
pub fn get_intersected_range(range1: &Range<usize>, range2: &Range<usize>) -> Range<usize> {
    debug_assert!(is_intersected(range1, range2));
    range1.start.max(range2.start)..range1.end.min(range2.end)
}

/// The type representing categories of Resident Set Size (RSS).
///
/// See <https://github.com/torvalds/linux/blob/fac04efc5c793dccbd07e2d59af9f90b7fc0dca4/include/linux/mm_types_task.h#L26..L32>
#[repr(u32)]
#[expect(non_camel_case_types)]
#[derive(Debug, Clone, Copy, TryFromInt)]
pub enum RssType {
    RSS_FILEPAGES = 0,
    RSS_ANONPAGES = 1,
}

const NUM_RSS_COUNTERS: usize = 2;

pub(super) struct RssDelta<'a> {
    delta: [isize; NUM_RSS_COUNTERS],
    operated_vmar: &'a Vmar,
}

impl<'a> RssDelta<'a> {
    pub(self) fn new(operated_vmar: &'a Vmar) -> Self {
        Self {
            delta: [0; NUM_RSS_COUNTERS],
            operated_vmar,
        }
    }

    pub(self) fn add(&mut self, rss_type: RssType, increment: isize) {
        self.delta[rss_type as usize] += increment;
    }

    fn get(&self, rss_type: RssType) -> isize {
        self.delta[rss_type as usize]
    }
}

impl Drop for RssDelta<'_> {
    fn drop(&mut self) {
        for i in 0..NUM_RSS_COUNTERS {
            let rss_type = RssType::try_from(i as u32).unwrap();
            let delta = self.get(rss_type);
            self.operated_vmar.add_rss_counter(rss_type, delta);
        }
    }
}
