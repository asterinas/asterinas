// SPDX-License-Identifier: MPL-2.0

//! Virtual Memory Address Regions (VMARs).

mod dyn_cap;
mod interval_set;
mod static_cap;
pub mod vm_mapping;

use core::{array, num::NonZeroUsize, ops::Range};

use align_ext::AlignExt;
use aster_rights::Rights;
use ostd::{
    cpu::CpuId,
    mm::{
        tlb::TlbFlushOp, vm_space::CursorMut, CachePolicy, PageFlags, PageProperty, VmSpace,
        MAX_USERSPACE_VADDR,
    },
    sync::RwMutexReadGuard,
    task::disable_preempt,
};

use self::{
    interval_set::{Interval, IntervalSet},
    vm_mapping::{MappedVmo, VmMapping},
};
use crate::{
    fs::utils::Inode,
    prelude::*,
    process::{Process, ResourceType},
    thread::exception::PageFaultInfo,
    util::per_cpu_counter::PerCpuCounter,
    vm::{
        perms::VmPerms,
        vmo::{Vmo, VmoRightsOp},
    },
};

/// Virtual Memory Address Regions (VMARs) are a type of capability that manages
/// user address spaces.
///
/// # Capabilities
///
/// As a capability, each VMAR is associated with a set of access rights,
/// whose semantics are explained below.
///
/// The semantics of each access rights for VMARs are described below:
///  * The Dup right allows duplicating a VMAR.
///  * The Read, Write, Exec rights allow creating memory mappings with
///    readable, writable, and executable access permissions, respectively.
///  * The Read and Write rights allow the VMAR to be read from and written to
///    directly.
///
/// VMARs are implemented with two flavors of capabilities:
/// the dynamic one (`Vmar<Rights>`) and the static one (`Vmar<R: TRights>`).
pub struct Vmar<R = Rights>(Arc<Vmar_>, R);

pub trait VmarRightsOp {
    /// Returns the access rights.
    fn rights(&self) -> Rights;

    /// Checks whether current rights meet the input `rights`.
    fn check_rights(&self, rights: Rights) -> Result<()> {
        if self.rights().contains(rights) {
            Ok(())
        } else {
            return_errno_with_message!(Errno::EACCES, "VMAR rights are insufficient");
        }
    }
}

impl<R> PartialEq for Vmar<R> {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl<R> Vmar<R> {
    /// FIXME: This function should require access control
    pub fn vm_space(&self) -> &Arc<VmSpace> {
        self.0.vm_space()
    }

    /// Resizes the original mapping.
    ///
    /// The range of the mapping goes from `map_addr..map_addr + old_size` to
    /// `map_addr..map_addr + new_size`.
    ///
    /// If the new mapping size is smaller than the original mapping size, the
    /// extra part will be unmapped. If the new mapping is larger than the old
    /// mapping and the extra part overlaps with existing mapping, resizing
    /// will fail and return `Err`.
    ///
    /// - When `check_single_mapping` is `true`, this method will check whether
    ///   the range of the original mapping is covered by a single [`VmMapping`].
    ///   If not, this method will return an `Err`.
    /// - When `check_single_mapping` is `false`, The range of the original
    ///   mapping does not have to solely map to a whole [`VmMapping`], but it
    ///   must ensure that all existing ranges have a mapping. Otherwise, this
    ///   method will return an `Err`.
    pub fn resize_mapping(
        &self,
        map_addr: Vaddr,
        old_size: usize,
        new_size: usize,
        check_single_mapping: bool,
    ) -> Result<()> {
        self.0
            .resize_mapping(map_addr, old_size, new_size, check_single_mapping)
    }

