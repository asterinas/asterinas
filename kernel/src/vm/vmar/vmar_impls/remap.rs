// SPDX-License-Identifier: MPL-2.0

use core::ops::Range;

use ostd::{
    mm::{
        CachePolicy, HIGHEST_PAGING_LEVEL, PageFlags, PageProperty, page_size_at,
        vm_space::VmQueriedItem,
    },
    task::disable_preempt,
};

use super::{PteRangeMeta, RsAsDelta, Vmar, VmarCursorMut};
use crate::{
    prelude::*,
    vm::vmar::{
        cursor_util::{
            check_range_mapped, check_range_within_one_mapping, find_next_mapped,
            propagate_if_needed, split_and_insert_rest,
        },
        interval_set::Interval,
        util::{get_intersected_range, is_intersected},
        vm_allocator::{PerCpuAllocator, PerCpuAllocatorGuard},
        vm_mapping::{MappedMemory, VmMapping},
    },
};

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
    pub fn resize_mapping(
        &self,
        map_addr: Vaddr,
        old_size: usize,
        new_size: usize,
        check_single_mapping: bool,
    ) -> Result<()> {
        debug_assert!(map_addr.is_multiple_of(PAGE_SIZE));
        debug_assert!(old_size.is_multiple_of(PAGE_SIZE));
        debug_assert!(new_size.is_multiple_of(PAGE_SIZE));

        let mut rs_as_delta = RsAsDelta::new(self);

        let preempt_guard = disable_preempt();

        let range_to_lock = map_addr
            ..map_addr
                .checked_add(old_size.max(new_size))
                .ok_or(Errno::EINVAL)?;

        // We need to lock the allocator before locking the address space.
        // At most one of them is `Some`.
        let mut dealloc_guard = (new_size < old_size).then_some(self.allocator.lock_for_dealloc());
        let alloc_guard = (new_size > old_size).then_some(
            self.allocator
                .alloc_specific(map_addr + old_size..map_addr + new_size)
                .map_err(|_| {
                    Error::with_message(
                        Errno::ENOMEM,
                        "resize_mapping: not enough space to expand the mapping",
                    )
                })?,
        );

        let mut cursor = self
            .vm_space
            .cursor_mut(&preempt_guard, &range_to_lock)
            .unwrap();

        drop(alloc_guard); // No need for allocator locks after cursor locks.

        let last_mapping = if check_single_mapping {
            check_range_within_one_mapping!(cursor, map_addr + old_size)?
        } else {
            check_range_mapped!(cursor, map_addr + old_size)?
        };

        if new_size == old_size {
            return Ok(());
        }

        if new_size < old_size {
            cursor.jump(map_addr + new_size).unwrap();
            self.remove_mappings(
                &mut cursor,
                old_size - new_size,
                &mut rs_as_delta,
                dealloc_guard.as_mut(),
            )?;
            return Ok(());
        }

        if !last_mapping.can_expand() {
            return_errno_with_message!(
                Errno::EINVAL,
                "resize_mapping: the mapping cannot be expanded"
            );
        }

        if last_mapping.map_end() > map_addr + old_size {
            return_errno_with_message!(
                Errno::ENOMEM,
                "resize_mapping: the mapping cannot be expanded to overlap itself"
            );
        }

        let last_mapping_addr = last_mapping.map_to_addr();
        cursor.jump(last_mapping_addr).unwrap();
        while cursor.push_level_if_exists().is_some() {}

        let vm_mapping = cursor
            .aux_meta_mut()
            .inner
            .take_one(&last_mapping_addr)
            .unwrap()
            .unwrap_mapping();

        let new_mapping = vm_mapping.enlarge(new_size - old_size);

        super::map::map_to_page_table(&mut cursor, new_mapping);

        Ok(())
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
    ///
    /// # Debug Assertions
    ///
    /// This method assumes that all addresses and sizes are page-aligned.
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
        if let Some(new_addr) = new_addr {
            debug_assert_eq!(new_addr % PAGE_SIZE, 0);
        }

        let preempt_guard = disable_preempt();

        let mut remap_op =
            AllocatedRemapOp::alloc_from(&self.allocator, old_addr, old_size, new_addr, new_size)?;

        let mut rs_as_delta = RsAsDelta::new(self);

        let mut cursor = self
            .vm_space
            .cursor_mut(&preempt_guard, &remap_op.lock_range)
            .unwrap();

        // Linux Compatibility: Linux unmaps the destination range/shrinks the
        // old range before checking if the source range is covered by a single
        // mapping.
        remap_op.perform_overwrite_and_shrink(self, &mut cursor, &mut rs_as_delta)?;

        cursor.jump(old_addr).unwrap();
        let last_mapping =
            match check_range_within_one_mapping!(cursor, old_addr + old_size.min(new_size)) {
                Ok(last_mapping) => last_mapping,
                Err(e) => {
                    remap_op.dealloc_if_fail();
                    return Err(e);
                }
            };

        if !last_mapping.can_expand() {
            remap_op.dealloc_if_fail();
            return_errno_with_message!(Errno::EINVAL, "remap: the mapping cannot be expanded");
        }

        remap_op.perform_move_and_extend(&mut cursor, &mut rs_as_delta)
    }
}

