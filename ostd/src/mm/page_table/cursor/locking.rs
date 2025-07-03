// SPDX-License-Identifier: MPL-2.0

//! Implementation of the locking protocol.

use core::{marker::PhantomData, mem::ManuallyDrop, ops::Range, sync::atomic::Ordering};

use align_ext::AlignExt;

use super::Cursor;
use crate::{
    mm::{
        nr_subpage_per_huge, paddr_to_vaddr,
        page_table::{
            page_size, pte_index, ChildRef, PageTable, PageTableConfig, PageTableEntryTrait,
            PageTableGuard, PageTableNodeRef, PagingConstsTrait, PagingLevel,
        },
        Vaddr,
    },
    task::atomic_mode::InAtomicMode,
};

pub(super) fn lock_range<'rcu, C: PageTableConfig>(
    pt: &'rcu PageTable<C>,
    guard: &'rcu dyn InAtomicMode,
    va: &Range<Vaddr>,
) -> Cursor<'rcu, C> {
    // The re-try loop of finding the sub-tree root.
    //
    // If we locked a stray node, we need to re-try. Otherwise, although
    // there are no safety concerns, the operations of a cursor on an stray
    // sub-tree will not see the current state and will not change the current
    // state, breaking serializability.
    let mut subtree_root = loop {
        if let Some(subtree_root) = try_traverse_and_lock_subtree_root(pt, guard, va) {
            break subtree_root;
        }
    };

    // Once we have locked the sub-tree that is not stray, we won't read any
    // stray nodes in the following traversal since we must lock before reading.
    let guard_level = subtree_root.level();
    let cur_node_va = va.start.align_down(page_size::<C>(guard_level + 1));
    dfs_acquire_lock(guard, &mut subtree_root, cur_node_va, va.clone());

    let mut path = core::array::from_fn(|_| None);
    path[guard_level as usize - 1] = Some(subtree_root);

    Cursor::<'rcu, C> {
        path,
        rcu_guard: guard,
        level: guard_level,
        guard_level,
        va: va.start,
        barrier_va: va.clone(),
        _phantom: PhantomData,
    }
}

pub(super) fn unlock_range<C: PageTableConfig>(cursor: &mut Cursor<'_, C>) {
    for i in (0..cursor.guard_level as usize - 1).rev() {
        if let Some(guard) = cursor.path[i].take() {
            let _ = ManuallyDrop::new(guard);
        }
    }
    let guard_node = cursor.path[cursor.guard_level as usize - 1].take().unwrap();
    let cur_node_va = cursor
        .barrier_va
        .start
        .align_down(page_size::<C>(cursor.guard_level + 1));

    // SAFETY: A cursor maintains that its corresponding sub-tree is locked.
    unsafe {
        dfs_release_lock(
            cursor.rcu_guard,
            guard_node,
            cur_node_va,
            cursor.barrier_va.clone(),
        )
    };
}

/// Finds and locks an intermediate page table node that covers the range.
///
/// If that node (or any of its ancestors) does not exist, we need to lock
/// the parent and create it. After the creation the lock of the parent will
/// be released and the new node will be locked.
///
/// If this function founds that a locked node is stray (because of racing with
/// page table recycling), it will return `None`. The caller should retry in
/// this case to lock the proper node.
fn try_traverse_and_lock_subtree_root<'rcu, C: PageTableConfig>(
    pt: &PageTable<C>,
    guard: &'rcu dyn InAtomicMode,
    va: &Range<Vaddr>,
) -> Option<PageTableGuard<'rcu, C>> {
    let mut cur_node_guard: Option<PageTableGuard<C>> = None;
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

        let cur_pt_ptr = paddr_to_vaddr(cur_pt_addr) as *mut C::E;
        // SAFETY:
        //  - The page table node is alive because (1) the root node is alive and
        //    (2) all child nodes cannot be recycled because we're in the RCU critical section.
        //  - The index is inside the bound, so the page table entry is valid.
        //  - All page table entries are aligned and accessed with atomic operations only.
        core::sync::atomic::fence(Ordering::Acquire);
        let cur_pte = unsafe { cur_pt_ptr.add(start_idx).read_volatile() };

        if cur_pte.is_present() {
            if cur_pte.is_last(cur_level) {
                break;
            }
            cur_pt_addr = cur_pte.paddr();
            cur_node_guard = None;
            continue;
        }

        // In case the child is absent, we should lock and allocate a new page table node.
        let mut pt_guard = cur_node_guard.take().unwrap_or_else(|| {
            // SAFETY: The node must be alive for at least `'rcu` since the
            // address is read from the page table node.
            let node_ref = unsafe { PageTableNodeRef::<'rcu, C>::borrow_paddr(cur_pt_addr) };
            node_ref.lock(guard)
        });
        if *pt_guard.stray_mut() {
            return None;
        }

        let mut cur_entry = pt_guard.entry(start_idx);
        if cur_entry.is_none() {
            let allocated_guard = cur_entry.alloc_if_none(guard).unwrap();
            cur_pt_addr = allocated_guard.start_paddr();
            cur_node_guard = Some(allocated_guard);
        } else if cur_entry.is_node() {
            let ChildRef::PageTable(pt) = cur_entry.to_ref() else {
                unreachable!();
            };
            cur_pt_addr = pt.start_paddr();
            cur_node_guard = None;
        } else {
            let guard = cur_entry.split_if_mapped_huge(guard).unwrap();
            cur_pt_addr = guard.start_paddr();
            cur_node_guard = None;
        }
    }

    let mut pt_guard = cur_node_guard.unwrap_or_else(|| {
        // SAFETY: The node must be alive for at least `'rcu` since the
        // address is read from the page table node.
        let node_ref = unsafe { PageTableNodeRef::<'rcu, C>::borrow_paddr(cur_pt_addr) };
        node_ref.lock(guard)
    });
    if *pt_guard.stray_mut() {
        return None;
    }

    Some(pt_guard)
}

