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

use aster_util::per_cpu_counter::PerCpuCounter;
pub use map::OffsetType;
use ostd::{
    cpu::CpuId,
    mm::{AuxPageTableMeta, PagingLevel, Vaddr, VmSpace, page_size_at, vm_space::CursorMut},
};

use super::{
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
    vm_space: Arc<VmarSpace>,
    /// The RSS counters.
    rss_counters: [PerCpuCounter; NUM_RSS_COUNTERS],
    /// The total size of all mappings in bytes.
    total_vm: PerCpuCounter,
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
            total_vm: PerCpuCounter::new(),
            process_vm,
        })
    }

    /// Returns the current RSS count for the given RSS type.
    pub fn get_rss_counter(&self, rss_type: RssType) -> usize {
        self.rss_counters[rss_type as usize].sum_all_cpus()
    }

    /// Returns the total size of the mappings in bytes.
    pub fn get_mappings_total_size(&self) -> usize {
        self.total_vm.sum_all_cpus()
    }

    /// Returns the attached `VmSpace`.
    pub fn vm_space(&self) -> &Arc<VmarSpace> {
        &self.vm_space
    }

    /// Returns the attached `ProcessVm`.
    pub fn process_vm(&self) -> &ProcessVm {
        &self.process_vm
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
            .get_mappings_total_size()
            .checked_add(expand_size)
            .ok_or(Errno::ENOMEM)?;
        if new_total_vm > rlimt_as as usize {
            return_errno_with_message!(Errno::ENOMEM, "address space limit overflow");
        }
        Ok(())
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

/// A helper struct to track resident set and address space size changes.
pub(super) struct RsAsDelta<'a> {
    rs_as_delta: [isize; NUM_RSS_COUNTERS],
    as_delta: isize,
    operated_vmar: &'a Vmar,
}

impl<'a> RsAsDelta<'a> {
    pub(super) fn new(operated_vmar: &'a Vmar) -> Self {
        Self {
            rs_as_delta: [0; NUM_RSS_COUNTERS],
            as_delta: 0,
            operated_vmar,
        }
    }

    pub(super) fn add_rs(&mut self, rss_type: RssType, increment: isize) {
        self.rs_as_delta[rss_type as usize] += increment;
    }

    pub(super) fn add_as(&mut self, increment: isize) {
        self.as_delta += increment;
    }

    fn get_rs(&self, rss_type: RssType) -> isize {
        self.rs_as_delta[rss_type as usize]
    }

    fn get_as(&self) -> isize {
        self.as_delta
    }
}

impl Drop for RsAsDelta<'_> {
    fn drop(&mut self) {
        for i in 0..NUM_RSS_COUNTERS {
            let rss_type = RssType::try_from(i as u32).unwrap();
            let delta = self.get_rs(rss_type);
            self.operated_vmar.add_rss_counter(rss_type, delta);
        }
        // `current_racy` is OK because adding on any CPU is OK.
        self.operated_vmar
            .total_vm
            .add_on_cpu(CpuId::current_racy(), self.get_as());
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
