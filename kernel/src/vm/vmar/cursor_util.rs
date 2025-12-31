// SPDX-License-Identifier: MPL-2.0

//! The utility functions for VMAR cursor navigation.

use core::ops::Range;

use align_ext::AlignExt;
use ostd::mm::{Vaddr, page_size_at, vm_space::VmQueriedItem};

use super::vmar_impls::{PteRangeMeta, VmarCursorMut};
use crate::vm::vmar::{
    interval_set::Interval, util::get_intersected_range, vm_mapping::VmMapping,
    vmar_impls::RsAsDelta,
};

/// Finds the first mapped [`VmMapping`] that overlaps the given range.
///
/// The given range is specified by the current virtual address of the cursor
/// and the `end`. If there is any, it returns the first mapped [`VmMapping`]
/// that intersects with the given range. And the cursor will be at that
/// mapping's level.
///
/// The cursor's virtual address is monotonically increasing but may not be at
/// the start of the returned [`VmMapping`].
///
/// # Why a Macro?
///
/// It could have been a function with the following signature:
///
/// ```ignore
/// pub(super) fn find_next_mapped<'a, 'rcu>(
///     cursor: &'a mut VmarCursorMut<'rcu>,
///     end: Vaddr,
/// ) -> Option<&'a VmMapping>
/// ```
///
/// But the borrow checker is unreasonably unhappy. Using a macro magically
/// avoids this issue.
macro_rules! find_next_mapped {
    ($cursor:expr, $end:expr) => {
        loop {
            use crate::vm::vmar::interval_set::Interval;

            let cur_va = $cursor.virt_addr();
            if cur_va >= $end {
                break None;
            }

            let Some(meta) = $cursor.aux_meta().inner.find_next(&cur_va) else {
                if $cursor.level() == $cursor.guard_level() {
                    break None;
                } else {
                    $cursor.pop_level();
                    let next_addr = $cursor.cur_va_range().end;
                    if next_addr >= $end || $cursor.jump(next_addr).is_err() {
                        break None;
                    }
                    continue;
                }
            };
            if meta.range().start >= $end {
                break None;
            }
            match meta {
                $crate::vm::vmar::vmar_impls::PteRangeMeta::ChildPt(r) => {
                    let start = r.start;
                    if cur_va < start {
                        $cursor.jump(start).unwrap();
                    }
                    $cursor.push_level_if_exists().unwrap();
                    continue;
                }
                $crate::vm::vmar::vmar_impls::PteRangeMeta::VmMapping(vm_mapping) => {
                    break Some(vm_mapping);
                }
            }
        }
    };
}

pub(super) use find_next_mapped;

/// Finds and takes the next unmappable auxiliary metadata in the given range.
///
/// If there's any metadata that overlaps the given range, it splits and takes
/// the overlapping part and returns it. The cursor's virtual address will be
/// at the start of the taken metadata.
pub(super) fn take_next_unmappable(
    cursor: &mut VmarCursorMut<'_>,
    end: Vaddr,
) -> Option<PteRangeMeta> {
    let start = cursor.virt_addr();
    while cursor.virt_addr() < end {
        let cur_va = cursor.virt_addr();
        let Some(ref meta) = cursor.aux_meta_mut().inner.find_next(&cur_va) else {
            if cursor.level() == cursor.guard_level() {
                return None;
            } else {
                cursor.pop_level();
                let next_addr = cursor.cur_va_range().end;
                if next_addr >= end || cursor.jump(next_addr).is_err() {
                    break;
                }
                continue;
            }
        };
        if meta.range().start >= end {
            return None;
        }
        match meta {
            PteRangeMeta::ChildPt(r) => {
                let range = r.clone();
                cursor.jump(range.start.max(start)).unwrap();
                if start <= range.start && range.end <= end {
                    return cursor.aux_meta_mut().inner.take_one(&range.start);
                } else {
                    cursor.push_level_if_exists().unwrap();
                    continue;
                }
            }
            PteRangeMeta::VmMapping(vm_mapping) => {
                let range = vm_mapping.range();
                cursor.jump(range.start.max(start)).unwrap();
                if start <= range.start && range.end <= end {
                    let taken = cursor
                        .aux_meta_mut()
                        .inner
                        .take_one(&range.start)
                        .unwrap()
                        .unwrap_mapping();
                    return Some(PteRangeMeta::VmMapping(taken));
                } else {
                    let intersected_range = get_intersected_range(&(start..end), &range);
                    propagate_if_needed(cursor, intersected_range.len());

                    let vm_mapping = cursor
                        .aux_meta_mut()
                        .inner
                        .take_one(&range.start)
                        .unwrap()
                        .unwrap_mapping();

                    let taken = split_and_insert_rest(cursor, vm_mapping, intersected_range);
                    return Some(PteRangeMeta::VmMapping(taken));
                }
            }
        }
    }

    None
}

