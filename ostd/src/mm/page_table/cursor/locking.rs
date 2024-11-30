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
            PageTableEntryTrait, PageTableGuard, PageTableMode, PageTableNodeRef,
            PagingConstsTrait, PagingLevel,
        },
        Paddr, Vaddr,
    },
    task::disable_preempt,
};

pub(super) fn lock_range<'a, M: PageTableMode, E: PageTableEntryTrait, C: PagingConstsTrait>(
    pt: &'a PageTable<M, E, C>,
    va: &Range<Vaddr>,
    new_pt_is_tracked: MapTrackingStatus,
) -> Cursor<'a, M, E, C> {
    let preempt_guard = disable_preempt();

    let mut subtree_root = traverse_and_lock_subtree_root(pt, va, new_pt_is_tracked);

    // Once we have locked the sub-tree that is not stray, we won't read any
    // stray nodes in the following traversal since we must lock before reading.
    let guard_level = subtree_root.level();
    let cur_node_va = va.start.align_down(page_size::<C>(guard_level + 1));
    dfs_acquire_lock(&mut subtree_root, cur_node_va, va.clone());

    let mut path = core::array::from_fn(|_| None);
    path[guard_level as usize - 1] = Some(subtree_root);

    Cursor::<'a, M, E, C> {
        path,
        level: guard_level,
        guard_level,
        va: va.start,
        barrier_va: va.clone(),
        preempt_guard,
        _phantom: PhantomData,
    }
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

    // SAFETY: A cursor maintains that its corresponding sub-tree is locked.
    unsafe { dfs_release_lock(guard_node, cur_node_va, cursor.barrier_va.clone()) };
}

/// Finds and locks an intermediate page table node that covers the range.
///
/// If that node (or any of its ancestors) does not exist, we need to lock
/// the parent and create it. After the creation the lock of the parent will
/// be released and the new node will be locked.
fn traverse_and_lock_subtree_root<
    'a,
    M: PageTableMode,
    E: PageTableEntryTrait,
    C: PagingConstsTrait,