    /// Remaps the original mapping to a new address and/or size.
    ///
    /// If the new mapping size is smaller than the original mapping size, the
    /// extra part will be unmapped.
    ///
    /// - If `new_addr` is `Some(new_addr)`, this method attempts to move the
    ///   mapping from `old_addr..old_addr + old_size` to `new_addr..new_addr +
    ///   new_size`. If any existing mappings lie within the target range,
    ///   they will be unmapped before the move.
    /// - If `new_addr` is `None`, a new range of size `new_size` will be
    ///   allocated, and the original mapping will be moved there.
    pub fn remap(
        &self,
        old_addr: Vaddr,
        old_size: usize,
        new_addr: Option<Vaddr>,
        new_size: usize,
    ) -> Result<Vaddr> {
        self.0.remap(old_addr, old_size, new_addr, new_size)
    }
}

pub(super) struct Vmar_ {
    /// VMAR inner
    inner: RwMutex<VmarInner>,
    /// The offset relative to the root VMAR
    base: Vaddr,
    /// The total size of the VMAR in bytes
    size: usize,
    /// The attached `VmSpace`
    vm_space: Arc<VmSpace>,
    /// The RSS counters.
    rss_counters: [PerCpuCounter; NUM_RSS_COUNTERS],
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
            return_errno_with_message!(Errno::EACCES, "Requested region is already occupied");
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
            .map_or(ROOT_VMAR_LOWEST_ADDR, |vm_mapping| vm_mapping.range().end);
        // FIXME: The up-align may overflow.
        let last_occupied_aligned = highest_occupied.align_up(align);
        if let Some(last) = last_occupied_aligned.checked_add(size) {
            if last <= ROOT_VMAR_CAP_ADDR {
                return Ok(last_occupied_aligned..last);
            }
        }

        // Slow path that we need to search for a free region.
        // Here, we use a simple brute-force FIRST-FIT algorithm.
        // Allocate as low as possible to reduce fragmentation.
        let mut last_end: Vaddr = ROOT_VMAR_LOWEST_ADDR;
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

pub const ROOT_VMAR_LOWEST_ADDR: Vaddr = 0x001_0000; // 64 KiB is the Linux configurable default
const ROOT_VMAR_CAP_ADDR: Vaddr = MAX_USERSPACE_VADDR;

/// Returns whether the input `vaddr` is a legal user space virtual address.
pub fn is_userspace_vaddr(vaddr: Vaddr) -> bool {
    (ROOT_VMAR_LOWEST_ADDR..ROOT_VMAR_CAP_ADDR).contains(&vaddr)
}

impl Interval<usize> for Arc<Vmar_> {
    fn range(&self) -> Range<usize> {
        self.base..(self.base + self.size)
    }
}

impl Vmar_ {
    fn new(
        inner: VmarInner,
        vm_space: Arc<VmSpace>,
        base: usize,
        size: usize,
        rss_counters: [PerCpuCounter; NUM_RSS_COUNTERS],
    ) -> Arc<Self> {
        Arc::new(Vmar_ {
            inner: RwMutex::new(inner),
            base,
            size,
            vm_space,
            rss_counters,
        })
    }

    fn new_root() -> Arc<Self> {
        let vmar_inner = VmarInner::new();
        let vm_space = VmSpace::new();
        Vmar_::new(
            vmar_inner,
            Arc::new(vm_space),
            0,
            ROOT_VMAR_CAP_ADDR,
            array::from_fn(|_| PerCpuCounter::new()),
        )
    }

    fn query(&self, range: Range<usize>) -> VmarQueryGuard<'_> {
        VmarQueryGuard {
            vmar: self.inner.read(),
            range,
        }
    }

    fn protect(&self, perms: VmPerms, range: Range<usize>) -> Result<()> {
        assert!(range.start % PAGE_SIZE == 0);
        assert!(range.end % PAGE_SIZE == 0);
        self.do_protect_inner(perms, range)?;
        Ok(())
    }

    // Do real protect. The protected range is ensured to be mapped.
    fn do_protect_inner(&self, perms: VmPerms, range: Range<usize>) -> Result<()> {
        let mut inner = self.inner.write();
        let vm_space = self.vm_space();

        let mut protect_mappings = Vec::new();

        for vm_mapping in inner.vm_mappings.find(&range) {
            protect_mappings.push((vm_mapping.map_to_addr(), vm_mapping.perms()));
        }

        for (vm_mapping_addr, vm_mapping_perms) in protect_mappings {
            if perms == vm_mapping_perms {
                continue;
            }
            let vm_mapping = inner.remove(&vm_mapping_addr).unwrap();
            let vm_mapping_range = vm_mapping.range();
            let intersected_range = get_intersected_range(&range, &vm_mapping_range);

            // Protects part of the taken `VmMapping`.
            let (left, taken, right) = vm_mapping.split_range(&intersected_range);

            // Puts the rest back.
            if let Some(left) = left {
                inner.insert_without_try_merge(left);
            }
            if let Some(right) = right {
                inner.insert_without_try_merge(right);
            }

            // Protects part of the `VmMapping`.
            let taken = taken.protect(vm_space.as_ref(), perms);
            inner.insert_try_merge(taken);
        }

        Ok(())
    }