/// Count all the mapped pages in the subtree and update the RSS & AS as if
/// doing unmapping.
///
/// The cursor must be at a PTE pointing to a child page table.s
///
/// If `allocator` is provided, also deallocates the unmapped ranges to it.
///
/// Also returns the total number of frames unmapped for checking purposes.
pub(super) fn unmap_count_rs_as(
    cursor: &mut VmarCursorMut<'_>,
    end: Vaddr,
    rs_as_delta: &mut RsAsDelta,
) -> usize {
    let old_va = cursor.virt_addr();
    let old_level = cursor.level();

    let mut total_frames_unmapped = 0;

    while let Some(vm_mapping) = find_next_mapped!(cursor, end) {
        let vm_mapping_range = vm_mapping.range();
        let frames_mapped = vm_mapping.frames_mapped();
        let rss_type = vm_mapping.rss_type();

        #[cfg_attr(not(debug_assertions), expect(unused_mut))]
        let mut count_frames_mapped = || {
            let mut mapped_frames = 0;
            let cur_page_size = page_size_at(cursor.level());
            for va in vm_mapping_range.clone().step_by(cur_page_size) {
                cursor.jump(va).unwrap();
                match cursor.query() {
                    VmQueriedItem::MappedRam { .. } => {
                        mapped_frames += 1;
                    }
                    VmQueriedItem::MappedIoMem { .. } => {
                        mapped_frames += cur_page_size / page_size_at(1);
                    }
                    VmQueriedItem::PageTable => {
                        panic!("found page table under VM mapping at {:#x}", va);
                    }
                    VmQueriedItem::None => {}
                }
            }
            mapped_frames
        };

        #[cfg(debug_assertions)]
        if let Some(frames_mapped) = frames_mapped {
            let counted_bytes = count_frames_mapped();
            debug_assert_eq!(frames_mapped, counted_bytes);
        }

        let frames_mapped = frames_mapped.unwrap_or_else(count_frames_mapped);

        rs_as_delta.add_rs(rss_type, -(frames_mapped as isize));
        rs_as_delta.sub_as(vm_mapping_range.len());

        total_frames_unmapped += frames_mapped;

        if cursor.jump(vm_mapping_range.end).is_err() {
            break;
        }
    }

    cursor.jump(old_va).unwrap();
    cursor.adjust_level(old_level);

    total_frames_unmapped
}

