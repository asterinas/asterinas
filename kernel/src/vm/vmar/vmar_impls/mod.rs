// SPDX-License-Identifier: MPL-2.0

mod access_alien;
mod fork;
pub(super) mod map;
pub(super) mod page_fault;
mod protect;
mod query;
pub(super) mod remap;
mod unmap;

use core::{
    array,
    ops::Range,
    sync::atomic::{AtomicUsize, Ordering},
};

use align_ext::AlignExt;
use aster_util::per_cpu_counter::PerCpuCounter;
use ostd::{cpu::CpuId, mm::VmSpace};

use super::{
    Rmap, RmapEntry, VMAR_CAP_ADDR, VMAR_LOWEST_ADDR,
    interval_set::{Interval, IntervalSet},
    is_userspace_vaddr, is_userspace_vaddr_range,
    util::{self, get_intersected_range},
    vm_mapping::{MappedMemory, MappedVmo, VmMapping},
};
use crate::{
    prelude::*,
    process::{INIT_STACK_SIZE, Process, ProcessVm, ResourceType},
    vm::page_cache::Vmo,
};

/// The upper bound for allocations under the `MAP_32BIT` mmap flag.
#[cfg(target_arch = "x86_64")]
const MAP_32BIT_HIGH_LIMIT: Vaddr = 0x8000_0000;

/// The VMAR (used to be Virtual Memory Address Region, but now an orphan
/// initialism).
///
/// A VMAR is the address space of a process.
pub struct Vmar {
    /// VMAR inner
    inner: RwMutex<VmarInner>,
    /// The attached `VmSpace`
    vm_space: Arc<VmSpace>,
    /// The RSS counters
    rss_counters: [PerCpuCounter; NUM_RSS_COUNTERS],
    /// The process VM
    process_vm: ProcessVm,
    /// The number of handles that this `Vmar` has (see [`super::VmarHandle`])
    num_handles: AtomicUsize,
    /// Weak self reference
    weak_self: Weak<Self>,
}

