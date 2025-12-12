// SPDX-License-Identifier: MPL-2.0

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
use ostd::{cpu::CpuId, mm::VmSpace};

use super::{
    VMAR_CAP_ADDR, VMAR_LOWEST_ADDR,
    interval_set::{Interval, IntervalSet},
    is_userspace_vaddr,
    util::{self, get_intersected_range},
    vm_mapping::{MappedMemory, MappedVmo, VmMapping},
};
use crate::{
    prelude::*,
    process::{Process, ProcessVm, ResourceType},
    vm::vmar::is_userspace_vaddr_range,
};

/// The VMAR (used to be Virtual Memory Address Region, but now an orphan
/// initialism).
///
/// A VMAR is the address space of a process.
pub struct Vmar {
    /// VMAR inner
    inner: RwMutex<VmarInner>,
    /// The attached `VmSpace`
    vm_space: Arc<VmSpace>,
    /// The RSS counters.
    rss_counters: [PerCpuCounter; NUM_RSS_COUNTERS],
    /// The process VM
    process_vm: ProcessVm,
}

impl Vmar {
    /// Creates a new VMAR.
    pub fn new(process_vm: ProcessVm) -> Arc<Self> {
        let inner = VmarInner::new();
        let vm_space = VmSpace::new();
        let rss_counters = array::from_fn(|_| PerCpuCounter::new());
        Arc::new(Vmar {
            inner: RwMutex::new(inner),
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

    /// Returns the attached `VmSpace`.
    pub fn vm_space(&self) -> &Arc<VmSpace> {
        &self.vm_space
    }

    /// Returns the attached `ProcessVm`.
    pub fn process_vm(&self) -> &ProcessVm {
        &self.process_vm
    }

    fn add_rss_counter(&self, rss_type: RssType, val: isize) {
        // There are races but updating a remote counter won't cause any problems.
        let cpu_id = CpuId::current_racy();
        self.rss_counters[rss_type as usize].add_on_cpu(cpu_id, val);
    }
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
    pub(super) fn new(operated_vmar: &'a Vmar) -> Self {
        Self {
            delta: [0; NUM_RSS_COUNTERS],
            operated_vmar,
        }
    }

    pub(super) fn add(&mut self, rss_type: RssType, increment: isize) {
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

struct VmarInner {
    /// The mapped pages and associated metadata.
    ///
    /// When inserting a `VmMapping` into this set, use `insert_try_merge` to
    /// auto-merge adjacent and compatible mappings, or `insert_without_try_merge`
    /// if the mapping is known not mergeable with any neighboring mappings.
    vm_mappings: IntervalSet<Vaddr, VmMapping>,
    /// The total mapped memory in bytes.
    total_vm: usize,
}

impl VmarInner {
    const fn new() -> Self {
        Self {
            vm_mappings: IntervalSet::new(),
            total_vm: 0,
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
            .vm_mappings
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
        self.total_vm += vm_mapping.map_size();
        self.vm_mappings.insert(vm_mapping);
    }

    /// Inserts a `VmMapping` into the `Vmar`, and attempts to merge it with
    /// neighboring mappings.
    ///
    /// This method will try to merge the `VmMapping` with neighboring mappings
    /// that are adjacent and compatible, in order to reduce fragmentation.
    ///
    /// Make sure the insertion doesn't exceed address space limit.
    fn insert_try_merge(&mut self, vm_mapping: VmMapping) {
        self.total_vm += vm_mapping.map_size();
        let mut vm_mapping = vm_mapping;
        let addr = vm_mapping.map_to_addr();

        if let Some(prev) = self.vm_mappings.find_prev(&addr) {
            let (new_mapping, to_remove) = vm_mapping.try_merge_with(prev);
            vm_mapping = new_mapping;
            if let Some(addr) = to_remove {
                self.vm_mappings.remove(&addr);
            }
        }

        if let Some(next) = self.vm_mappings.find_next(&addr) {
            let (new_mapping, to_remove) = vm_mapping.try_merge_with(next);
            vm_mapping = new_mapping;
            if let Some(addr) = to_remove {
                self.vm_mappings.remove(&addr);
            }
        }

        self.vm_mappings.insert(vm_mapping);
    }

    /// Removes a `VmMapping` based on the provided key from the `Vmar`.
    fn remove(&mut self, key: &Vaddr) -> Option<VmMapping> {
        let vm_mapping = self.vm_mappings.remove(key)?;
        self.total_vm -= vm_mapping.map_size();
        Some(vm_mapping)
    }

    /// Finds a set of [`VmMapping`]s that intersect with the provided range.
    fn query(&self, range: &Range<Vaddr>) -> impl Iterator<Item = &VmMapping> {
        self.vm_mappings.find(range)
    }

    /// Calculates the total amount of overlap between `VmMapping`s
    /// and the provided range.
    fn count_overlap_size(&self, range: Range<Vaddr>) -> usize {
        let mut sum_overlap_size = 0;
        for vm_mapping in self.vm_mappings.find(&range) {
            let vm_mapping_range = vm_mapping.range();
            let intersected_range = get_intersected_range(&range, &vm_mapping_range);
            sum_overlap_size += intersected_range.end - intersected_range.start;
        }
        sum_overlap_size
    }

    /// Allocates a free region for mapping with a specific offset and size.
    ///
    /// If the provided range is already occupied, return an error.
    fn alloc_free_region_exact(&mut self, offset: Vaddr, size: usize) -> Result<Range<Vaddr>> {
        if self
            .vm_mappings
            .find(&(offset..offset + size))
            .next()
            .is_some()
        {
            return_errno_with_message!(
                Errno::EEXIST,
                "the range contains pages that are already mapped"
            );
        }

        Ok(offset..(offset + size))
    }

    /// Allocates a free region for mapping with a specific offset and size.
    ///
    /// If the provided range is already occupied, this function truncates all
    /// the mappings that intersect with the range.
    fn alloc_free_region_exact_truncate(
        &mut self,
        vm_space: &VmSpace,
        offset: Vaddr,
        size: usize,
        rss_delta: &mut RssDelta,
    ) -> Result<Range<Vaddr>> {
        let range = offset..offset + size;
        let mut mappings_to_remove = Vec::new();
        for vm_mapping in self.vm_mappings.find(&range) {
            mappings_to_remove.push(vm_mapping.map_to_addr());
        }

        for vm_mapping_addr in mappings_to_remove {
            let vm_mapping = self.remove(&vm_mapping_addr).unwrap();
            let vm_mapping_range = vm_mapping.range();
            let intersected_range = get_intersected_range(&range, &vm_mapping_range);

            let (left, taken, right) = vm_mapping.split_range(&intersected_range);
            if let Some(left) = left {
                self.insert_without_try_merge(left);
            }
            if let Some(right) = right {
                self.insert_without_try_merge(right);
            }

            rss_delta.add(taken.rss_type(), -(taken.unmap(vm_space) as isize));
        }

        Ok(offset..(offset + size))
    }

    /// Allocates a free region for mapping.
    ///
    /// If no such region is found, return an error.
    fn alloc_free_region(&mut self, size: usize, align: usize) -> Result<Range<Vaddr>> {
        // Fast path that there's still room to the end.
        let highest_occupied = self
            .vm_mappings
            .iter()
            .next_back()
            .map_or(VMAR_LOWEST_ADDR, |vm_mapping| vm_mapping.range().end);
        // FIXME: The up-align may overflow.
        let last_occupied_aligned = highest_occupied.align_up(align);
        if let Some(last) = last_occupied_aligned.checked_add(size)
            && last <= VMAR_CAP_ADDR
        {
            return Ok(last_occupied_aligned..last);
        }

        // Slow path that we need to search for a free region.
        // Here, we use a simple brute-force FIRST-FIT algorithm.
        // Allocate as low as possible to reduce fragmentation.
        let mut last_end: Vaddr = VMAR_LOWEST_ADDR;
        for vm_mapping in self.vm_mappings.iter() {
            let range = vm_mapping.range();

            debug_assert!(range.start >= last_end);
            debug_assert!(range.end <= highest_occupied);

            let last_aligned = last_end.align_up(align);
            let needed_end = last_aligned
                .checked_add(size)
                .ok_or(Error::new(Errno::ENOMEM))?;

            if needed_end <= range.start {
                return Ok(last_aligned..needed_end);
            }

            last_end = range.end;
        }

        return_errno_with_message!(Errno::ENOMEM, "Cannot find free region for mapping");
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
            return_errno_with_message!(Errno::EINVAL, "resizing a mapping to zero is invalid");
        }

        if new_size == old_size {
            return Ok(());
        }

        if !is_userspace_vaddr_range(map_addr, new_size) {
            return_errno_with_message!(Errno::EINVAL, "the address range is not in userspace");
        }

        if new_size < old_size {
            // Shrink the mapping. The old mapping is larger.
            if map_addr.checked_add(old_size).is_none() {
                return_errno_with_message!(Errno::EINVAL, "the address range overflows");
            }
            self.alloc_free_region_exact_truncate(
                vm_space,
                map_addr + new_size,
                old_size - new_size,
                rss_delta,
            )?;
            return Ok(());
        }

        // Expand the mapping. The old mapping is smaller.
        let old_map_end = map_addr + old_size;
        self.alloc_free_region_exact(old_map_end, new_size - old_size)
            .map_err(|_| {
                Error::with_message(
                    Errno::ENOMEM,
                    "the range contains pages that are already mapped",
                )
            })?;

        let last_mapping = self.vm_mappings.find_one(&(old_map_end - 1)).unwrap();
        let last_mapping_addr = last_mapping.map_to_addr();
        debug_assert_eq!(last_mapping.map_end(), old_map_end);

        if !last_mapping.can_expand() {
            return_errno_with_message!(Errno::EFAULT, "device mappings cannot be expanded");
        }

        self.check_extra_size_fits_rlimit(new_size - old_size)?;
        let last_mapping = self.remove(&last_mapping_addr).unwrap();
        let last_mapping = last_mapping.enlarge(new_size - old_size);
        self.insert_try_merge(last_mapping);
        Ok(())
    }
}