/// The operations needed for remapping a mapping.
#[derive(Debug)]
struct AllocatedRemapOp<'a> {
    lock_range: Range<Vaddr>,
    resize_op: Option<ResizeOp>,
    move_op: Option<MoveRangeOp>,
    allocator_guard: PerCpuAllocatorGuard<'a>,
}

#[derive(Debug)]
enum ResizeOp {
    /// Remove before moving the mappings.
    Remove(Range<Vaddr>),
    /// Extend after moving the mappings.
    ///
    /// The boolean specifies whether the existing mappings need to be cleared.
    Extend(Range<Vaddr>, bool),
}

#[derive(Debug, Clone)]
struct MoveRangeOp {
    from: Vaddr,
    to: Vaddr,
    size: usize,
    /// If the existing mappings in the destination range need to be cleared.
    need_to_clear_dest: bool,
}

impl<'a> AllocatedRemapOp<'a> {
    /// Calculate the required operations and allocate new regions if needed.
    ///
    /// This function does not deallocate any old regions.
    fn alloc_from(
        allocator: &'a PerCpuAllocator,
        old_addr: Vaddr,
        old_size: usize,
        new_addr: Option<Vaddr>,
        new_size: usize,
    ) -> Result<Self> {
        let mut lowest_addr = old_addr;
        let mut highest_addr = old_addr.checked_add(old_size).ok_or(Errno::EINVAL)?;

        let mut resize = if new_size < old_size {
            Some(ResizeOp::Remove(old_addr + new_size..old_addr + old_size))
        } else {
            // Calculate the `Extend` part later.
            None
        };

        let (move_op, allocator_guard) = if let Some(new_addr) = new_addr {
            let new_end = new_addr.checked_add(new_size).ok_or(Errno::EINVAL)?;
            let new_range = new_addr..new_end;

            if is_intersected(&new_range, &(old_addr..old_addr + old_size)) {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "remap: the new range overlaps with the old one"
                );
            };

            lowest_addr = lowest_addr.min(new_addr);
            highest_addr = highest_addr.max(new_end);

            let (allocator_guard, _) = allocator.alloc_specific_overwrite(new_range.clone())?;

            resize = if new_size > old_size {
                Some(ResizeOp::Extend(new_addr + old_size..new_end, true))
            } else {
                resize
            };
            (
                Some(MoveRangeOp {
                    from: old_addr,
                    to: new_addr,
                    size: old_size.min(new_size),
                    need_to_clear_dest: true,
                }),
                allocator_guard,
            )
        } else {
            assert!(
                new_size > old_size,
                "shrinking without fixed address; use `resize_mapping` instead"
            );

            let old_end = old_addr.checked_add(old_size).ok_or(Errno::EINVAL)?;
            let new_end_if_expand = old_addr.checked_add(new_size);
            if let Some(new_end) = new_end_if_expand
                && let Ok(allocator_guard) = allocator.alloc_specific(old_end..new_end)
            {
                highest_addr = highest_addr.max(new_end);

                // Fast path: expand in place.
                resize = Some(ResizeOp::Extend(old_end..new_end, false));
                (None, allocator_guard)
            } else {
                let (allocator_guard, new_addr) = allocator.alloc(new_size, PAGE_SIZE)?;

                lowest_addr = lowest_addr.min(new_addr);
                highest_addr = highest_addr.max(new_addr + new_size);

                resize = Some(ResizeOp::Extend(
                    new_addr + old_size..new_addr + new_size,
                    false,
                ));
                (
                    Some(MoveRangeOp {
                        from: old_addr,
                        to: new_addr,
                        size: old_size,
                        need_to_clear_dest: false,
                    }),
                    allocator_guard,
                )
            }
        };

