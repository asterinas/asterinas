// SPDX-License-Identifier: MPL-2.0

//! Implementation of the locking protocol.

use core::{marker::PhantomData, ops::Range, sync::atomic::Ordering};

use align_ext::AlignExt;

use super::{Cursor, MAX_NR_LEVELS};
use crate::{
    mm::{
        nr_subpage_per_huge, paddr_to_vaddr,
        page_table::{
            load_pte, page_size, pte_index, zeroed_pt_pool, Child, MapTrackingStatus, PageTable,
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
    // Start RCU read-side critical section.
    let preempt_guard = disable_preempt();

    zeroed_pt_pool::prefill(&preempt_guard);

    let mut path: [Option<PageTableLock<E, C>>; MAX_NR_LEVELS] = core::array::from_fn(|_| None);
    let mut level = C::NR_LEVELS;

    // The re-try loop of finding the sub-tree root.
    //
    // If we locked an astray node, we need to re-try. Otherwise, although
    // there are no safety concerns, the operations of a cursor on an astray
    // sub-tree will not see the current state and will not change the current
    // state, breaking serializability.
    let sub_tree = 'retry: loop {
        // Retry clean up.
        for lock in path.iter_mut() {
            if let Some(guard) = lock.take() {
                let _ = guard.unlock().into_raw();
            }
        }

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
            if level < C::NR_LEVELS {
                if let Some(upper_pt) = path[(level + 1) as usize - 1].take() {
                    let _ = upper_pt.unlock().into_raw();
                };
            }

            let start_idx = pte_index::<C>(va.start, level);
            let level_too_high = {
                let end_idx = pte_index::<C>(va.end - 1, level);
                level > 1 && start_idx == end_idx
            };
            if !level_too_high {
                break;
            }

            let cur_pt_ptr = paddr_to_vaddr(cur_pt_addr) as *mut E;
            // SAFETY:
            // - The page table node is alive because (1) the root node is alive and (2) all child nodes cannot
            //   be recycled if there are cursors.
            // - The index is inside the bound, so the page table entry is valid.
            // - All page table entries are aligned and accessed with atomic operations only.
            let cur_pte = unsafe { load_pte(cur_pt_ptr.add(start_idx), Ordering::Acquire) };
            if cur_pte.is_present() {
                if cur_pte.is_last(level) {
                    break;
                } else {
                    cur_pt_addr = cur_pte.paddr();
                    level -= 1;
                }
            } else {
                // In either marked case or not mapped case, we should lock
                // and allocate a new page table node.
                let guard = path[level as usize - 1].get_or_insert_with(|| {
                    let ptn = cur_pt_from_addr(&cur_pt_addr);
                    debug_assert_eq!(ptn.level(), level);
                    ptn.lock()
                });
                if *guard.astray_mut() {
                    continue 'retry;
                }
                let cur_entry = guard.entry(start_idx);
                if cur_entry.is_none() {
                    let pt =
                        zeroed_pt_pool::alloc::<E, C>(&preempt_guard, level - 1, new_pt_is_tracked);
                    cur_pt_addr = pt.paddr();
                    let _ = cur_entry.replace(Child::PageTable(cur_pt_from_addr(&cur_pt_addr)));
                    level -= 1;
                    let old = path[level as usize - 1].replace(pt);
                    debug_assert!(old.is_none());
                } else if cur_entry.is_node() {
                    let Child::PageTableRef(pt) = cur_entry.to_ref() else {
                        unreachable!();
                    };
                    cur_pt_addr = pt;
                    level -= 1;
                } else if cur_entry.is_token() {
                    let pt = cur_entry.split_if_huge_token().unwrap();
                    cur_pt_addr = pt.paddr();
                    level -= 1;
                    let old = path[level as usize - 1].replace(pt);
                    debug_assert!(old.is_none());
                }
            }
        }

        let sub_tree = path[level as usize - 1].get_or_insert_with(|| {
            let ptn = cur_pt_from_addr(&cur_pt_addr);
            debug_assert_eq!(ptn.level(), level);
            ptn.lock()
        });
        if *sub_tree.astray_mut() {
            continue 'retry;
        }
        debug_assert_eq!(sub_tree.level(), level);

        break sub_tree; // Break the re-try loop.
    };

    // Once we have locked the sub-tree that is not astray, we won't read any
    // astray nodes in the following traversal since we must lock before reading.
    dfs_acquire_lock(
        sub_tree,
        va.start.align_down(page_size::<C>(level + 1)),
        va.clone(),
    );

    Cursor::<'a, M, E, C> {
        path,
        level,
        guard_level: level,
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
    debug_assert!(!*cur_node.astray_mut());
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
                | Child::PageTable(_)
                | Child::Token(_) => {}
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
                | Child::PageTable(_)
                | Child::Token(_) => {}
            }
        }
    }

    let _ = cur_node.unlock().into_raw();
}

/// Marks all the nodes in the sub-tree rooted at the node as astray.
///
/// This function must be called upon the node after the node is removed
/// from the parent page table.
///
/// This function also unlocks the nodes in the sub-tree. It returns the
/// sub-tree in an unlocked form.
///
/// # Safety
///
/// The caller must ensure that all the nodes in the sub-tree are locked.
///
/// This function must not be called upon a shared node. E.g., the second-
/// top level nodes that the kernel space and user space share.
pub(super) unsafe fn dfs_mark_astray<E: PageTableEntryTrait, C: PagingConstsTrait>(
    mut sub_tree: PageTableLock<E, C>,
) -> PageTableNode<E, C> {
    *sub_tree.astray_mut() = true;

    if sub_tree.level() > 1 {
        for i in (0..nr_subpage_per_huge::<C>()).rev() {
            let child = sub_tree.entry(i);
            match child.to_ref() {
                Child::PageTableRef(pt) => {
                    // SAFETY: The caller ensures that the node is locked.
                    let locked_pt = unsafe { PageTableLock::<E, C>::from_raw_paddr(pt) };
                    let unlocked_pt = dfs_mark_astray(locked_pt);
                    let _ = unlocked_pt.into_raw();
                }
                Child::None
                | Child::Frame(_, _)
                | Child::Untracked(_, _, _)
                | Child::PageTable(_)
                | Child::Token(_) => {}
            }
        }
    }

    sub_tree.unlock()
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
