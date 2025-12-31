// SPDX-License-Identifier: MPL-2.0

mod access_alien;
mod fork;
pub(super) mod map;
pub(super) mod page_fault;
mod protect;
mod query;
pub(super) mod remap;
mod rs_as_delta;
mod unmap;

use core::{
    array,
    ops::Range,
    sync::atomic::{AtomicIsize, AtomicUsize, Ordering},
};

use aster_util::per_cpu_counter::PerCpuCounter;
pub use map::OffsetType;
use osdk_heap_allocator::{CpuLocalBox, alloc_cpu_local};
use ostd::{
    cpu::{CpuId, all_cpus},
    mm::{AuxPageTableMeta, PagingLevel, VmSpace, page_size_at, vm_space::CursorMut},
};
pub use remap::RemapOldMappingAction;
pub(super) use rs_as_delta::RsAsDelta;
pub use rs_as_delta::RssType;

use super::{
    RmapEntry,
    cursor::CursorExt,
    interval_set::{Interval, IntervalSet},
    vm_mapping::{MappedMemory, MappedVmo, VmMapping},
};
use crate::{
    prelude::*,
    process::ProcessVm,
    vm::{page_cache::Vmo, vmar::vm_allocator::VirtualAddressAllocator},
};

/// The upper bound for allocations under the `MAP_32BIT` mmap flag.
#[cfg(target_arch = "x86_64")]
const MAP_32BIT_HIGH_LIMIT: Vaddr = 0x8000_0000;

/// The VMAR (used to be Virtual Memory Address Region, but now an orphan
/// initialism).
///
/// A VMAR is the address space of a process.
pub(crate) struct Vmar {
    /// The attached `VmSpace`.
    vm_space: Arc<VmarSpace>,
    /// The allocator for virtual address ranges.
    pub(super) allocator: VirtualAddressAllocator,
    /// The used quota of address space size on each CPU.
    ///
    /// The sum of the values on each CPU is the total number of virtual memory
    /// bytes mapped. Values on each CPU does not have a specific meaning, but
    /// it must not exceed the resource limit divided by the number of CPUs.
    pub(super) mapped_vm_size: CpuLocalBox<AtomicIsize>,
    /// The RSS counters.
    rss_counters: [PerCpuCounter; rs_as_delta::NUM_RSS_COUNTERS],
    /// The process VM.
    process_vm: ProcessVm,
    /// The number of handles that this `Vmar` has (see [`super::VmarHandle`])
    num_handles: AtomicUsize,
    /// Weak self-reference used by VMO reverse mappings.
    weak_self: Weak<Self>,
}

impl Vmar {
    /// Creates a new VMAR.
    ///
    /// This method should only be invoked by [`super::VmarHandle`].
    pub(super) fn new(process_vm: ProcessVm) -> Result<Arc<Self>> {
        let vm_space = Arc::new(VmSpace::<PerPtMeta>::new());
        let allocator = VirtualAddressAllocator::new()?;
        let mapped_vm_size = alloc_cpu_local(|_| AtomicIsize::new(0))?;
        let rss_counters = array::from_fn(|_| PerCpuCounter::new());
        let vmar = Arc::new_cyclic(move |weak_self| Vmar {
            vm_space,
            allocator,
            mapped_vm_size,
            rss_counters,
            process_vm,
            num_handles: AtomicUsize::new(1),
            weak_self: weak_self.clone(),
        });

        let stack_region = vmar.process_vm.init_stack().reserved_region();
        vmar.reserve_specific(stack_region).unwrap();

        Ok(vmar)
    }

    /// Returns the current RSS count for the given RSS type.
    pub(crate) fn get_rss_counter(&self, rss_type: RssType) -> usize {
        self.rss_counters[rss_type as usize].sum_all_cpus()
    }

    /// Returns the total size of the mappings in bytes.
    pub(crate) fn get_mappings_total_size(&self) -> usize {
        all_cpus()
            .map(|cpu| self.mapped_vm_size.get_on_cpu(cpu).load(Ordering::Relaxed))
            .sum::<isize>() as usize
    }

    /// Returns the attached `VmSpace`.
    pub(crate) fn vm_space(&self) -> &Arc<VmarSpace> {
        &self.vm_space
    }

    /// Returns the attached `ProcessVm`.
    pub(crate) fn process_vm(&self) -> &ProcessVm {
        &self.process_vm
    }