/// Acquires the locks for the given range in the sub-tree rooted at the node.
///
/// `cur_node_va` must be the virtual address of the `cur_node`. The `va_range`
/// must be within the range of the `cur_node`. The range must not be empty.
///
/// The function will forget all the [`PageTableGuard`] objects in the sub-tree.
fn dfs_acquire_lock<C: PageTableConfig>(
    guard: &dyn InAtomicMode,
    cur_node: &mut PageTableGuard<'_, C>,
    cur_node_va: Vaddr,
    va_range: Range<Vaddr>,
) {
    debug_assert!(!*cur_node.stray_mut());

    let cur_level = cur_node.level();
    if cur_level == 1 {
        return;
    }

    let idx_range = dfs_get_idx_range::<C>(cur_level, cur_node_va, &va_range);
    for i in idx_range {
        let child = cur_node.entry(i);
        match child.to_ref() {
            ChildRef::PageTable(pt) => {
                let mut pt_guard = pt.lock(guard);
                let child_node_va = cur_node_va + i * page_size::<C>(cur_level);
                let child_node_va_end = child_node_va + page_size::<C>(cur_level);
                let va_start = va_range.start.max(child_node_va);
                let va_end = va_range.end.min(child_node_va_end);
                dfs_acquire_lock(guard, &mut pt_guard, child_node_va, va_start..va_end);
                let _ = ManuallyDrop::new(pt_guard);
            }
            ChildRef::None | ChildRef::Frame(_, _, _) => {}
        }
    }
}

/// Releases the locks for the given range in the sub-tree rooted at the node.
///
/// # Safety
///
/// The caller must ensure that the nodes in the specified sub-tree are locked
/// and all guards are forgotten.
unsafe fn dfs_release_lock<'rcu, C: PageTableConfig>(
    guard: &'rcu dyn InAtomicMode,
    mut cur_node: PageTableGuard<'rcu, C>,
    cur_node_va: Vaddr,
    va_range: Range<Vaddr>,
) {
    let cur_level = cur_node.level();
    if cur_level == 1 {
        return;
    }

    let idx_range = dfs_get_idx_range::<C>(cur_level, cur_node_va, &va_range);
    for i in idx_range.rev() {
        let child = cur_node.entry(i);
        match child.to_ref() {
            ChildRef::PageTable(pt) => {
                // SAFETY: The caller ensures that the node is locked and the new guard is unique.
                let child_node = unsafe { pt.make_guard_unchecked(guard) };
                let child_node_va = cur_node_va + i * page_size::<C>(cur_level);
                let child_node_va_end = child_node_va + page_size::<C>(cur_level);
                let va_start = va_range.start.max(child_node_va);
                let va_end = va_range.end.min(child_node_va_end);
                // SAFETY: The caller ensures that all the nodes in the sub-tree are locked and all
                // guards are forgotten.
                unsafe { dfs_release_lock(guard, child_node, child_node_va, va_start..va_end) };
            }
            ChildRef::None | ChildRef::Frame(_, _, _) => {}
        }
    }
}

/// Marks all the nodes in the sub-tree rooted at the node as stray, and
/// returns the num of pages mapped within the sub-tree.
///
/// It must be called upon the node after the node is removed from the parent
/// page table. It also unlocks the nodes in the sub-tree.
///
/// This function returns the number of physical frames mapped in the sub-tree.
///
/// # Safety
///
/// The caller must ensure that all the nodes in the sub-tree are locked
/// and all guards are forgotten.
///
/// This function must not be called upon a shared node, e.g., the second-
/// top level nodes that the kernel space and user space share.
pub(super) unsafe fn dfs_mark_stray_and_unlock<C: PageTableConfig>(
    rcu_guard: &dyn InAtomicMode,
    mut sub_tree: PageTableGuard<C>,
) -> usize {
    *sub_tree.stray_mut() = true;

    if sub_tree.level() == 1 {
        return sub_tree.nr_children() as usize;
    }

    let mut num_frames = 0;

    for i in (0..nr_subpage_per_huge::<C>()).rev() {
        let child = sub_tree.entry(i);
        match child.to_ref() {
            ChildRef::PageTable(pt) => {
                // SAFETY: The caller ensures that the node is locked and the new guard is unique.
                let locked_pt = unsafe { pt.make_guard_unchecked(rcu_guard) };
                // SAFETY: The caller ensures that all the nodes in the sub-tree are locked and all
                // guards are forgotten.
                num_frames += unsafe { dfs_mark_stray_and_unlock(rcu_guard, locked_pt) };
            }
            ChildRef::None | ChildRef::Frame(_, _, _) => {}
        }
    }

    num_frames
}

fn dfs_get_idx_range<C: PagingConstsTrait>(
    cur_node_level: PagingLevel,
    cur_node_va: Vaddr,
    va_range: &Range<Vaddr>,
) -> Range<usize> {
    debug_assert!(va_range.start >= cur_node_va);
    debug_assert!(va_range.end <= cur_node_va.saturating_add(page_size::<C>(cur_node_level + 1)));

    let start_idx = (va_range.start - cur_node_va) / page_size::<C>(cur_node_level);
    let end_idx = (va_range.end - cur_node_va).div_ceil(page_size::<C>(cur_node_level));

    debug_assert!(start_idx < end_idx);
    debug_assert!(end_idx <= nr_subpage_per_huge::<C>());

    start_idx..end_idx
}
