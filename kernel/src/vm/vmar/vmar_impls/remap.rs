// SPDX-License-Identifier: MPL-2.0

use core::ops::Range;

use ostd::{
    mm::{page_size_at, vm_space::VmQueriedItem},
    task::{DisabledPreemptGuard, disable_preempt},
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
        is_userspace_vaddr_range,
        util::{get_intersected_range, is_intersected},
        vm_allocator::AllocatorGuard,
        vm_mapping::MappedMemory,
        vmar_impls::map::{map_populate, map_to_page_table},
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
    ///
    /// [`VmMapping`]: crate::vm::vmar::vm_mapping::VmMapping
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

        let (_alloc_guard, mut cursor) =
            self.allocator
                .alloc_specific_and_lock(&preempt_guard, self.vm_space(), &range_to_lock);

        if new_size > old_size {
            cursor.jump(map_addr + old_size).unwrap();
            if find_next_mapped!(cursor, map_addr + new_size).is_some() {
                return_errno_with_message!(
                    Errno::ENOMEM,
                    "resize_mapping: the expanded range overlaps with existing mappings"
                );
            }
            cursor.jump(map_addr).unwrap();
        }

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
            self.remove_mappings(&mut cursor, old_size - new_size, &mut rs_as_delta)?;
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

        self.add_mapping_size(&preempt_guard, new_size - old_size)?;

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

        let (mut remap_op, mut cursor) = AllocatedRemapOp::alloc_from(
            &preempt_guard,
            self,
            old_addr,
            old_size,
            new_addr,
            new_size,
        )?;

        let mut rs_as_delta = RsAsDelta::new(self);

        // Linux Compatibility: Linux unmaps the destination range/shrinks the
        // old range before checking if the source range is covered by a single
        // mapping.
        remap_op.perform_overwrite_and_shrink(self, &mut cursor, &mut rs_as_delta)?;

        cursor.jump(old_addr).unwrap();
        let last_mapping =
            check_range_within_one_mapping!(cursor, old_addr + old_size.min(new_size))?;

        if !last_mapping.can_expand() {
            return_errno_with_message!(Errno::EINVAL, "remap: the mapping cannot be expanded");
        }

        drop(rs_as_delta);
        if let Some(range) = remap_op.new_mapping_range() {
            self.add_mapping_size(&preempt_guard, range.len())?;
        }

        Ok(remap_op.perform_move_and_extend(&mut cursor))
    }
}