        let remap_op = AllocatedRemapOp {
            lock_range: lowest_addr..highest_addr,
            resize_op: resize,
            move_op,
            allocator_guard,
        };

        Ok(remap_op)
    }

    fn new_mapping_range(&self) -> Option<Range<Vaddr>> {
        match (&self.move_op, &self.resize_op) {
            (Some(move_op), _) => Some(
                move_op.to
                    ..move_op.to
                        + move_op.size
                        + if let Some(ResizeOp::Extend(ext_range, _)) = &self.resize_op {
                            ext_range.len()
                        } else {
                            0
                        },
            ),
            (None, Some(ResizeOp::Extend(ext_range, _))) => Some(ext_range.clone()),
            _ => None,
        }
    }

    fn new_mapping_range_to_clear(&self) -> Option<Range<Vaddr>> {
        match (&self.move_op, &self.resize_op) {
            (Some(move_op), _) if move_op.need_to_clear_dest => Some(
                move_op.to
                    ..move_op.to
                        + move_op.size
                        + if let Some(ResizeOp::Extend(ext_range, true)) = &self.resize_op {
                            ext_range.len()
                        } else {
                            0
                        },
            ),
            (None, Some(ResizeOp::Extend(ext_range, true))) => Some(ext_range.clone()),
            _ => None,
        }
    }

    fn dealloc_if_fail(mut self) {
        if let Some(range) = self.new_mapping_range() {
            self.allocator_guard.dealloc(range);
        }
    }

    /// Performs:
    ///  - the clearing of fixed re-mapping destinations;
    ///  - the shrinking of mappings in the source.
    fn perform_overwrite_and_shrink(
        &mut self,
        vmar: &Vmar,
        cursor: &mut VmarCursorMut<'_>,
        rs_as_delta: &mut RsAsDelta,
    ) -> Result<()> {
        if let Some(range) = self.new_mapping_range_to_clear() {
            cursor.jump(range.start).unwrap();
            // We did `alloc_specific_overwrite` so don't do deallocation.
            vmar.remove_mappings(cursor, range.len(), rs_as_delta, None)?;
        }

        #[cfg(debug_assertions)]
        if let Some(range) = self.new_mapping_range() {
            crate::vm::vmar::cursor_util::check_range_not_mapped(cursor, range);
        }

        if let Some(ResizeOp::Remove(range)) = &self.resize_op {
            let len = range.len();
            cursor.jump(range.start).unwrap();
            vmar.remove_mappings(cursor, len, rs_as_delta, Some(&mut self.allocator_guard))?;
        }

        Ok(())
    }

    fn perform_move_and_extend(
        mut self,
        cursor: &mut VmarCursorMut<'_>,
        rs_as_delta: &mut RsAsDelta,
    ) -> Result<Vaddr> {
        if let Some(move_op) = self.move_op.clone() {
            move_op.perform(&mut self.allocator_guard, &mut *cursor)?;
        }

        if let Some(ResizeOp::Extend(range, _)) = &self.resize_op {
            cursor.jump(range.start - PAGE_SIZE).unwrap();
            while cursor.push_level_if_exists().is_some() {}
            let mapping = cursor
                .aux_meta_mut()
                .inner
                .take_one(&(range.start - PAGE_SIZE))
                .unwrap()
                .unwrap_mapping();

            let new_mapping = mapping.enlarge(range.len());

            rs_as_delta.add_as(range.len() as isize);

            super::map::map_to_page_table(cursor, new_mapping);
        }

        Ok(if let Some(MoveRangeOp { to, .. }) = &self.move_op {
            *to
        } else {
            self.lock_range.start
        })
    }
}