    /// Handles user space page fault, if the page fault is successfully handled, return Ok(()).
    pub fn handle_page_fault(&self, page_fault_info: &PageFaultInfo) -> Result<()> {
        let address = page_fault_info.address;
        if !(self.base..self.base + self.size).contains(&address) {
            return_errno_with_message!(Errno::EACCES, "page fault addr is not in current vmar");
        }

        let inner = self.inner.read();

        if let Some(vm_mapping) = inner.vm_mappings.find_one(&address) {
            debug_assert!(vm_mapping.range().contains(&address));

            let mut rss_delta = RssDelta::new(self);
            return vm_mapping.handle_page_fault(&self.vm_space, page_fault_info, &mut rss_delta);
        }

        return_errno_with_message!(Errno::EACCES, "page fault addr is not in current vmar");
    }

    /// Clears all content of the root VMAR.
    fn clear_root_vmar(&self) -> Result<()> {
        let mut inner = self.inner.write();
        inner.vm_mappings.clear();

        // Keep `inner` locked to avoid race conditions.
        let preempt_guard = disable_preempt();
        let full_range = 0..MAX_USERSPACE_VADDR;
        let mut cursor = self
            .vm_space
            .cursor_mut(&preempt_guard, &full_range)
            .unwrap();
        cursor.unmap(full_range.len());
        cursor.flusher().sync_tlb_flush();

        Ok(())
    }

    pub fn remove_mapping(&self, range: Range<usize>) -> Result<()> {
        let mut inner = self.inner.write();
        let mut rss_delta = RssDelta::new(self);
        inner.alloc_free_region_exact_truncate(
            &self.vm_space,
            range.start,
            range.len(),
            &mut rss_delta,
        )?;
        Ok(())
    }

    /// Splits and unmaps the found mapping if the new size is smaller.
    /// Enlarges the last mapping if the new size is larger.
    fn resize_mapping(
        &self,
        map_addr: Vaddr,
        old_size: usize,
        new_size: usize,
        check_single_mapping: bool,
    ) -> Result<()> {
        let mut inner = self.inner.write();
        let mut rss_delta = RssDelta::new(self);

        if check_single_mapping {
            inner.check_lies_in_single_mapping(map_addr, old_size)?;
        } else if inner.vm_mappings.find_one(&map_addr).is_none() {
            return_errno_with_message!(Errno::EFAULT, "there is no mapping at the old address")
        }
        // FIXME: We should check whether all existing ranges in
        // `map_addr..map_addr + old_size` have a mapping. If not,
        // we should return an `Err`.

        inner.resize_mapping(&self.vm_space, map_addr, old_size, new_size, &mut rss_delta)
    }

    fn remap(
        &self,
        old_addr: Vaddr,
        old_size: usize,
        new_addr: Option<Vaddr>,
        new_size: usize,
    ) -> Result<Vaddr> {
        debug_assert_eq!(old_addr % PAGE_SIZE, 0);
        debug_assert_eq!(old_size % PAGE_SIZE, 0);
        debug_assert_eq!(new_size % PAGE_SIZE, 0);

        let mut inner = self.inner.write();
        let mut rss_delta = RssDelta::new(self);

        if inner.vm_mappings.find_one(&old_addr).is_none() {
            return_errno_with_message!(
                Errno::EFAULT,
                "remap: there is no mapping at the old address"
            )
        }

        // Shrink the old mapping first.
        old_addr.checked_add(old_size).ok_or(Errno::EINVAL)?;
        let (old_size, old_range) = if new_size < old_size {
            inner.alloc_free_region_exact_truncate(
                &self.vm_space,
                old_addr + new_size,
                old_size - new_size,
                &mut rss_delta,
            )?;
            (new_size, old_addr..old_addr + new_size)
        } else {
            (old_size, old_addr..old_addr + old_size)
        };

        // Allocate a new free region that does not overlap with the old range.
        let new_range = if let Some(new_addr) = new_addr {
            let new_range = new_addr..new_addr.checked_add(new_size).ok_or(Errno::EINVAL)?;
            if new_addr % PAGE_SIZE != 0
                || !is_userspace_vaddr(new_addr)
                || !is_userspace_vaddr(new_range.end - 1)
            {
                return_errno_with_message!(Errno::EINVAL, "remap: invalid fixed new address");
            }
            if is_intersected(&old_range, &new_range) {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "remap: the new range overlaps with the old one"
                );
            }
            inner.alloc_free_region_exact_truncate(
                &self.vm_space,
                new_addr,
                new_size,
                &mut rss_delta,
            )?
        } else {
            inner.alloc_free_region(new_size, PAGE_SIZE)?
        };