impl Vmar {
    /// Creates a new VMAR.
    ///
    /// This method should only be invoked by [`super::VmarHandle`].
    pub(super) fn new(process_vm: ProcessVm) -> Arc<Self> {
        let inner = VmarInner::new();
        let vm_space = VmSpace::new();
        let rss_counters = array::from_fn(|_| PerCpuCounter::new());
        Arc::new_cyclic(move |weak_self| Vmar {
            inner: RwMutex::new(inner),
            vm_space: Arc::new(vm_space),
            rss_counters,
            process_vm,
            num_handles: AtomicUsize::new(1),
            weak_self: weak_self.clone(),
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

    /// Returns whether this VMAR has multiple handles.
    pub fn has_multiple_handles(&self) -> bool {
        self.num_handles.load(Ordering::Relaxed) > 1
    }

    /// Increases the number of handles.
    ///
    /// This method should only be invoked by [`super::VmarHandle`].
    pub(super) fn inc_num_handles(&self) {
        let old_num_handles = self.num_handles.fetch_add(1, Ordering::Relaxed);
        debug_assert_ne!(old_num_handles, 0);
    }

    /// Decreases the number of handles.
    ///
    /// This method should only be invoked by [`super::VmarHandle`].
    pub(super) fn dec_num_handles(&self) {
        let old_num_handles = self.num_handles.fetch_sub(1, Ordering::Relaxed);
        debug_assert_ne!(old_num_handles, 0);
        if old_num_handles == 1 {
            // Clear all the mappings. The last process using this VMAR exited
            // or executed a new program, so this VMAR no longer has a handle.
            self.clear();
        }
    }

    fn add_rss_counter(&self, rss_type: RssType, val: isize) {
        // There are races but updating a remote counter won't cause any problems.
        let cpu_id = CpuId::current_racy();
        self.rss_counters[rss_type as usize].add_on_cpu(cpu_id, val);
    }
}

/// The type representing categories of Resident Set Size (RSS).
///
/// Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/include/linux/mm_types_task.h#L26-L32>
#[repr(u32)]
#[derive(Clone, Copy, Debug, TryFromInt)]
pub enum RssType {
    File = 0,
    Anon = 1,
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

/// A holder for a reverse mapping entry that is to be removed.
///
/// This holds the [`Vmo`] object, so the lock guard for reverse mappings can
/// have a correct lifetime (see [`Self::remove`]).
#[must_use]
struct RmapToRemove {
    vmo: Option<Arc<Vmo>>,
}

impl RmapToRemove {
    pub(self) fn new(vmo: Option<Arc<Vmo>>) -> Self {
        Self { vmo }
    }

    pub(self) fn remove(&self, vmar: &Vmar, addr: Vaddr) -> Option<MutexGuard<'_, Rmap>> {
        let mut rmap = self.vmo.as_ref()?.rmap().lock();
        rmap.remove(vmar.weak_self.clone(), addr);
        Some(rmap)
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

        let rlimit_as = process
            .resource_limits()
            .get_rlimit(ResourceType::RLIMIT_AS)
            .get_cur();

        if rlimit_as.saturating_sub(self.total_vm as u64) < expand_size as u64 {
            return_errno_with_message!(Errno::ENOMEM, "the address space size limit is reached");
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
    fn insert_without_try_merge(
        &mut self,
        vmar: &Vmar,
        vm_mapping: VmMapping,
        rmap: Option<&mut Rmap>,
    ) {
        self.total_vm += vm_mapping.map_size();

        if let Some(rmap) = rmap {
            rmap.insert(
                vmar.weak_self.clone(),
                RmapEntry {
                    vaddr: vm_mapping.map_to_addr(),
                    offset: vm_mapping.vmo().unwrap().offset(),
                    size: vm_mapping.map_size(),
                },
            );
        }
        self.vm_mappings.insert(vm_mapping);
    }

    /// Inserts a `VmMapping` into the `Vmar`, and attempts to merge it with
    /// neighboring mappings.
    ///
    /// This method will try to merge the `VmMapping` with neighboring mappings
    /// that are adjacent and compatible, in order to reduce fragmentation.
    ///
    /// Make sure the insertion doesn't exceed address space limit.
    fn insert_try_merge(
        &mut self,
        vmar: &Vmar,
        vm_mapping: VmMapping,
        mut rmap: Option<&mut Rmap>,
    ) {
        self.total_vm += vm_mapping.map_size();
        let mut vm_mapping = vm_mapping;
        let addr = vm_mapping.map_to_addr();

        if let Some(prev) = self.vm_mappings.find_prev(&addr) {
            let (new_mapping, to_remove) = vm_mapping.try_merge_with(prev);
            vm_mapping = new_mapping;
            if let Some(addr) = to_remove {
                self.vm_mappings.remove(&addr);
                if let Some(rmap) = rmap.as_deref_mut() {
                    rmap.remove(vmar.weak_self.clone(), addr);
                }
            }
        }

        if let Some(next) = self.vm_mappings.find_next(&addr) {
            let (new_mapping, to_remove) = vm_mapping.try_merge_with(next);
            vm_mapping = new_mapping;
            if let Some(addr) = to_remove {
                self.vm_mappings.remove(&addr);
                if let Some(rmap) = rmap.as_deref_mut() {
                    rmap.remove(vmar.weak_self.clone(), addr);
                }
            }
        }

        if let Some(rmap) = rmap {
            rmap.insert(
                vmar.weak_self.clone(),
                RmapEntry {
                    vaddr: vm_mapping.map_to_addr(),
                    offset: vm_mapping.vmo().unwrap().offset(),
                    size: vm_mapping.map_size(),
                },
            );
        }
        self.vm_mappings.insert(vm_mapping);
    }

    /// Removes a `VmMapping` based on the provided key from the `Vmar`.
    fn remove(&mut self, key: &Vaddr) -> Option<(VmMapping, RmapToRemove)> {
        let vm_mapping = self.vm_mappings.remove(key)?;
        self.total_vm -= vm_mapping.map_size();

        let rmap_to_remove = RmapToRemove::new(vm_mapping.vmo_for_rmap().cloned());

        Some((vm_mapping, rmap_to_remove))
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
        vmar: &Vmar,
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
            let (vm_mapping, rmap_to_remove) = self.remove(&vm_mapping_addr).unwrap();
            let mut rmap = rmap_to_remove.remove(vmar, vm_mapping_addr);

            let vm_mapping_range = vm_mapping.range();
            let intersected_range = get_intersected_range(&range, &vm_mapping_range);

            let (left, taken, right) = vm_mapping.split_range(&intersected_range);
            if let Some(left) = left {
                self.insert_without_try_merge(vmar, left, rmap.as_deref_mut());
            }
            if let Some(right) = right {
                self.insert_without_try_merge(vmar, right, rmap.as_deref_mut());
            }

            // Note that `rmap` must be dropped before `taken`. Otherwise,
            // there is a possibility of a deadlock because a device mapping
            // may attempt to access its reverse mappings in `Drop`.
            drop(rmap);

            rss_delta.add(taken.rss_type(), -(taken.unmap(&vmar.vm_space) as isize));
        }

        Ok(offset..(offset + size))
    }

    /// Allocates a free region for mapping, searching from high address to low address.
    ///
    /// If no such region is found, return an error.
    fn alloc_free_region(&mut self, size: usize, align: usize) -> Result<Range<Vaddr>> {
        // This value represents the highest possible address for a new mapping.
        // For simplicity, we use a fixed value `2048` here. The value contains the following considerations:
        // - The stack fixed padding size.
        // - The stack random padding size.
        // - The future growth of the stack.
        // FIXME: This value should consider the process's actual stack configuration, which may
        // exist in `ResourceLimits`.
        let high_limit = VMAR_CAP_ADDR - INIT_STACK_SIZE - PAGE_SIZE * 2048;
        let low_limit = VMAR_LOWEST_ADDR;
        self.alloc_free_region_in_range(size, align, low_limit, high_limit)
    }

    /// Allocates a free region for mapping that resides below 2 GiB.
    ///
    /// This is specifically used for supporting the `MAP_32BIT` mmap flag.
    #[cfg(target_arch = "x86_64")]
    fn alloc_free_region_below_2gib(&mut self, size: usize, align: usize) -> Result<Range<Vaddr>> {
        let high_limit = MAP_32BIT_HIGH_LIMIT.min(VMAR_CAP_ADDR);
        let low_limit = VMAR_LOWEST_ADDR;
        self.alloc_free_region_in_range(size, align, low_limit, high_limit)
    }

    /// Core logic for `alloc_free_region` and `alloc_free_region_below_2gib`.
    ///
    /// Searches for a free region in `[low_limit, high_limit)`, from high to low.
    fn alloc_free_region_in_range(
        &mut self,
        size: usize,
        align: usize,
        low_limit: Vaddr,
        high_limit: Vaddr,
    ) -> Result<Range<Vaddr>> {
        fn try_alloc_in_hole(
            hole_start: Vaddr,
            hole_end: Vaddr,
            size: usize,
            align: usize,
        ) -> Option<Range<Vaddr>> {
            let start = hole_end.checked_sub(size)?.align_down(align);
            if start >= hole_start {
                Some(start..start + size)
            } else {
                None
            }
        }

        let mut prev_vm_mapping_start = high_limit;
        for vm_mapping in self.vm_mappings.iter().rev() {
            let hole_start = vm_mapping.range().end.max(low_limit);
            let hole_end = prev_vm_mapping_start.min(high_limit);

            if let Some(region) = try_alloc_in_hole(hole_start, hole_end, size, align) {
                return Ok(region);
            }

            prev_vm_mapping_start = vm_mapping.range().start;
            if prev_vm_mapping_start <= low_limit {
                break;
            }
        }

        // Check the hole between `low_limit` and the lowest mapping.
        if prev_vm_mapping_start > low_limit {
            let hole_start = low_limit;
            let hole_end = prev_vm_mapping_start.min(high_limit);
            if let Some(region) = try_alloc_in_hole(hole_start, hole_end, size, align) {
                return Ok(region);
            }
        }

        return_errno_with_message!(Errno::ENOMEM, "no free region for mapping can be found");
    }

    /// Splits and unmaps the found mapping if the new size is smaller.
    /// Enlarges the last mapping if the new size is larger.
    fn resize_mapping(
        &mut self,
        vmar: &Vmar,
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
                vmar,
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
        let (last_mapping, rmap_to_remove) = self.remove(&last_mapping_addr).unwrap();
        let mut rmap = rmap_to_remove.remove(vmar, last_mapping_addr);
        let last_mapping = last_mapping.enlarge(new_size - old_size);
        self.insert_try_merge(vmar, last_mapping, rmap.as_deref_mut());
        Ok(())
    }
}
