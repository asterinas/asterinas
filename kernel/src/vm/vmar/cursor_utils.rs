// SPDX-License-Identifier: MPL-2.0

//! The utility functions for VMAR cursor navigation.

use core::range;

use ostd::mm::{Vaddr, vm_space::CursorMut};

use super::{Interval, PerPtMeta, PteRangeMeta, VmMapping, VmarCursor};
use crate::vm::vmar::RssDelta;

/// Finds the next mapped `VmMapping` in the given range.
///
/// The given range is specified by the current virtual address of the cursor
/// and the length `len`. If there is any, it returns the first mapped
/// [`VmMapping`] that intersects with the given range. And the cursor will be
/// at that mapping's level.
///
/// The cursor's virtual address is monotonically increasing but may not be at
/// the start of the returned [`VmMapping`].
pub(super) fn find_next_mapped(cursor: &mut VmarCursor<'_>, len: usize) -> Option<&VmMapping> {
    let end = cursor.virt_addr() + len;
    while cursor.virt_addr() < end {
        let cur_va = cursor.virt_addr();
        let Some(ref meta) = cursor.aux_meta().inner.find_next(&cur_va) else {
            if cursor.level() == cursor.guard_level() {
                return None;
            } else {
                cursor.pop_level();
                continue;
            }
        };
        if meta.range().start >= end {
            return None;
        }
        match meta {
            PteRangeMeta::ChildPt(r) => {
                if cur_va < r.start {
                    cursor.jump(r.start);
                }
                cursor.push_level_if_exists().unwrap();
                continue;
            }
            PteRangeMeta::VmMapping(vm_mapping) => {
                return Some(vm_mapping);
            }
        }
    }
}

/// Like [`find_next_mapped`], but finds the next unmappable subtree instead.
pub(super) fn find_next_unmappable(
    cursor: &mut VmarCursor<'_>,
    len: usize,
) -> Option<Range<Vaddr>> {
    let end = cursor.virt_addr() + len;
    while cursor.virt_addr() < end {
        let cur_va = cursor.virt_addr();
        let Some(ref meta) = cursor.aux_meta().inner.find_next(&cur_va) else {
            if cursor.level() == cursor.guard_level() {
                return None;
            } else {
                cursor.pop_level();
                continue;
            }
        };
        if meta.range().start >= end {
            return None;
        }
        match meta {
            PteRangeMeta::ChildPt(r) => {
                return r.clone();
            }
            PteRangeMeta::VmMapping(vm_mapping) => {
                return Some(vm_mapping.range());
            }
        }
    }
}

/// Count all the mapped pages and update the RSS as if doing unmapping.
pub(super) fn unmap_count_rss(cursor: &mut VmarCursor<'_>, len: usize, rss_delta: &mut RssDelta) {
    let end = cursor.virt_addr() + len;
    while let Some(vm_mapping) = find_next_mapped(cursor, end - cursor.virt_addr()) {
        let vm_mapping_range = vm_mapping.range();
        let bytes_mapped = vm_mapping.bytes_mapped().unwrap_or_else(|| {
            let mut mapped_bytes = 0;
            let cur_page_size = page_size_at(cursor.level());
            for va in vm_mapping_range.step_by(cur_page_size) {
                cursor.jump(va).unwrap();
                if cursor.query().is_some() {
                    mapped_bytes += cur_page_size;
                }
            }
            mapped_bytes
        });
        rss_delta.add(vm_mapping.rss_type(), -(bytes_mapped as isize));
    }
}

/// Propagates the huge [`VmMapping`] and the PTE at the current VA if
///  - the start is not aligned, or
///  - the end is inside the PTE's range.
pub(super) fn propagate_if_needed(cursor: &mut VmarCursor<'_>, len: usize) {
    let start = cursor.virt_addr();
    let end = start + len;

    while !start.is_multiple_of(page_size_at(cursor.level())) || end < cursor.cur_va_range().end {
        cursor.adjust_level(cursor.level() - 1);
    }
}

/// Splits the `split` range part from the given [`VmMapping`] and puts the
/// rest back to current PT's auxiliary metadata.
///
/// This function assumes that
///  1. the provided [`VmMapping`] is taken from current PT's auxiliary
///     metadata, so no merging is needed when inserting back the rest parts;
///  2. [`propagate_if_needed`] has been called before, so that the start of
///     `split` is aligned to the current level's page size.
/// And if the end is not aligned, this function only extracts the aligned part.
pub(super) fn split_and_insert_rest(
    cursor: &mut VmarCursor<'_>,
    vm_mapping: VmMapping,
    split: Range<Vaddr>,
) -> VmMapping {
    let cur_page_size = page_size_at(cursor.level());

    debug_assert!(split.start.is_multiple_of(cur_page_size));

    let aligned_range = split.start..split.end.align_down(cur_page_size);

    debug_assert!(!aligned_range.is_empty());

    let (left, taken, right) = vm_mapping.split_range(&aligned_range);

    if let Some(left) = left {
        cursor.aux_meta().insert_without_try_merge(left);
    }
    if let Some(right) = right {
        cursor.aux_meta().insert_without_try_merge(right);
    }

    taken
}