        // Create a new `VmMapping`.
        let old_mapping = {
            let old_mapping_addr = inner.check_lies_in_single_mapping(old_addr, old_size)?;
            let vm_mapping = inner.remove(&old_mapping_addr).unwrap();
            let (left, old_mapping, right) = vm_mapping.split_range(&old_range);
            if let Some(left) = left {
                inner.insert_without_try_merge(left);
            }
            if let Some(right) = right {
                inner.insert_without_try_merge(right);
            }
            old_mapping
        };
        // Note that we have ensured that `new_size >= old_size` at the beginning.
        let new_mapping = old_mapping.clone_for_remap_at(new_range.start).unwrap();
        inner.insert_try_merge(new_mapping.enlarge(new_size - old_size));

        let preempt_guard = disable_preempt();
        let total_range = old_range.start.min(new_range.start)..old_range.end.max(new_range.end);
        let vmspace = self.vm_space();
        let mut cursor = vmspace.cursor_mut(&preempt_guard, &total_range).unwrap();

        // Move the mapping.
        let mut current_offset = 0;
        while current_offset < old_size {
            cursor.jump(old_range.start + current_offset).unwrap();
            let Some(mapped_va) = cursor.find_next(old_size - current_offset) else {
                break;
            };
            let (va, Some((frame, prop))) = cursor.query().unwrap() else {
                panic!("Found mapped page but query failed");
            };
            debug_assert_eq!(mapped_va, va.start);
            cursor.unmap(PAGE_SIZE);

            let offset = mapped_va - old_range.start;
            cursor.jump(new_range.start + offset).unwrap();
            cursor.map(frame, prop);

            current_offset = offset + PAGE_SIZE;
        }

        cursor.flusher().dispatch_tlb_flush();
        cursor.flusher().sync_tlb_flush();

        Ok(new_range.start)
    }

    /// Returns the attached `VmSpace`.
    fn vm_space(&self) -> &Arc<VmSpace> {
        &self.vm_space
    }

    pub(super) fn new_fork_root(self: &Arc<Self>) -> Result<Arc<Self>> {
        let new_vmar_ = {
            let vmar_inner = VmarInner::new();
            let new_space = VmSpace::new();
            Vmar_::new(
                vmar_inner,
                Arc::new(new_space),
                self.base,
                self.size,
                array::from_fn(|_| PerCpuCounter::new()),
            )
        };

        {
            let inner = self.inner.read();
            let mut new_inner = new_vmar_.inner.write();

            // Clone mappings.
            let preempt_guard = disable_preempt();
            let new_vmspace = new_vmar_.vm_space();
            let range = self.base..(self.base + self.size);
            let mut new_cursor = new_vmspace.cursor_mut(&preempt_guard, &range).unwrap();
            let cur_vmspace = self.vm_space();
            let mut cur_cursor = cur_vmspace.cursor_mut(&preempt_guard, &range).unwrap();
            let mut rss_delta = RssDelta::new(&new_vmar_);

            for vm_mapping in inner.vm_mappings.iter() {
                let base = vm_mapping.map_to_addr();

                // Clone the `VmMapping` to the new VMAR.
                let new_mapping = vm_mapping.new_fork()?;
                new_inner.insert_without_try_merge(new_mapping);

                // Protect the mapping and copy to the new page table for COW.
                cur_cursor.jump(base).unwrap();
                new_cursor.jump(base).unwrap();

                let num_copied =
                    cow_copy_pt(&mut cur_cursor, &mut new_cursor, vm_mapping.map_size());

                rss_delta.add(vm_mapping.rss_type(), num_copied as isize);
            }

            cur_cursor.flusher().issue_tlb_flush(TlbFlushOp::All);
            cur_cursor.flusher().dispatch_tlb_flush();
            cur_cursor.flusher().sync_tlb_flush();
        }

        Ok(new_vmar_)
    }