/// Propagates the huge [`VmMapping`] and the PTE at the current VA if
///  - the start is not aligned, or
///  - the end is inside the PTE's range.
pub(super) fn propagate_if_needed(cursor: &mut VmarCursorMut<'_>, len: usize) {
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
///
/// And if the end is not aligned, this function only extracts the aligned part.
pub(super) fn split_and_insert_rest(
    cursor: &mut VmarCursorMut<'_>,
    vm_mapping: VmMapping,
    split: Range<Vaddr>,
) -> VmMapping {
    let cur_page_size = page_size_at(cursor.level());

    debug_assert!(split.start.is_multiple_of(cur_page_size));

    let aligned_range = split.start..split.end.align_down(cur_page_size);

    debug_assert!(!aligned_range.is_empty());

    let (left, taken, right) = vm_mapping.split_range(&aligned_range);

    if let Some(left) = left {
        cursor.aux_meta_mut().insert_without_try_merge(left);
    }
    if let Some(right) = right {
        cursor.aux_meta_mut().insert_without_try_merge(right);
    }

    taken
}

/// Checks whether the given range is fully mapped.
///
/// If fully mapped, returns the reference to the last mapping (split at page
/// table boundaries).
///
/// # Why a Macro?
///
/// It could be a function with the following signature:
///
/// ```ignore
/// pub(super) fn check_range_mapped(
//     cursor: &mut VmarCursorMut<'_>,
///     end: Vaddr,
/// ) -> Result<&VmMapping>;
/// ```
///
/// But the borrow checker is unreasonably unhappy. Using a macro magically
/// avoids this issue.
macro_rules! check_range_mapped {
    ($cursor:expr, $end:expr) => {{
        use $crate::{
            error::{Errno, Error},
            prelude::*,
            vm::vmar::{VmMapping, cursor_util::find_next_mapped},
        };

        const ERR: Result<&VmMapping> = Err(Error::with_message(
            Errno::EFAULT,
            "the range is not fully mapped",
        ));
        let start = $cursor.virt_addr();

        let mut last_end = None;

        loop {
            let Some(vm_mapping) = find_next_mapped!($cursor, $end) else {
                break ERR;
            };

            if let Some(last_end) = last_end
                && last_end != vm_mapping.map_to_addr()
            {
                debug_assert!(last_end < vm_mapping.map_to_addr());
                break ERR;
            } else if last_end.is_none() && vm_mapping.map_to_addr() > start {
                break ERR;
            }

            let map_end = vm_mapping.map_end();
            if map_end >= $end {
                break Ok(vm_mapping);
            }

            last_end = Some(map_end);
            $cursor.jump(map_end).unwrap();
        }
    }};
}

pub(super) use check_range_mapped;

/// Checks whether the given range is fully mapped.
///
/// If fully mapped within one [`VmMapping`], returns the start address of the
/// last mapping (split at page table boundaries).
///
/// # Why a Macro?
///
/// It could be a function with the following signature:
///
/// ```ignore
/// pub(super) fn check_range_within_one_mapping(
///     cursor: &mut VmarCursorMut<'_>,
///     end: Vaddr,
/// ) -> Result<&VmMapping>;
/// ```
///
/// But the borrow checker is unreasonably unhappy. Using a macro magically
/// avoids this issue.
macro_rules! check_range_within_one_mapping {
    ($cursor:expr, $end:expr) => {{
        use $crate::{
            error::{Errno, Error},
            prelude::*,
            vm::vmar::{VmMapping, cursor_util::find_next_mapped},
        };

        const ERR: Result<&VmMapping> = Err(Error::with_message(
            Errno::EFAULT,
            "the range is not covered by a single mapping",
        ));
        let start = $cursor.virt_addr();

        let mut last: Option<VmMapping> = None;

        loop {
            let Some(vm_mapping) = find_next_mapped!($cursor, $end) else {
                break ERR;
            };

            if let Some(last) = last {
                if !last.can_merge_with(vm_mapping) {
                    break ERR;
                }
            } else if vm_mapping.map_to_addr() > start {
                break ERR;
            }

            let map_end = vm_mapping.map_end();
            if map_end >= $end {
                break Ok(vm_mapping);
            }

            last = Some(vm_mapping.clone_for_check());
            $cursor.jump(map_end).unwrap();
        }
    }};
}

pub(super) use check_range_within_one_mapping;

/// #Panics
///
/// Panics if the range contains [`VmMapping`] or mapped pages.
#[cfg(any(debug_assertions, ktest))]
pub fn check_range_not_mapped(cursor: &mut VmarCursorMut, range: Range<Vaddr>) {
    cursor.jump(range.start).unwrap();
    if let Some(vm_mapping) = find_next_mapped!(cursor, range.end) {
        panic!(
            "existing VM mapping found in range {:#x?} to clear: {:#x?}",
            range, vm_mapping
        );
    }
    cursor.jump(range.start).unwrap();
    if let Some(va) = cursor.find_next(range.len()) {
        panic!(
            "existing page table mapping found in range {:#x?} to clear: {:#x?}",
            range, va
        );
    }
}
