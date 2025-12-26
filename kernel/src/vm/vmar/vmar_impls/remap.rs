// SPDX-License-Identifier: MPL-2.0

use ostd::{mm::vm_space::VmQueriedItem, task::disable_preempt};

use super::{RssDelta, Vmar, util::is_intersected};
use crate::{prelude::*, vm::vmar::is_userspace_vaddr_range};

impl Vmar {
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
    ///
    /// [`VmMapping`]: crate::vm::vmar::vm_mapping::VmMapping
    pub fn resize_mapping(
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
    ///
    /// # Panics
    ///
    /// This method panics if `new_addr` is `None` and `new_size <= old_size`.
    /// Use `resize_mapping` instead in this case.
    pub fn remap(
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

        let Some(old_mapping) = inner.vm_mappings.find_one(&old_addr) else {
            return_errno_with_message!(
                Errno::EFAULT,
                "remap: there is no mapping at the old address"
            )
        };
        if new_size > old_size {
            if !old_mapping.can_expand() {
                return_errno_with_message!(
                    Errno::EFAULT,
                    "remap: device mappings cannot be expanded"
                );
            }
            inner.check_extra_size_fits_rlimit(new_size - old_size)?;
        }

        // Shrink the old mapping first.
        if old_addr.checked_add(old_size).is_none() {
            return_errno_with_message!(Errno::EINVAL, "remap: the address range overflows");
        }
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
            if !new_addr.is_multiple_of(PAGE_SIZE) || !is_userspace_vaddr_range(new_addr, new_size)
            {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "remap: the new range is not aligned or not in userspace"
                );
            }
            if is_intersected(&old_range, &(new_addr..new_addr + new_size)) {
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
            debug_assert!(new_size > old_size);

            // Fast path: expand the old mapping in place to the new size
            if inner
                .alloc_free_region_exact(old_range.end, new_size - old_size)
                .is_ok()
            {
                let old_mapping_addr = inner.check_lies_in_single_mapping(old_addr, old_size)?;
                let old_mapping = inner.remove(&old_mapping_addr).unwrap();
                let new_mapping = old_mapping.enlarge(new_size - old_size);
                inner.insert_try_merge(new_mapping);
                return Ok(old_range.start);
            }

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
        let new_mapping = old_mapping.clone_for_remap_at(new_range.start);
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
            let (va, Some(item)) = cursor.query().unwrap() else {
                panic!("Found mapped page but query failed");
            };
            debug_assert_eq!(mapped_va, va.start);
            cursor.unmap(PAGE_SIZE);

            let offset = mapped_va - old_range.start;
            cursor.jump(new_range.start + offset).unwrap();

            match item {
                VmQueriedItem::MappedRam { frame, prop } => {
                    cursor.map(frame, prop);
                }
                VmQueriedItem::MappedIoMem { paddr, prop } => {
                    // For MMIO pages, find the corresponding `IoMem` and map it
                    // at the new location
                    let (iomem, offset) = cursor.find_iomem_by_paddr(paddr).unwrap();
                    cursor.map_iomem(iomem, prop, PAGE_SIZE, offset);
                }
            }

            current_offset = offset + PAGE_SIZE;
        }

        cursor.flusher().dispatch_tlb_flush();
        cursor.flusher().sync_tlb_flush();

        Ok(new_range.start)
    }
}