    pub fn get_rss_counter(&self, rss_type: RssType) -> usize {
        self.rss_counters[rss_type as usize].get()
    }

    fn add_rss_counter(&self, rss_type: RssType, val: isize) {
        // There are races but updating a remote counter won't cause any problems.
        let cpu_id = CpuId::current_racy();
        self.rss_counters[rss_type as usize].add(cpu_id, val);
    }
}

/// Sets mappings in the source page table as read-only to trigger COW, and
/// copies the mappings to the destination page table.
///
/// The copied range starts from `src`'s current position with the given
/// `size`. The destination range starts from `dst`'s current position.
///
/// The number of physical frames copied is returned.
fn cow_copy_pt(src: &mut CursorMut<'_>, dst: &mut CursorMut<'_>, size: usize) -> usize {
    let start_va = src.virt_addr();
    let end_va = start_va + size;
    let mut remain_size = size;

    let mut num_copied = 0;

    let op = |flags: &mut PageFlags, _cache: &mut CachePolicy| {
        *flags -= PageFlags::W;
    };

    while let Some(mapped_va) = src.find_next(remain_size) {
        let (va, Some((frame, mut prop))) = src.query().unwrap() else {
            panic!("Found mapped page but query failed");
        };
        debug_assert_eq!(mapped_va, va.start);

        src.protect_next(end_va - mapped_va, op).unwrap();

        dst.jump(mapped_va).unwrap();
        op(&mut prop.flags, &mut prop.cache);
        dst.map(frame, prop);

        remain_size = end_va - src.virt_addr();

        num_copied += 1;
    }

    num_copied
}

impl<R> Vmar<R> {
    /// The base address, i.e., the offset relative to the root VMAR.
    ///
    /// The base address of a root VMAR is zero.
    pub fn base(&self) -> Vaddr {
        self.0.base
    }

    /// The size of the VMAR in bytes.
    pub fn size(&self) -> usize {
        self.0.size
    }

    /// Returns the current RSS count for the given RSS type.
    pub fn get_rss_counter(&self, rss_type: RssType) -> usize {
        self.0.get_rss_counter(rss_type)
    }

    /// Returns the total size of the mappings in bytes.
    pub fn get_mappings_total_size(&self) -> usize {
        self.0.inner.read().total_vm
    }
}

/// Options for creating a new mapping. The mapping is not allowed to overlap
/// with any child VMARs. And unless specified otherwise, it is not allowed
/// to overlap with any existing mapping, either.
pub struct VmarMapOptions<'a, R1, R2> {
    parent: &'a Vmar<R1>,
    vmo: Option<Vmo<R2>>,
    inode: Option<Arc<dyn Inode>>,
    perms: VmPerms,
    vmo_offset: usize,
    size: usize,
    offset: Option<usize>,
    align: usize,
    can_overwrite: bool,
    // Whether the mapping is mapped with `MAP_SHARED`
    is_shared: bool,
    // Whether the mapping needs to handle surrounding pages when handling page fault.
    handle_page_faults_around: bool,
}

impl<'a, R1, R2> VmarMapOptions<'a, R1, R2> {
    /// Creates a default set of options with the VMO and the memory access
    /// permissions.
    ///
    /// The VMO must have access rights that correspond to the memory
    /// access permissions. For example, if `perms` contains `VmPerms::Write`,
    /// then `vmo.rights()` should contain `Rights::WRITE`.
    pub fn new(parent: &'a Vmar<R1>, size: usize, perms: VmPerms) -> Self {
        Self {
            parent,
            vmo: None,
            inode: None,
            perms,
            vmo_offset: 0,
            size,
            offset: None,
            align: PAGE_SIZE,
            can_overwrite: false,
            is_shared: false,
            handle_page_faults_around: false,
        }
    }