impl MoveRangeOp {
    fn perform<'a, 'g, 'rcu>(
        &self,
        allocator: &'a mut PerCpuAllocatorGuard<'g>,
        cursor: &'a mut VmarCursorMut<'rcu>,
    ) -> Result<()> {
        let from_range = self.from..self.from + self.size;

        cursor.jump(self.from).unwrap();

        while let Some(vm_mapping_ref) = find_next_mapped!(cursor, self.from + self.size) {
            let vm_mapping_range = vm_mapping_ref.range();

            let intersected_range = get_intersected_range(&from_range, &vm_mapping_range);

            cursor.jump(intersected_range.start).unwrap();
            propagate_if_needed(cursor, intersected_range.len());

            let Some(PteRangeMeta::VmMapping(vm_mapping)) =
                cursor.aux_meta_mut().inner.remove(&intersected_range.start)
            else {
                panic!("`find_next_mapped` does not stop at mapped `VmMapping`");
            };

            let taken = split_and_insert_rest(cursor, vm_mapping, intersected_range);

            let old_map_addr = taken.map_to_addr();
            let move_map_size = taken.range().len();

            let new_map_addr = self.to + (old_map_addr - self.from);
            let new_mapping = taken.remap_at(new_map_addr);

            map_and_populate_io_mem(cursor, new_mapping);

            allocator.dealloc(old_map_addr..old_map_addr + move_map_size);
            move_mappings(cursor, old_map_addr, new_map_addr, move_map_size);

            if cursor.jump(old_map_addr + move_map_size).is_err() {
                break;
            }
        }

        Ok(())
    }
}

fn map_and_populate_io_mem(cursor: &mut VmarCursorMut<'_>, vm_mapping: VmMapping) {
    for (mapping, level) in vm_mapping.split_for_pt(HIGHEST_PAGING_LEVEL) {
        let va = mapping.map_to_addr();
        cursor.jump(va).unwrap();

        cursor.adjust_level(level);

        let map_end = va + mapping.map_size();
        let page_range = va..map_end;

        let flags = PageFlags::from(mapping.perms()) | PageFlags::ACCESSED;
        let map_prop = PageProperty::new_user(flags, CachePolicy::Writeback);

        if let MappedMemory::Device(io_mem) = mapping.mapped_mem() {
            cursor.map_iomem(io_mem.clone(), map_prop, page_range.len(), 0);
        }

        cursor.aux_meta_mut().insert_try_merge(mapping);
    }
}

fn move_mappings(
    cursor: &mut VmarCursorMut<'_>,
    old_map_addr: Vaddr,
    new_map_addr: Vaddr,
    size: usize,
) {
    let end = old_map_addr + size;

    cursor.jump(old_map_addr).unwrap();

    while let Some(va) = cursor.find_next(end - cursor.virt_addr()) {
        let orig_level = cursor.level();
        let to_va = new_map_addr + (va - old_map_addr);

        match cursor.query() {
            VmQueriedItem::MappedRam { frame, prop } => {
                let frame = frame.clone();

                cursor.jump(to_va).unwrap();
                cursor.adjust_level(frame.map_level());

                cursor.map(frame, prop);
            }
            // `IoMem` mappings are populated during move; no need to handle here.
            VmQueriedItem::MappedIoMem { .. } => {}
            _ => unreachable!("`find_next` stopped at an intermediate or absent PTE"),
        }

        cursor.jump(va).unwrap();
        cursor.adjust_level(orig_level);

        cursor.unmap();
        if cursor.jump(va + page_size_at(orig_level)).is_err() {
            break;
        }
    }
}
