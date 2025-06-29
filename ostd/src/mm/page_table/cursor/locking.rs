// SPDX-License-Identifier: MPL-2.0

//! Implementation of the locking protocol.

use core::{marker::PhantomData, mem::ManuallyDrop, ops::Range};

use super::Cursor;
use crate::{
    mm::{
        page_table::{
            cursor::{GuardInPath, MAX_NR_LEVELS},
            pte_index, ChildRef, PageTable, PageTableConfig, PagingConstsTrait,
        },
        Vaddr,
    },
    task::atomic_mode::InAtomicMode,
};

pub(super) fn lock_range<'a, C: PageTableConfig>(
    pt: &'a PageTable<C>,
    guard: &'a dyn InAtomicMode,
    va: &Range<Vaddr>,
) -> Cursor<'a, C> {
    let mut path: [GuardInPath<'a, C>; MAX_NR_LEVELS] =
        core::array::from_fn(|_| GuardInPath::Unlocked);

    let mut cur_pt = pt.root.borrow();

    // Go down and get proper locks. The cursor should hold a write lock of a
    // page table node containing the virtual address range.
    //
    // While going down, we will hold read locks of previous path of too-high levels.
    let cur_wlock = loop {
        let cur_level = cur_pt.level();

        let start_idx = pte_index::<C>(va.start, cur_level);
        let level_too_high = {
            let end_idx = pte_index::<C>(va.end - 1, cur_level);
            cur_level > 1 && start_idx == end_idx
        };
        if !level_too_high {
            break None;
        }

        let mut cur_pt_rlockguard = cur_pt.clone_ref().lock_read(guard);

        let entry = cur_pt_rlockguard.entry(start_idx);
        let child_ref = entry.to_ref();
        match child_ref {
            ChildRef::PageTable(pt) => {
                path[cur_level as usize - 1] = GuardInPath::Read(cur_pt_rlockguard);
                cur_pt = pt;
                continue;
            }
            ChildRef::None | ChildRef::Frame(_, _, _) => {
                // Upgrade to write lock.
                drop(cur_pt_rlockguard);
                let mut cur_pt_wlockguard = cur_pt.clone_ref().lock_write(guard);

                let mut entry = cur_pt_wlockguard.entry(start_idx);
                match entry.to_ref() {
                    ChildRef::PageTable(pt) => {
                        // We are here because of the non-atomic upgrade. We need
                        // to downgrade back to read lock. Other threads can't
                        // remove the child since we've already hold the parent
                        // read lock. Therefore a non-atomic downgrade is fine.
                        drop(cur_pt_wlockguard);
                        let cur_pt_rlockguard = cur_pt.clone_ref().lock_read(guard);
                        path[cur_level as usize - 1] = GuardInPath::Read(cur_pt_rlockguard);
                        cur_pt = pt;
                        continue;
                    }
                    ChildRef::None | ChildRef::Frame(_, _, _) => {
                        // We need to allocate a new page table node.
                        let wguard = if let Some(allocated) = entry.alloc_if_none(guard) {
                            allocated
                        } else {
                            entry.split_if_mapped_huge(guard).unwrap()
                        };
                        let cur_pt = wguard.as_ref();
                        // This is implicitly write locked. Don't drop (unlock) it.
                        let _ = ManuallyDrop::new(wguard);
                        // Downgrade to read lock.
                        drop(cur_pt_wlockguard);
                        let cur_pt_rlockguard = cur_pt.clone_ref().lock_read(guard);
                        path[cur_level as usize - 1] = GuardInPath::Read(cur_pt_rlockguard);
                        continue;
                    }
                }
            }
        }
    };

    // Get write lock of the current page table node.
    let cur_level = cur_pt.level();
    let cur_pt_wlockguard = cur_wlock.unwrap_or_else(|| cur_pt.lock_write(guard));
    path[cur_level as usize - 1] = GuardInPath::Write(cur_pt_wlockguard);

    #[cfg(debug_assertions)]
    {
        for i in (C::NR_LEVELS..cur_level).rev() {
            assert!(matches!(&path[i as usize - 1], GuardInPath::Read(_)));
        }
    }

    Cursor::<'a, C> {
        path,
        rcu_guard: guard,
        level: cur_level,
        guard_level: cur_level,
        va: va.start,
        barrier_va: va.clone(),
        _phantom: PhantomData,
    }
}

pub(super) fn unlock_range<C: PageTableConfig>(cursor: &mut Cursor<'_, C>) {
    #[cfg(debug_assertions)]
    {
        for i in 1..cursor.level {
            debug_assert!(matches!(
                cursor.path[i as usize - 1].take(),
                GuardInPath::Unlocked
            ))
        }
    }

    for i in cursor.level..cursor.guard_level {
        let GuardInPath::ImplicitWrite(guard) = cursor.path[i as usize - 1].take() else {
            panic!(
                "Expected implicitly locked guard at level {}, found {:?}",
                i,
                cursor.path[i as usize - 1]
            );
        };
        // This is implicitly write locked. Don't drop (unlock) it.
        let _ = ManuallyDrop::new(guard);
    }

    let GuardInPath::Write(guard_node) = cursor.path[cursor.guard_level as usize - 1].take() else {
        panic!("Expected write lock");
    };

    drop(guard_node);

    for i in (cursor.guard_level + 1)..=C::NR_LEVELS {
        let GuardInPath::Read(rguard) = cursor.path[i as usize - 1].take() else {
            panic!(
                "Expected read lock at level {}, found {:?}",
                i,
                cursor.path[i as usize - 1]
            );
        };
        drop(rguard);
    }
}
