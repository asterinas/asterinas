// SPDX-License-Identifier: MPL-2.0

//! Implementation of the locking protocol.

use core::{marker::PhantomData, ops::Range, sync::atomic::Ordering};

use align_ext::AlignExt;

use super::Cursor;
use crate::{
    mm::{
        nr_subpage_per_huge, paddr_to_vaddr,
        page_table::{
            load_pte, page_size, pte_index, Child, MapTrackingStatus, PageTable,
            PageTableEntryTrait, PageTableLock, PageTableMode, PageTableNode, PagingConstsTrait,
            PagingLevel,
        },
        Vaddr,
    },
    task::disable_preempt,
};

pub(super) fn lock_range<'a, M: PageTableMode, E: PageTableEntryTrait, C: PagingConstsTrait>(
    pt: &'a PageTable<M, E, C>,
    va: &Range<Vaddr>,
    new_pt_is_tracked: MapTrackingStatus,
) -> Cursor<'a, M, E, C> {
    let mut cursor = Cursor::<'a, M, E, C> {
        path: core::array::from_fn(|_| None),
        level: C::NR_LEVELS,
        guard_level: C::NR_LEVELS,
        va: va.start,
        barrier_va: va.clone(),
        preempt_guard: disable_preempt(),
        _phantom: PhantomData,
    };

    let mut cur_pt_addr = pt.root.start_paddr();
    // Must be called with `cur_pt_addr`.
    let cur_pt_from_addr = |cur_pt_addr: &usize| -> PageTableNode<E, C> {
        // SAFETY: The address corresponds to a child converted into a PTE.
        // It must be alive since the root node is alive and contains a
        // reference to this child.
        unsafe { PageTableNode::from_raw(*cur_pt_addr) }
    };

    // Go down and get proper locks. The cursor should hold a lock of a
    // page table node containing the virtual address range.
    //
    // While going down, previous path of too-high levels will be released.
    loop {
        // Clear the guard of the previous level.
        if cursor.level < C::NR_LEVELS {
            if let Some(upper_pt) = cursor.path[(cursor.level + 1) as usize - 1].take() {
                let _ = upper_pt.unlock().into_raw();
            };
        }

        let start_idx = pte_index::<C>(va.start, cursor.level);
        let level_too_high = {
            let end_idx = pte_index::<C>(va.end - 1, cursor.level);
            cursor.level > 1 && start_idx == end_idx
        };
        if !level_too_high {
            break;
        }

        let cur_pt_ptr = paddr_to_vaddr(cur_pt_addr) as *mut E;
        // SAFETY:
        // - The page table node is alive because (1) the root node is alive
        //   and (2) all child nodes cannot be recycled if there are cursors.
        // - The index is inside the bound, so the page table entry is valid.
        // - All page table entries are aligned and accessed with atomic
        //   operations only.
        let cur_pte = unsafe { load_pte(cur_pt_ptr.add(start_idx), Ordering::Acquire) };
        if cur_pte.is_present() {
            if cur_pte.is_last(cursor.level) {
                break;
            } else {
                cur_pt_addr = cur_pte.paddr();
                cursor.level -= 1;
            }
        } else {
            // In either marked case or not mapped case, we should lock
            // and allocate a new page table node.
            let guard = cursor.path[cursor.level as usize - 1].get_or_insert_with(|| {
                let ptn = cur_pt_from_addr(&cur_pt_addr);
                debug_assert_eq!(ptn.level(), cursor.level);
                ptn.lock()
            });
            let cur_entry = guard.entry(start_idx);
            if cur_entry.is_none() {
                let pt = PageTableLock::<E, C>::alloc(cursor.level - 1, new_pt_is_tracked);
                cur_pt_addr = pt.paddr();
                let _ = cur_entry.replace(Child::PageTable(cur_pt_from_addr(&cur_pt_addr)));
                cursor.level -= 1;
                let old = cursor.path[cursor.level as usize - 1].replace(pt);
                debug_assert!(old.is_none());
            } else if cur_entry.is_node() {
                let Child::PageTableRef(pt) = cur_entry.to_ref() else {
                    unreachable!();
                };
                cur_pt_addr = pt;
                cursor.level -= 1;
            } else {
                break;
            }
        }
    }

    let sub_tree = cursor.path[cursor.level as usize - 1].get_or_insert_with(|| {
        let ptn = cur_pt_from_addr(&cur_pt_addr);
        debug_assert_eq!(ptn.level(), cursor.level);
        ptn.lock()
    });
    debug_assert_eq!(sub_tree.level(), cursor.level);

    dfs_acquire_lock(
        sub_tree,
        va.start.align_down(page_size::<C>(cursor.level + 1)),
        va.clone(),
    );

    cursor.guard_level = cursor.level;

    cursor
}