    /// Binds a [`Vmo`] to the mapping.
    ///
    /// If the mapping is a private mapping, its size may not be equal to that
    /// of the [`Vmo`]. For example, it is OK to create a mapping whose size is
    /// larger than that of the [`Vmo`], although one cannot read from or write
    /// to the part of the mapping that is not backed by the [`Vmo`].
    ///
    /// Such _oversized_ mappings are useful for two reasons:
    ///  1. [`Vmo`]s are resizable. So even if a mapping is backed by a VMO
    ///     whose size is equal to that of the mapping initially, we cannot
    ///     prevent the VMO from shrinking.
    ///  2. Mappings are not allowed to overlap by default. As a result,
    ///     oversized mappings can reserve space for future expansions.
    ///
    /// The [`Vmo`] of a mapping will be implicitly set if [`Self::inode`] is
    /// set.
    ///
    /// # Panics
    ///
    /// This function panics if an [`Inode`] is already provided.
    pub fn vmo(mut self, vmo: Vmo<R2>) -> Self {
        if self.inode.is_some() {
            panic!("Cannot set `vmo` when `inode` is already set");
        }
        self.vmo = Some(vmo);

        self
    }

    /// Sets the offset of the first memory page in the VMO that is to be
    /// mapped into the VMAR.
    ///
    /// The offset must be page-aligned and within the VMO.
    ///
    /// The default value is zero.
    pub fn vmo_offset(mut self, offset: usize) -> Self {
        self.vmo_offset = offset;
        self
    }

    /// Sets the mapping's alignment.
    ///
    /// The default value is the page size.
    ///
    /// The provided alignment must be a power of two and a multiple of the
    /// page size.
    pub fn align(mut self, align: usize) -> Self {
        self.align = align;
        self
    }

    /// Sets the mapping's offset inside the VMAR.
    ///
    /// The offset must satisfy the alignment requirement.
    /// Also, the mapping's range `[offset, offset + size)` must be within
    /// the VMAR.
    ///
    /// If not set, the system will choose an offset automatically.
    pub fn offset(mut self, offset: usize) -> Self {
        self.offset = Some(offset);
        self
    }

    /// Sets whether the mapping can overwrite existing mappings.
    ///
    /// The default value is false.
    ///
    /// If this option is set to true, then the `offset` option must be
    /// set.
    pub fn can_overwrite(mut self, can_overwrite: bool) -> Self {
        self.can_overwrite = can_overwrite;
        self
    }

    /// Sets whether the mapping can be shared with other process.
    ///
    /// The default value is false.
    ///
    /// If this value is set to true, the mapping will be shared with child
    /// process when forking.
    pub fn is_shared(mut self, is_shared: bool) -> Self {
        self.is_shared = is_shared;
        self
    }

    /// Sets the mapping to handle surrounding pages when handling page fault.
    pub fn handle_page_faults_around(mut self) -> Self {
        self.handle_page_faults_around = true;
        self
    }
}

impl<R1> VmarMapOptions<'_, R1, Rights> {
    /// Binds an [`Inode`] to the mapping.
    ///
    /// This is used for file-backed mappings. The provided file inode will be
    /// mapped. See [`Self::vmo`] for details on the map size.
    ///
    /// If an [`Inode`] is provided, the [`Self::vmo`] must not be provided
    /// again. The actually mapped [`Vmo`] will be the [`Inode`]'s page cache.
    ///
    /// # Panics
    ///
    /// This function panics if:
    ///  - a [`Vmo`] or [`Inode`] is already provided;
    ///  - the provided [`Inode`] does not have a page cache.
    pub fn inode(mut self, inode: Arc<dyn Inode>) -> Self {
        if self.vmo.is_some() {
            panic!("Cannot set `inode` when `vmo` is already set");
        }
        self.vmo = Some(
            inode
                .page_cache()
                .expect("Map an inode without page cache")
                .to_dyn(),
        );
        self.inode = Some(inode);

        self
    }
}

