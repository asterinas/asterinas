// SPDX-License-Identifier: MPL-2.0

use alloc::{
    boxed::Box,
    collections::BTreeMap,
    sync::{Arc, Weak},
};
use core::ops::Bound;

use aster_core::{fs::file::MappedObject, prelude::*};
use ostd::mm::PAGE_SIZE;

use crate::gem::object::DrmGemObject;

/// A userspace-visible byte offset in the fake GEM mmap-offset space.
pub(super) type DrmMmapOffset = u64;

// The fake mmap-offset address space mirrors Linux's DRM VMA manager.
// It starts above offsets that may represent positions in a real file,
// and reserves a larger, architecture-dependent range for GEM object mappings.
//
// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/drm/drm_vma_manager.h#L32-L42>.
const DRM_MMAP_OFFSET_START_PAGE: u64 = ((u32::MAX as u64) / PAGE_SIZE as u64) + 1;
const DRM_MMAP_OFFSET_PAGE_COUNT: u64 = ((u32::MAX as u64) / PAGE_SIZE as u64) * 256;

/// The device-wide mmap-offset namespace for GEM objects.
///
/// The offsets are handle-like tokens rather than positions in a real file.
#[derive(Debug)]
pub(crate) struct DrmGemMmapOffsetSpace {
    free_ranges: BTreeMap<u64, u64>,
    allocated_ranges: BTreeMap<u64, DrmMmapOffsetAllocation>,
}

impl Default for DrmGemMmapOffsetSpace {
    fn default() -> Self {
        let mut free_ranges = BTreeMap::new();

        free_ranges.insert(DRM_MMAP_OFFSET_START_PAGE, DRM_MMAP_OFFSET_PAGE_COUNT);

        Self {
            free_ranges,
            allocated_ranges: BTreeMap::new(),
        }
    }
}

impl DrmGemMmapOffsetSpace {
    /// Returns an object's mmap offset, allocating its range if necessary.
    pub(crate) fn get_or_allocate_offset(&mut self, gem_object: &Arc<DrmGemObject>) -> Result<u64> {
        if let Some(offset) = gem_object.mmap_offset() {
            return Ok(offset);
        }

        self.reclaim_dead_ranges();

        let size = gem_object.size();
        // All supported targets have pointer widths no greater than 64 bits.
        let page_count = (size / PAGE_SIZE) as u64;

        let (best_start, best_len) = self
            .free_ranges
            .iter()
            .filter(|(_, len)| **len >= page_count)
            .min_by_key(|(_, len)| **len)
            .map(|(start, len)| (*start, *len))
            .ok_or(Errno::ENOMEM)?;

        self.free_ranges.remove(&best_start);
        if best_len > page_count {
            let remaining_start = best_start + page_count;
            let remaining_len = best_len - page_count;
            self.free_ranges.insert(remaining_start, remaining_len);
        }

        let allocation = DrmMmapOffsetAllocation {
            page_count,
            gem_object: Arc::downgrade(gem_object),
        };

        let offset = best_start * PAGE_SIZE as u64;
        gem_object.set_mmap_offset(offset);
        self.allocated_ranges.insert(best_start, allocation);

        Ok(offset)
    }

    /// Resolves a range in the fake mmap-offset space and creates its mapped object.
    ///
    /// This validates the range and the client's access before delegating to the
    /// owning GEM object.
    pub(crate) fn create_mapped_object(
        &self,
        client_id: u64,
        offset: usize,
        size: usize,
    ) -> Result<Box<dyn MappedObject>> {
        // All supported targets have pointer widths no greater than 64 bits,
        // so converting page counts from `usize` to `u64` is lossless.
        let start_page = (offset / PAGE_SIZE) as u64;
        let page_count = (size / PAGE_SIZE) as u64;
        if page_count == 0 {
            return_errno_with_message!(Errno::EINVAL, "the GEM mapping size must not be zero");
        }
        let end_page = start_page + page_count;

        let Some((allocated_start_page, allocated_range)) = self
            .allocated_ranges
            .range(..=start_page)
            .next_back()
            .map(|(start_page, range)| (*start_page, range))
        else {
            return_errno_with_message!(
                Errno::EINVAL,
                "the mmap range does not belong to a GEM object"
            );
        };
        let range_end_page = allocated_start_page + allocated_range.page_count;
        if end_page > range_end_page {
            return_errno_with_message!(
                Errno::EINVAL,
                "the mmap range does not belong to a GEM object"
            );
        }

        let Some(gem_object) = allocated_range.gem_object.upgrade() else {
            return_errno_with_message!(Errno::EINVAL, "the GEM object no longer exists");
        };
        if !gem_object.is_mmap_allowed(client_id) {
            return_errno_with_message!(Errno::EACCES, "the GEM object is not accessible");
        }

        let object_page_offset = (start_page - allocated_start_page) as usize;
        let object_offset = object_page_offset * PAGE_SIZE;
        gem_object.create_mapped_object(object_offset, size)
    }

    fn reclaim_dead_ranges(&mut self) {
        let free_ranges = &mut self.free_ranges;

        for (start_page, allocation) in self.allocated_ranges.extract_if(.., |_, allocation| {
            allocation.gem_object.strong_count() == 0
        }) {
            let mut free_start = start_page;
            let mut free_end = start_page + allocation.page_count;

            if let Some((prev_start, prev_len)) = free_ranges
                .range(..free_start)
                .next_back()
                .map(|(start, len)| (*start, *len))
            {
                let prev_end = prev_start + prev_len;
                if prev_end == free_start {
                    free_ranges.remove(&prev_start);
                    free_start = prev_start;
                }
            }

            if let Some((next_start, next_len)) = free_ranges
                .range((Bound::Excluded(free_start), Bound::Unbounded))
                .next()
                .map(|(start, len)| (*start, *len))
                && free_end == next_start
            {
                free_ranges.remove(&next_start);
                free_end += next_len;
            }

            free_ranges.insert(free_start, free_end - free_start);
        }
    }
}

/// An allocation whose start page is the corresponding `allocated_ranges` key.
#[derive(Debug)]
struct DrmMmapOffsetAllocation {
    page_count: u64,
    gem_object: Weak<DrmGemObject>,
}