pub(super) fn unlock_range<M: PageTableMode, E: PageTableEntryTrait, C: PagingConstsTrait>(
    cursor: &mut Cursor<'_, M, E, C>,
) {
    for i in (0..cursor.guard_level as usize - 1).rev() {
        if let Some(guard) = cursor.path[i].take() {
            let _ = guard.into_raw_paddr();
        }
    }
    let guard_node = cursor.path[cursor.guard_level as usize - 1].take().unwrap();
    let cur_node_va = cursor.barrier_va.start / page_size::<C>(cursor.guard_level + 1)
        * page_size::<C>(cursor.guard_level + 1);
    dfs_release_lock(guard_node, cur_node_va, cursor.barrier_va.clone());
}

/// Acquires the locks for the given range in the sub-tree rooted at the node.
///
/// `cur_node_va` must be the virtual address of the `cur_node`. The `va_range`
/// must be within the range of the `cur_node`. The range must not be empty.
///
/// The function will forget all the [`PageTableLock`] objects in the sub-tree
/// with [`PageTableLock::into_raw_paddr`].
fn dfs_acquire_lock<E: PageTableEntryTrait, C: PagingConstsTrait>(
    cur_node: &mut PageTableLock<E, C>,
    cur_node_va: Vaddr,
    va_range: Range<Vaddr>,
) {
    let cur_level = cur_node.level();

    if cur_level > 1 {
        let idx_range = dfs_get_idx_range::<C>(cur_level, cur_node_va, &va_range);
        for i in idx_range {
            let child = cur_node.entry(i);
            match child.to_ref() {
                Child::PageTableRef(pt) => {
                    // SAFETY: This must be alive since we have a reference
                    // to the parent node that is still alive.
                    let pt = unsafe { PageTableNode::<E, C>::from_raw(pt) };
                    let mut pt = pt.lock();
                    let child_node_va = cur_node_va + i * page_size::<C>(cur_level);
                    let child_node_va_end = child_node_va + page_size::<C>(cur_level);
                    let va_start = va_range.start.max(child_node_va);
                    let va_end = va_range.end.min(child_node_va_end);
                    dfs_acquire_lock(&mut pt, child_node_va, va_start..va_end);
                    let _ = pt.into_raw_paddr();
                }
                Child::None
                | Child::Frame(_, _)
                | Child::Untracked(_, _, _)
                | Child::PageTable(_) => {}
            }
        }
    }
}

/// Releases the locks for the given range in the sub-tree rooted at the node.
fn dfs_release_lock<E: PageTableEntryTrait, C: PagingConstsTrait>(
    mut cur_node: PageTableLock<E, C>,
    cur_node_va: Vaddr,
    va_range: Range<Vaddr>,
) {
    let cur_level = cur_node.level();

    if cur_level > 1 {
        let idx_range = dfs_get_idx_range::<C>(cur_level, cur_node_va, &va_range);
        for i in idx_range.rev() {
            let child = cur_node.entry(i);
            match child.to_ref() {
                Child::PageTableRef(pt) => {
                    // SAFETY: The node was locked before and we have a
                    // reference to the parent node that is still alive.
                    let child_node = unsafe { PageTableLock::<E, C>::from_raw_paddr(pt) };
                    let child_node_va = cur_node_va + i * page_size::<C>(cur_level);
                    let child_node_va_end = child_node_va + page_size::<C>(cur_level);
                    let va_start = va_range.start.max(child_node_va);
                    let va_end = va_range.end.min(child_node_va_end);
                    dfs_release_lock(child_node, child_node_va, va_start..va_end);
                }
                Child::None
                | Child::Frame(_, _)
                | Child::Untracked(_, _, _)
                | Child::PageTable(_) => {}
            }
        }
    }

    let _ = cur_node.unlock().into_raw();
}

fn dfs_get_idx_range<C: PagingConstsTrait>(
    cur_node_level: PagingLevel,
    cur_node_va: Vaddr,
    va_range: &Range<Vaddr>,
) -> Range<usize> {
    debug_assert!(va_range.start >= cur_node_va);
    debug_assert!(va_range.end <= cur_node_va + page_size::<C>(cur_node_level + 1));

    let start_idx = (va_range.start - cur_node_va) / page_size::<C>(cur_node_level);
    let end_idx = (va_range.end - cur_node_va).div_ceil(page_size::<C>(cur_node_level));

    debug_assert!(start_idx < end_idx);
    debug_assert!(end_idx <= nr_subpage_per_huge::<C>());

    start_idx..end_idx
}