impl<R1, R2> VmarMapOptions<'_, R1, R2>
where
    Vmo<R2>: VmoRightsOp,
{
    /// Creates the mapping and adds it to the parent VMAR.
    ///
    /// All options will be checked at this point.
    ///
    /// On success, the virtual address of the new mapping is returned.
    pub fn build(self) -> Result<Vaddr> {
        self.check_options()?;
        let Self {
            parent,
            vmo,
            inode,
            perms,
            vmo_offset,
            size: map_size,
            offset,
            align,
            can_overwrite,
            is_shared,
            handle_page_faults_around,
        } = self;

        let mut inner = parent.0.inner.write();

        inner.check_extra_size_fits_rlimit(map_size).or_else(|e| {
            if can_overwrite {
                let offset = offset.ok_or(Error::with_message(
                    Errno::EINVAL,
                    "offset cannot be None since can overwrite is set",
                ))?;
                // MAP_FIXED may remove pages overlapped with requested mapping.
                let expand_size = map_size - inner.count_overlap_size(offset..offset + map_size);
                inner.check_extra_size_fits_rlimit(expand_size)
            } else {
                Err(e)
            }
        })?;

        // Allocates a free region.
        trace!("allocate free region, map_size = 0x{:x}, offset = {:x?}, align = 0x{:x}, can_overwrite = {}", map_size, offset, align, can_overwrite);
        let map_to_addr = if can_overwrite {
            // If can overwrite, the offset is ensured not to be `None`.
            let offset = offset.ok_or(Error::with_message(
                Errno::EINVAL,
                "offset cannot be None since can overwrite is set",
            ))?;
            let mut rss_delta = RssDelta::new(&parent.0);
            inner.alloc_free_region_exact_truncate(
                parent.vm_space(),
                offset,
                map_size,
                &mut rss_delta,
            )?;
            offset
        } else if let Some(offset) = offset {
            inner.alloc_free_region_exact(offset, map_size)?;
            offset
        } else {
            let free_region = inner.alloc_free_region(map_size, align)?;
            free_region.start
        };

        // Build the mapping.
        let vmo = vmo.map(|vmo| MappedVmo::new(vmo.to_dyn(), vmo_offset));
        let vm_mapping = VmMapping::new(
            NonZeroUsize::new(map_size).unwrap(),
            map_to_addr,
            vmo,
            inode,
            is_shared,
            handle_page_faults_around,
            perms,
        );

        // Add the mapping to the VMAR.
        inner.insert_try_merge(vm_mapping);

        Ok(map_to_addr)
    }

    /// Checks whether all options are valid.
    fn check_options(&self) -> Result<()> {
        // Check align.
        debug_assert!(self.align % PAGE_SIZE == 0);
        debug_assert!(self.align.is_power_of_two());
        if self.align % PAGE_SIZE != 0 || !self.align.is_power_of_two() {
            return_errno_with_message!(Errno::EINVAL, "invalid align");
        }
        debug_assert!(self.size % self.align == 0);
        if self.size % self.align != 0 {
            return_errno_with_message!(Errno::EINVAL, "invalid mapping size");
        }
        debug_assert!(self.vmo_offset % self.align == 0);
        if self.vmo_offset % self.align != 0 {
            return_errno_with_message!(Errno::EINVAL, "invalid vmo offset");
        }
        if let Some(offset) = self.offset {
            debug_assert!(offset % self.align == 0);
            if offset % self.align != 0 {
                return_errno_with_message!(Errno::EINVAL, "invalid offset");
            }
        }
        self.check_perms()?;
        Ok(())
    }

    /// Checks whether the permissions of the mapping is subset of vmo rights.
    fn check_perms(&self) -> Result<()> {
        let Some(vmo) = &self.vmo else {
            return Ok(());
        };

        let perm_rights = Rights::from(self.perms);
        vmo.check_rights(perm_rights)
    }
}

/// A guard that allows querying a [`Vmar`] for its mappings.
pub struct VmarQueryGuard<'a> {
    vmar: RwMutexReadGuard<'a, VmarInner>,
    range: Range<usize>,
}

impl VmarQueryGuard<'_> {
    /// Returns an iterator over the [`VmMapping`]s that intersect with the
    /// provided range when calling [`Vmar::query`].
    pub fn iter(&self) -> impl Iterator<Item = &VmMapping> {
        self.vmar.query(&self.range)
    }
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
    operated_vmar: &'a Vmar_,
}

