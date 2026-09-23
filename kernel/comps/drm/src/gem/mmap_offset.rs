// SPDX-License-Identifier: MPL-2.0

use alloc::{
    collections::BTreeMap,
    sync::{Arc, Weak},
    vec::Vec,
};
use core::ops::Bound;

use aster_core::prelude::*;
use ostd::mm::PAGE_SIZE;

use crate::gem::object::DrmGemObject;

// The fake mmap-offset address space mirrors Linux's DRM VMA manager.
// It starts above offsets that may represent positions in a real file,
// and reserves a larger, architecture-dependent range for GEM object mappings.
//
// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/drm/drm_vma_manager.h#L32-L42>.
const DRM_MMAP_OFFSET_START_PAGE: u64 = ((u32::MAX as u64) / PAGE_SIZE as u64) + 1;
const DRM_MMAP_OFFSET_PAGE_COUNT: u64 = ((u32::MAX as u64) / PAGE_SIZE as u64) * 256;

#[derive(Debug)]
struct DrmMmapOffsetRange {
    start_page: u64,
    page_count: u64,
    gem_object: Weak<DrmGemObject>,
}

/// The device-wide mmap-offset namespace for GEM objects.
///
/// The offsets are handle-like tokens rather than positions in a real file.
#[derive(Debug)]
pub(crate) struct DrmGemMmapOffsetSpace {
    free_ranges: BTreeMap<u64, u64>,
    allocated_ranges: BTreeMap<u64, DrmMmapOffsetRange>,
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
    /// Allocates a fake mmap-offset range for an object if it has none.
    pub(crate) fn ensure_allocated(&mut self, gem_object: &Arc<DrmGemObject>) -> Result<()> {
        if gem_object.has_mmap_offset() {
            return Ok(());
        }

        let size = gem_object.size();
        // All supported targets have pointer widths no greater than 64 bits.
        let page_count = (size / PAGE_SIZE) as u64;

        self.reclaim_dead_objects();

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

        let offset_range = DrmMmapOffsetRange {
            start_page: best_start,
            page_count,
            gem_object: Arc::downgrade(gem_object),
        };

        gem_object.set_mmap_offset_start_page(offset_range.start_page);
        self.allocated_ranges
            .insert(offset_range.start_page, offset_range);
        Ok(())
    }

    /// Looks up the object that fully covers the requested page range.
    pub(crate) fn lookup(
        &self,
        start_page: u64,
        page_count: u64,
    ) -> Option<(Arc<DrmGemObject>, usize)> {
        if page_count == 0 {
            return None;
        }
        let end_page = start_page.checked_add(page_count)?;

        let (allocated_start_page, allocated_range) = self
            .allocated_ranges
            .range(..=start_page)
            .next_back()
            .map(|(start_page, range)| (*start_page, range))?;
        let gem_object = allocated_range.gem_object.upgrade()?;
        let range_end_page = allocated_start_page + allocated_range.page_count;
        if end_page <= range_end_page {
            let object_page_offset = (start_page - allocated_start_page) as usize;
            return Some((gem_object, object_page_offset));
        }

        None
    }

    fn reclaim_dead_objects(&mut self) {
        let dead_ranges = self
            .allocated_ranges
            .extract_if(.., |_, range| range.gem_object.strong_count() == 0)
            .map(|(start_page, range)| (start_page, range.page_count))
            .collect::<Vec<_>>();

        for (start_page, page_count) in dead_ranges {
            self.free_range(start_page, page_count);
        }
    }

    fn free_range(&mut self, start_page: u64, page_count: u64) {
        let mut free_start = start_page;
        let mut free_end = start_page + page_count;

        if let Some((prev_start, prev_len)) = self
            .free_ranges
            .range(..free_start)
            .next_back()
            .map(|(start, len)| (*start, *len))
        {
            let prev_end = prev_start + prev_len;
            if prev_end == free_start {
                self.free_ranges.remove(&prev_start);
                free_start = prev_start;
            }
        }

        if let Some((next_start, next_len)) = self
            .free_ranges
            .range((Bound::Excluded(free_start), Bound::Unbounded))
            .next()
            .map(|(start, len)| (*start, *len))
            && free_end == next_start
        {
            self.free_ranges.remove(&next_start);
            free_end += next_len;
        }

        self.free_ranges.insert(free_start, free_end - free_start);
    }
}