/// The operations needed for remapping a mapping.
#[derive(Debug)]
struct AllocatedRemapOp<'a> {
    resize_op: Option<ResizeOp>,
    move_op: Option<MoveRangeOp>,
    #[expect(dead_code)]
    alloc_guard: AllocatorGuard<'a>,
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
    ///
    /// Returns [`Errno::ENOMEM`] if the new range overlaps with existing
    /// mappings when `new_addr` is `Some`.
    fn alloc_from(
        guard: &'a DisabledPreemptGuard,
        parent: &'a Vmar,
        old_addr: Vaddr,
        old_size: usize,
        new_addr: Option<Vaddr>,
        new_size: usize,
    ) -> Result<(Self, VmarCursorMut<'a>)> {
        let mut resize = if new_size < old_size {
            Some(ResizeOp::Remove(old_addr + new_size..old_addr + old_size))
        } else {
            // Calculate the `Extend` later.
            None
        };

        let (move_op, alloc_guard, cursor) = if let Some(new_addr) = new_addr {
            if !new_addr.is_multiple_of(PAGE_SIZE) || !is_userspace_vaddr_range(new_addr, new_size)
            {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "remap: the new range is not aligned or not in userspace"
                );
            }
            let new_end = new_addr
                .checked_add(new_size)
                .ok_or(Error::with_message(Errno::EINVAL, "mremap: end overflows"))?;
            let new_range = new_addr..new_end;

            if is_intersected(&new_range, &(old_addr..old_addr + old_size)) {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "remap: the new range overlaps with the old one"
                );
            };

            let lowest_addr = old_addr.min(new_addr);
            let highest_addr = old_addr
                .checked_add(old_size)
                .ok_or(Errno::EINVAL)?
                .max(new_end);

            let (alloc_guard, cursor) = parent.allocator.alloc_specific_and_lock_larger(
                guard,
                parent.vm_space(),
                &new_range,
                &(lowest_addr..highest_addr),
            );

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
                alloc_guard,
                cursor,
            )
        } else {
            assert!(
                new_size > old_size,
                "shrinking without fixed address; use `resize_mapping` instead"
            );

            let old_end = old_addr.checked_add(old_size).ok_or(Errno::EINVAL)?;
            let new_end_if_expand = old_addr.checked_add(new_size);
            if let Some(new_end) = new_end_if_expand
                && let Ok((alloc_guard, cursor)) = lock_range_and_check_empty(
                    guard,
                    parent,
                    &(old_addr..new_end),
                    &(old_end..new_end),
                )
            {
                // Fast path: expand in place.
                resize = Some(ResizeOp::Extend(old_end..new_end, false));
                (None, alloc_guard, cursor)
            } else {
                let (new_addr, alloc_guard, cursor) =
                    parent.allocator.alloc_and_lock_covering_another(
                        guard,
                        parent.vm_space(),
                        old_addr..old_end,
                        new_size,
                        PAGE_SIZE,
                    )?;

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
                    alloc_guard,
                    cursor,
                )
            }
        };

        let remap_op = AllocatedRemapOp {
            resize_op: resize,
            move_op,
            alloc_guard,
        };

        Ok((remap_op, cursor))
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
            vmar.remove_mappings(cursor, range.len(), rs_as_delta)?;
        }

        #[cfg(debug_assertions)]
        if let Some(range) = self.new_mapping_range() {
            crate::vm::vmar::cursor_util::check_range_not_mapped(cursor, range);
        }

        if let Some(ResizeOp::Remove(range)) = &self.resize_op {
            let len = range.len();
            cursor.jump(range.start).unwrap();
            vmar.remove_mappings(cursor, len, rs_as_delta)?;
        }

        Ok(())
    }

    fn perform_move_and_extend(self, cursor: &mut VmarCursorMut<'_>) -> Vaddr {
        if let Some(move_op) = self.move_op.clone() {
            move_op.perform(&mut *cursor);
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

            super::map::map_to_page_table(cursor, new_mapping);
        }

        if let Some(MoveRangeOp { to, .. }) = &self.move_op {
            *to
        } else {
            cursor.guard_va_range().start
        }
    }
}

impl MoveRangeOp {
    fn perform<'a, 'rcu>(&self, cursor: &'a mut VmarCursorMut<'rcu>) {
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

            if matches!(new_mapping.mapped_mem(), MappedMemory::Device(_)) {
                map_populate(cursor, new_mapping);
            } else {
                map_to_page_table(cursor, new_mapping);
            }

            move_mappings(cursor, old_map_addr, new_map_addr, move_map_size);

            if cursor.jump(old_map_addr + move_map_size).is_err() {
                break;
            }
        }
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

fn lock_range_and_check_empty<'a>(
    guard: &'a DisabledPreemptGuard,
    parent: &'a Vmar,
    lock_range: &Range<Vaddr>,
    check_range: &Range<Vaddr>,
) -> Result<(AllocatorGuard<'a>, VmarCursorMut<'a>)> {
    debug_assert!(check_range.start < check_range.end);
    debug_assert!(lock_range.contains(&check_range.start));
    debug_assert!(lock_range.contains(&(check_range.end - 1)));

    let (alloc_guard, mut cursor) = parent.allocator.alloc_specific_and_lock_larger(
        guard,
        parent.vm_space(),
        check_range,
        lock_range,
    );

    cursor.jump(check_range.start).unwrap();
    if find_next_mapped!(cursor, check_range.end).is_some() {
        return_errno_with_message!(
            Errno::ENOMEM,
            "remap: the new range overlaps with existing mappings"
        );
    }

    Ok((alloc_guard, cursor))
}
