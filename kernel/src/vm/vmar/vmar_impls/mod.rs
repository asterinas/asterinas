// SPDX-License-Identifier: MPL-2.0

mod access_remote;
mod fork;
mod map;
mod page_fault;
mod protect;
mod query;
mod remap;
mod rs_as_delta;
mod unmap;

use core::{
    array,
    ops::Range,
    sync::atomic::{AtomicIsize, Ordering},
};

use aster_util::per_cpu_counter::PerCpuCounter;
pub use map::OffsetType;
use osdk_heap_allocator::{CpuLocalBox, alloc_cpu_local};
use ostd::{
    cpu::{CpuId, all_cpus},
    mm::{AuxPageTableMeta, PagingLevel, Vaddr, VmSpace, page_size_at, vm_space::CursorMut},
};
pub(super) use rs_as_delta::RsAsDelta;
pub use rs_as_delta::RssType;

use super::{
    interval_set::{Interval, IntervalSet},
    vm_mapping::{MappedMemory, MappedVmo, VmMapping},
};
use crate::{prelude::*, process::ProcessVm, vm::vmar::vm_allocator::VirtualAddressAllocator};

/// Virtual Memory Address Regions (VMARs) are a type of capability that manages
/// user address spaces.
pub struct Vmar {
    /// The attached `VmSpace`.
    vm_space: Arc<VmarSpace>,
    /// The allocator for virtual address ranges.
    allocator: VirtualAddressAllocator,
    /// The used quota of address space size on each CPU.
    ///
    /// The sum of the values on each CPU is the total number of virtual memory
    /// bytes mapped. Values on each CPU does not have a specific meaning, but
    /// it must not exceed the resource limit divided by the number of CPUs.
    mapped_vm_size: CpuLocalBox<AtomicIsize>,
    /// The RSS counters.
    rss_counters: [PerCpuCounter; rs_as_delta::NUM_RSS_COUNTERS],
    /// The process VM.
    process_vm: ProcessVm,
}

impl Vmar {
    /// Creates a new VMAR.
    pub fn new(process_vm: ProcessVm) -> Result<Arc<Self>> {
        let vm_space = VmSpace::<PerPtMeta>::new();
        let rss_counters = array::from_fn(|_| PerCpuCounter::new());
        let vmar = Vmar {
            vm_space: Arc::new(vm_space),
            allocator: VirtualAddressAllocator::new()?,
            rss_counters,
            mapped_vm_size: alloc_cpu_local(|_| AtomicIsize::new(0))?,
            process_vm,
        };

        let stack_region = vmar.process_vm.init_stack().reserved_region();
        vmar.reserve_specific(stack_region).unwrap();

        Ok(Arc::new(vmar))
    }

    /// Returns the current RSS count for the given RSS type.
    pub fn get_rss_counter(&self, rss_type: RssType) -> usize {
        self.rss_counters[rss_type as usize].sum_all_cpus()
    }

    /// Returns the total size of the mappings in bytes.
    pub fn get_mappings_total_size(&self) -> usize {
        all_cpus()
            .map(|cpu| self.mapped_vm_size.get_on_cpu(cpu).load(Ordering::Relaxed))
            .sum::<isize>() as usize
    }

    /// Returns the attached `VmSpace`.
    pub fn vm_space(&self) -> &Arc<VmarSpace> {
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

#[derive(Debug)]
pub struct PerPtMeta {
    pub inner: IntervalSet<Vaddr, PteRangeMeta>,
}

pub type VmarCursorMut<'a> = CursorMut<'a, PerPtMeta>;
pub type VmarSpace = VmSpace<PerPtMeta>;

#[derive(Debug)]
pub enum PteRangeMeta {
    ChildPt(Range<Vaddr>),
    VmMapping(VmMapping),
}

impl PteRangeMeta {
    #[track_caller]
    pub fn unwrap_mapping(self) -> VmMapping {
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