>(
    pt: &'a PageTable<M, E, C>,
    va: &Range<Vaddr>,
    new_pt_is_tracked: MapTrackingStatus,
) -> PageTableGuard<'a, E, C> {
    // # Safety
    // Must be called with `cur_pt_addr` and `'a`, `E`, `E` of the residing function.
    unsafe fn lock_cur_pt<'a, E: PageTableEntryTrait, C: PagingConstsTrait>(
        cur_pt_addr: Paddr,
    ) -> PageTableGuard<'a, E, C> {
        // SAFETY: The reference is valid for `'a` because the page table
        // is alive within `'a` and `'a` is under the RCU read guard.
        let ptn_ref = unsafe { PageTableNodeRef::<'a, E, C>::borrow_paddr(cur_pt_addr) };
        // Forfeit a guard protecting a node that lives for `'a` rather
        // than the lifetime of `ptn_ref`.
        let pt_addr = ptn_ref.lock().into_raw_paddr();
        // SAFETY: The lock guard was forgotten at the above line. We manually
        // ensure that the protected node lives for `'a`.
        unsafe { PageTableGuard::<'a, E, C>::from_raw_paddr(pt_addr) }
    }

    let mut cur_node_guard: Option<PageTableGuard<E, C>> = None;
    let mut cur_pt_addr = pt.root.start_paddr();
    for cur_level in (1..=C::NR_LEVELS).rev() {
        let start_idx = pte_index::<C>(va.start, cur_level);
        let level_too_high = {
            let end_idx = pte_index::<C>(va.end - 1, cur_level);
            cur_level > 1 && start_idx == end_idx
        };
        if !level_too_high {
            break;
        }

        let cur_pt_ptr = paddr_to_vaddr(cur_pt_addr) as *mut E;
        // SAFETY:
        //  - The page table node is alive because (1) the root node is alive and
        //    (2) all child nodes cannot be recycled because we're in the RCU critical section.
        //  - The index is inside the bound, so the page table entry is valid.
        //  - All page table entries are aligned and accessed with atomic operations only.
        let cur_pte = unsafe { load_pte(cur_pt_ptr.add(start_idx), Ordering::Acquire) };

        if cur_pte.is_present() {
            if cur_pte.is_last(cur_level) {
                break;
            }
            cur_pt_addr = cur_pte.paddr();
            cur_node_guard = None;
            continue;
        }

        // In case the child is absent, we should lock and allocate a new page table node.
        // SAFETY: It is called with the required parameters.
        let mut guard = cur_node_guard
            .take()
            .unwrap_or_else(|| unsafe { lock_cur_pt::<'a, E, C>(cur_pt_addr) });
        let mut cur_entry = guard.entry(start_idx);
        if cur_entry.is_none() {
            let allocated_guard = cur_entry.alloc_if_none(new_pt_is_tracked).unwrap();

            cur_pt_addr = allocated_guard.start_paddr();
            cur_node_guard = Some(allocated_guard);
        } else if cur_entry.is_node() {
            let Child::PageTableRef(pt) = cur_entry.to_ref() else {
                unreachable!();
            };
            cur_pt_addr = pt.start_paddr();
            cur_node_guard = None;
        } else {
            break;
        }
    }

    // SAFETY: It is called with the required parameters.
    cur_node_guard.unwrap_or_else(|| unsafe { lock_cur_pt::<'a, E, C>(cur_pt_addr) })
}

/// Acquires the locks for the given range in the sub-tree rooted at the node.
///
/// `cur_node_va` must be the virtual address of the `cur_node`. The `va_range`
/// must be within the range of the `cur_node`. The range must not be empty.
///
/// The function will forget all the [`PageTableGuard`] objects in the sub-tree
/// with [`PageTableGuard::into_raw_paddr`].
fn dfs_acquire_lock<E: PageTableEntryTrait, C: PagingConstsTrait>(
    cur_node: &mut PageTableGuard<'_, E, C>,
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
                    let mut pt_guard = pt.lock();
                    let child_node_va = cur_node_va + i * page_size::<C>(cur_level);
                    let child_node_va_end = child_node_va + page_size::<C>(cur_level);
                    let va_start = va_range.start.max(child_node_va);
                    let va_end = va_range.end.min(child_node_va_end);
                    dfs_acquire_lock(&mut pt_guard, child_node_va, va_start..va_end);
                    let _ = pt_guard.into_raw_paddr();
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
///
/// # Safety
///
/// The caller must ensure that the nodes in the specified sub-tree are locked.
unsafe fn dfs_release_lock<E: PageTableEntryTrait, C: PagingConstsTrait>(
    mut cur_node: PageTableGuard<E, C>,
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
                    let child_node =
                        unsafe { PageTableGuard::<E, C>::from_raw_paddr(pt.start_paddr()) };
                    let child_node_va = cur_node_va + i * page_size::<C>(cur_level);
                    let child_node_va_end = child_node_va + page_size::<C>(cur_level);
                    let va_start = va_range.start.max(child_node_va);
                    let va_end = va_range.end.min(child_node_va_end);
                    // SAFETY: The caller ensures that this sub-tree is locked.
                    unsafe { dfs_release_lock(child_node, child_node_va, va_start..va_end) };
                }
                Child::None
                | Child::Frame(_, _)
                | Child::Untracked(_, _, _)
                | Child::PageTable(_) => {}
            }
        }
    }
}

fn dfs_get_idx_range<C: PagingConstsTrait>(
    cur_node_level: PagingLevel,
    cur_node_va: Vaddr,
    va_range: &Range<Vaddr>,
) -> Range<usize> {
    debug_assert!(va_range.start >= cur_node_va);
    debug_assert!(
        va_range.end
            <= cur_node_va
                .checked_add(page_size::<C>(cur_node_level + 1))
                .unwrap_or(Vaddr::MAX)
    );

    let start_idx = (va_range.start - cur_node_va) / page_size::<C>(cur_node_level);
    let end_idx = (va_range.end - cur_node_va).div_ceil(page_size::<C>(cur_node_level));

    debug_assert!(start_idx < end_idx);
    debug_assert!(end_idx <= nr_subpage_per_huge::<C>());

    start_idx..end_idx
}