    /// Returns whether this VMAR has multiple handles.
    pub(crate) fn has_multiple_handles(&self) -> bool {
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

    /// Takes a snapshot of reverse-map entries while the page-table range is locked.
    fn snapshot_rmap_entries(
        cursor: &mut VmarCursorMut<'_>,
        ranges: &[Range<Vaddr>],
    ) -> Vec<(Arc<Vmo>, RmapEntry)> {
        let original_addr = cursor.virt_addr();
        let original_level = cursor.level();
        let mut entries = Vec::new();

        for range in ranges {
            cursor.jump(range.start).unwrap();
            while let Some(mapping) = cursor.find_next_mapped(range.end) {
                let mapping_end = mapping.map_end();
                if let Some((vmo, entry)) = mapping.rmap_entry_in(range)
                    && !entries
                        .iter()
                        .any(|(old_vmo, old_entry): &(Arc<Vmo>, RmapEntry)| {
                            Arc::ptr_eq(old_vmo, &vmo)
                                && old_entry.vaddr == entry.vaddr
                                && old_entry.offset == entry.offset
                                && old_entry.size == entry.size
                        })
                {
                    entries.push((vmo, entry));
                }

                if cursor.jump(mapping_end).is_err() {
                    break;
                }
            }
        }

        if cursor.guard_va_range().contains(&original_addr) {
            cursor.jump(original_addr).unwrap();
            cursor.adjust_level(original_level);
        }

        entries
    }

    /// Refreshes reverse-map entries before releasing the page-table cursor.
    ///
    /// Lock order for forward mapping changes is page table, then reverse map.
    /// Reverse-map walkers must use nonblocking cursor acquisition and retry
    /// after dropping their reverse-map lock.
    fn refresh_rmap_entries(
        &self,
        cursor: &mut VmarCursorMut<'_>,
        old_entries: Vec<(Arc<Vmo>, RmapEntry)>,
        ranges: &[Range<Vaddr>],
    ) {
        let new_entries = Self::snapshot_rmap_entries(cursor, ranges);
        let mut old_vmos: Vec<Arc<Vmo>> = Vec::new();
        for (vmo, _) in old_entries {
            if !old_vmos.iter().any(|old| Arc::ptr_eq(old, &vmo)) {
                old_vmos.push(vmo);
            }
        }

        for vmo in old_vmos {
            let mut rmap = vmo.rmap().lock();
            for range in ranges {
                rmap.remove_range(self.weak_self.clone(), range);
            }
        }

        for (vmo, entry) in new_entries {
            vmo.rmap().lock().insert(self.weak_self.clone(), entry);
        }
    }
}

#[derive(Debug)]
pub(crate) struct PerPtMeta {
    pub(super) inner: IntervalSet<Vaddr, PteRangeMeta>,
}

pub(super) type VmarCursorMut<'a> = CursorMut<'a, PerPtMeta>;
pub(crate) type VmarSpace = VmSpace<PerPtMeta>;

#[derive(Debug)]
pub(super) enum PteRangeMeta {
    ChildPt(Range<Vaddr>),
    VmMapping(VmMapping),
}

impl PteRangeMeta {
    #[track_caller]
    pub(super) fn unwrap_mapping(self) -> VmMapping {
        match self {
            PteRangeMeta::VmMapping(vm_mapping) => vm_mapping,
            PteRangeMeta::ChildPt(_) => panic!("called `unwrap_mapping` on a `ChildPt`"),
        }
    }
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

    fn alloc_child_page_table(&mut self, va: Vaddr, level: PagingLevel) -> Self {
        let page_size = page_size_at(level);
        let range = va..va + page_size;

        let old = self.inner.take_one(&va);
        let child_meta = match old {
            Some(PteRangeMeta::ChildPt(_)) => {
                unreachable!("should not allocate child PT for existing child PT")
            }
            Some(PteRangeMeta::VmMapping(mapping)) => {
                let (left, mid, right) = mapping.split_range(&range);

                if let Some(left) = left {
                    self.inner.insert(PteRangeMeta::VmMapping(left));
                }
                if let Some(right) = right {
                    self.inner.insert(PteRangeMeta::VmMapping(right));
                }

                let child_meta_val = PteRangeMeta::VmMapping(mid);
                let mut child_meta = PerPtMeta::new();
                child_meta.inner.insert(child_meta_val);

                child_meta
            }
            None => {
                // No existing mapping, just insert a new child PT meta.
                Self::new()
            }
        };

        self.inner.insert(PteRangeMeta::ChildPt(range));

        child_meta
    }
}

impl PerPtMeta {
    const fn new() -> Self {
        Self {
            inner: IntervalSet::new(),
        }
    }

    /// Inserts a `VmMapping` into the `Vmar`, without attempting to merge with
    /// neighboring mappings.
    ///
    /// The caller must ensure that the given `VmMapping` is not mergeable with
    /// any neighboring mappings.
    ///
    /// Make sure the insertion doesn't exceed address space limit.
    pub(super) fn insert_without_try_merge(&mut self, vm_mapping: VmMapping) {
        self.inner.insert(PteRangeMeta::VmMapping(vm_mapping));
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

        if let Some(PteRangeMeta::VmMapping(prev)) = self.inner.find_prev(&addr) {
            let (new_mapping, to_remove) = vm_mapping.try_merge_with(prev);
            vm_mapping = new_mapping;
            if let Some(addr) = to_remove {
                self.inner.remove(&addr);
            }
        }

        if let Some(PteRangeMeta::VmMapping(next)) = self.inner.find_next(&addr) {
            let (new_mapping, to_remove) = vm_mapping.try_merge_with(next);
            vm_mapping = new_mapping;
            if let Some(addr) = to_remove {
                self.inner.remove(&addr);
            }
        }

        self.inner.insert(PteRangeMeta::VmMapping(vm_mapping));
    }
}