impl<'a> RssDelta<'a> {
    pub(self) fn new(operated_vmar: &'a Vmar_) -> Self {
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

#[cfg(ktest)]
mod test {
    use ostd::{
        mm::{CachePolicy, FrameAllocOptions},
        prelude::*,
    };

    use super::*;

    #[ktest]
    fn test_cow_copy_pt() {
        let vm_space = VmSpace::new();
        let map_range = PAGE_SIZE..(PAGE_SIZE * 2);
        let cow_range = 0..PAGE_SIZE * 512 * 512;
        let page_property = PageProperty::new_user(PageFlags::RW, CachePolicy::Writeback);
        let preempt_guard = disable_preempt();

        // Allocates and maps a frame.
        let frame = FrameAllocOptions::default().alloc_frame().unwrap();
        let start_paddr = frame.start_paddr();
        let frame_clone_for_assert = frame.clone();

        vm_space
            .cursor_mut(&preempt_guard, &map_range)
            .unwrap()
            .map(frame.into(), page_property); // Original frame moved here

        // Confirms the initial mapping.
        assert!(matches!(
            vm_space.cursor(&preempt_guard, &map_range).unwrap().query().unwrap(),
            (va, Some((frame, prop))) if va.start == map_range.start && frame.start_paddr() == start_paddr && prop.flags == PageFlags::RW
        ));

        // Creates a child page table with copy-on-write protection.
        let child_space = VmSpace::new();
        {
            let mut child_cursor = child_space.cursor_mut(&preempt_guard, &cow_range).unwrap();
            let mut parent_cursor = vm_space.cursor_mut(&preempt_guard, &cow_range).unwrap();
            let num_copied = cow_copy_pt(&mut parent_cursor, &mut child_cursor, cow_range.len());
            assert_eq!(num_copied, 1); // Only one page should be copied
        };

        // Confirms that parent and child VAs map to the same physical address.
        {
            let child_map_frame_addr = {
                let (_, Some((frame, _))) = child_space
                    .cursor(&preempt_guard, &map_range)
                    .unwrap()
                    .query()
                    .unwrap()
                else {
                    panic!("Child mapping query failed");
                };
                frame.start_paddr()
            };
            let parent_map_frame_addr = {
                let (_, Some((frame, _))) = vm_space
                    .cursor(&preempt_guard, &map_range)
                    .unwrap()
                    .query()
                    .unwrap()
                else {
                    panic!("Parent mapping query failed");
                };
                frame.start_paddr()
            };
            assert_eq!(child_map_frame_addr, parent_map_frame_addr);
            assert_eq!(child_map_frame_addr, start_paddr);
        }

        // Unmaps the range from the parent.
        vm_space
            .cursor_mut(&preempt_guard, &map_range)
            .unwrap()
            .unmap(map_range.len());

        // Confirms that the child VA remains mapped.
        assert!(matches!(
            child_space.cursor(&preempt_guard, &map_range).unwrap().query().unwrap(),
            (va, Some((frame, prop)))  if va.start == map_range.start && frame.start_paddr() == start_paddr && prop.flags == PageFlags::R
        ));

        // Creates a sibling page table (from the now-modified parent).
        let sibling_space = VmSpace::new();
        {
            let mut sibling_cursor = sibling_space
                .cursor_mut(&preempt_guard, &cow_range)
                .unwrap();
            let mut parent_cursor = vm_space.cursor_mut(&preempt_guard, &cow_range).unwrap();
            let num_copied = cow_copy_pt(&mut parent_cursor, &mut sibling_cursor, cow_range.len());
            assert_eq!(num_copied, 0); // No pages should be copied
        }

        // Verifies that the sibling is unmapped as it was created after the parent unmapped the range.
        assert!(matches!(
            sibling_space
                .cursor(&preempt_guard, &map_range)
                .unwrap()
                .query()
                .unwrap(),
            (_, None)
        ));

        // Drops the parent page table.
        drop(vm_space);

        // Confirms that the child VA remains mapped after the parent is dropped.
        assert!(matches!(
            child_space.cursor(&preempt_guard, &map_range).unwrap().query().unwrap(),
            (va, Some((frame, prop)))  if va.start == map_range.start && frame.start_paddr() == start_paddr && prop.flags == PageFlags::R
        ));

        // Unmaps the range from the child.
        child_space
            .cursor_mut(&preempt_guard, &map_range)
            .unwrap()
            .unmap(map_range.len());

        // Maps the range in the sibling using the third clone.
        sibling_space
            .cursor_mut(&preempt_guard, &map_range)
            .unwrap()
            .map(frame_clone_for_assert.into(), page_property);

        // Confirms that the sibling mapping points back to the original frame's physical address.
        assert!(matches!(
            sibling_space.cursor(&preempt_guard, &map_range).unwrap().query().unwrap(),
            (va, Some((frame, prop)))  if va.start == map_range.start && frame.start_paddr() == start_paddr && prop.flags == PageFlags::RW
        ));

        // Confirms that the child remains unmapped.
        assert!(matches!(
            child_space
                .cursor(&preempt_guard, &map_range)
                .unwrap()
                .query()
                .unwrap(),
            (_, None)
        ));
    }
}
