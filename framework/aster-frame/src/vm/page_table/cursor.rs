// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;
use core::{any::TypeId, mem::size_of, ops::Range};

use super::{
    Child, KernelMode, MapInfo, MapOp, MapProperty, PageTable, PageTableConstsTrait,
    PageTableEntryTrait, PageTableError, PageTableFrame, PageTableMode, PtfRef,
};
use crate::{
    sync::{ArcSpinLockGuard, SpinLock},
    vm::{paddr_to_vaddr, Paddr, Vaddr, VmFrame},
};

/// The cursor for forward traversal over the page table.
///
/// Each method may move the cursor forward, doing mapping unmaping, or
/// querying this slot.
///
/// Doing mapping is somewhat like a depth-first search on a tree, except
/// that we modify the tree while traversing it. We use a stack to simulate
/// the recursion.
///
/// Any read or write accesses to nodes require exclusive access on the
/// entire path from the root to the node. But cursor can be created without
/// holding the lock, and can release the lock after yeilding the current
/// slot while querying over the page table with a range. Simultaneous
/// reading or writing to the same range in the page table will not produce
/// consistent results, only validity is guaranteed.
pub(super) struct PageTableCursor<
    'a,
    M: PageTableMode,
    E: PageTableEntryTrait,
    C: PageTableConstsTrait,
> where
    [(); C::NR_ENTRIES_PER_FRAME]:,
    [(); C::NR_LEVELS]:,
{
    stack: [Option<PtfRef<E, C>>; C::NR_LEVELS],
    lock_guard: [Option<ArcSpinLockGuard<PageTableFrame<E, C>>>; C::NR_LEVELS],
    level: usize,
    va: Vaddr,
}

#[derive(Debug, Clone)]
pub(super) enum MapOption {
    Map {
        frame: VmFrame,
        prop: MapProperty,
    },
    MapUntyped {
        pa: Paddr,
        len: usize,
        prop: MapProperty,
    },
    Unmap {
        len: usize,
    },
}

impl MapOption {
    fn paddr(&self) -> Option<Paddr> {
        match self {
            MapOption::Map { frame, prop } => Some(frame.start_paddr()),
            MapOption::MapUntyped { pa, len, prop } => Some(*pa),
            MapOption::Unmap { len } => None,
        }
    }
    fn prop(&self) -> Option<MapProperty> {
        match self {
            MapOption::Map { frame, prop } => Some(*prop),
            MapOption::MapUntyped { pa, len, prop } => Some(*prop),
            MapOption::Unmap { len } => None,
        }
    }
    fn len(&self) -> usize {
        match self {
            // A VmFrame currently has a fixed size of 1 base page.
            MapOption::Map { frame, prop } => crate::arch::mm::PageTableConsts::BASE_PAGE_SIZE,
            MapOption::MapUntyped { pa, len, prop } => *len,
            MapOption::Unmap { len: l } => *l,
        }
    }
    fn consume(&mut self, len: usize) -> Self {
        match self {
            MapOption::Map { frame, prop } => {
                debug_assert_eq!(len, crate::arch::mm::PageTableConsts::BASE_PAGE_SIZE);
                let ret = self.clone();
                *self = MapOption::Unmap { len: 0 };
                ret
            }
            MapOption::MapUntyped { pa, len: l, prop } => {
                debug_assert!(*l >= len);
                let ret = MapOption::MapUntyped {
                    pa: *pa,
                    len,
                    prop: *prop,
                };
                *self = MapOption::MapUntyped {
                    pa: *pa + len,
                    len: *l - len,
                    prop: *prop,
                };
                ret
            }
            MapOption::Unmap { len: l } => {
                debug_assert!(*l >= len);
                let ret = MapOption::Unmap { len };
                *l -= len;
                ret
            }
        }
    }
}

impl<M: PageTableMode, E: PageTableEntryTrait, C: PageTableConstsTrait> PageTableCursor<'_, M, E, C>
where
    [(); C::NR_ENTRIES_PER_FRAME]:,
    [(); C::NR_LEVELS]:,
{
    pub(super) fn new(pt: &PageTable<M, E, C>, va: Vaddr) -> Self {
        let mut stack = core::array::from_fn(|_| None);
        stack[0] = Some(pt.root_frame.clone());
        let lock_guard = core::array::from_fn(|_| None);
        Self {
            stack,
            lock_guard,
            level: C::NR_LEVELS,
            va,
        }
    }

    /// Map or unmap the range starting from the current address.
    ///
    /// The argument `create` allows you to map the continuous range to a physical
    /// range with the given map property.
    ///
    /// The function will map as more huge pages as possible, and it will split
    /// the huge pages into smaller pages if necessary. If the input range is large,
    /// the resulting mappings may look like this (if very huge pages supported):
    ///
    /// ```text
    /// start                                                             end
    ///   |----|----------------|--------------------------------|----|----|
    ///    base      huge                     very huge           base base
    ///    4KiB      2MiB                       1GiB              4KiB  4KiB
    /// ```
    ///
    /// In practice it is suggested to use simple wrappers for this API that maps
    /// frames for safety and conciseness.
    ///
    /// # Safety
    ///
    /// This function manipulates the page table directly, and it is unsafe because
    /// it may cause undefined behavior if the caller does not ensure that the
    /// mapped address is valid and the page table is not corrupted if it is used
    /// by the kernel.
    pub(super) unsafe fn map(&mut self, option: MapOption) {
        self.acquire_locks();
        let len = option.len();
        let end = self.va + len;
        let mut option = option;
        while self.va < end {
            // Skip if we are unmapping and it is already invalid.
            let cur_pte = unsafe { self.cur_pte_ptr().read() };
            if matches!(option, MapOption::Unmap { .. }) && !cur_pte.is_valid() {
                self.next_slot();
                continue;
            }

            // We check among the conditions that may lead to a level down.
            let is_pa_not_aligned = option
                .paddr()
                .map(|pa| pa % C::page_size(self.level) != 0)
                .unwrap_or(false);
            let map_but_too_huge = self.level > C::HIGHEST_TRANSLATION_LEVEL
                && !matches!(option, MapOption::Unmap { .. });
            // We ensure not mapping in reserved kernel shared tables or releasing it.
            // Although it may be an invariant for all architectures and will be optimized
            // out by the compiler since `C::NR_LEVELS - 1 > C::HIGHEST_TRANSLATION_LEVEL`.
            let kshared_lvl_down =
                TypeId::of::<M>() == TypeId::of::<KernelMode>() && self.level >= C::NR_LEVELS - 1;
            if map_but_too_huge
                || kshared_lvl_down
                || self.va % C::page_size(self.level) != 0
                || self.va + C::page_size(self.level) > end
                || is_pa_not_aligned
            {
                let ld_prop = option.prop().unwrap_or(MapProperty::new_invalid());
                self.level_down(Some(ld_prop));
                continue;
            }
            self.map_page(option.consume(C::page_size(self.level)));
            self.next_slot();
        }
        self.release_locks();
    }

    /// Apply the given operation to all the mappings within the range.
    pub(super) unsafe fn protect(
        &mut self,
        len: usize,
        op: impl MapOp,
        allow_protect_invalid: bool,
    ) -> Result<(), PageTableError> {
        self.acquire_locks();
        let end = self.va + len;
        while self.va < end {
            let cur_pte = unsafe { self.cur_pte_ptr().read() };
            if !cur_pte.is_valid() {
                if !allow_protect_invalid {
                    return Err(PageTableError::ProtectingInvalid);
                }
                self.next_slot();
                continue;
            }
            // Go down if it's not a last node or if the page size is too big.
            if !(cur_pte.is_huge() || self.level == 1)
                || (self.va % C::page_size(self.level)) != 0
                || self.va + C::page_size(self.level) > end
            {
                self.level_down(Some(op(cur_pte.info())));
                continue;
            }
            // Apply the operation.
            unsafe {
                self.cur_pte_ptr().write(E::new(
                    cur_pte.paddr(),
                    op(cur_pte.info()),
                    cur_pte.is_huge(),
                    true,
                ))
            };
            self.next_slot();
        }
        self.release_locks();
        Ok(())
    }

    fn cur_pte_ptr(&self) -> *mut E {
        let ptf = self.lock_guard[C::NR_LEVELS - self.level].as_ref().unwrap();
        let frame_addr = paddr_to_vaddr(ptf.inner.start_paddr());
        let offset = C::in_frame_index(self.va, self.level);
        (frame_addr + offset * size_of::<E>()) as *mut E
    }

    /// Traverse forward in the current level to the next PTE.
    /// If reached the end of a page table frame, it leads itself up to the next frame of the parent frame.
    fn next_slot(&mut self) {
        let page_size = C::page_size(self.level);
        while self.level < C::NR_LEVELS && C::in_frame_index(self.va + page_size, self.level) == 0 {
            self.level_up();
        }
        self.va += page_size;
    }

    /// Go up a level. We release the current frame if it has no mappings since the cursor only moves
    /// forward. And we will do the final cleanup using `level_up` when the cursor is dropped.
    ///
    /// This method requires locks acquired before calling it. The discarded level will be unlocked.
    fn level_up(&mut self) {
        let last_map_cnt_is_zero = {
            let top_ptf = self.lock_guard[C::NR_LEVELS - self.level].as_ref().unwrap();
            top_ptf.map_count == 0
        };
        self.stack[C::NR_LEVELS - self.level] = None;
        self.lock_guard[C::NR_LEVELS - self.level] = None;
        self.level += 1;
        let can_release_child =
            TypeId::of::<M>() == TypeId::of::<KernelMode>() && self.level < C::NR_LEVELS;
        if can_release_child && last_map_cnt_is_zero {
            let top_ptf = self.lock_guard[C::NR_LEVELS - self.level]
                .as_deref_mut()
                .unwrap();
            let frame_addr = paddr_to_vaddr(top_ptf.inner.start_paddr());
            let idx = C::in_frame_index(self.va, self.level);
            unsafe { (frame_addr as *mut E).add(idx).write(E::new_invalid()) }
            top_ptf.child[idx] = None;
            top_ptf.map_count -= 1;
        }
    }

    /// A level down operation during traversal. It may split a huge page into
    /// smaller pages if we have an end address within the next mapped huge page.
    /// It may also create a new child frame if the current frame does not have one.
    /// If that may happen the map property of intermediate level `prop` should be
    /// passed in correctly. Whether the map property matters in an intermediate
    /// level is architecture-dependent.
    ///
    /// This method requires write locks acquired before calling it. The newly added
    /// level will still hold the lock.
    unsafe fn level_down(&mut self, prop: Option<MapProperty>) {
        debug_assert!(self.level > 1);
        // Check if the child frame exists.
        let nxt_lvl_frame = {
            let idx = C::in_frame_index(self.va, self.level);
            let child = {
                let top_ptf = self.lock_guard[C::NR_LEVELS - self.level].as_ref().unwrap();
                &top_ptf.child[idx]
            };
            if let Some(Child::PageTable(nxt_lvl_frame)) = child {
                Some(nxt_lvl_frame.clone())
            } else {
                None
            }
        };
        // Create a new child frame if it does not exist. Sure it could be done only if
        // it is allowed to modify the page table.
        let nxt_lvl_frame = nxt_lvl_frame.unwrap_or_else(|| {
            let mut new_frame = PageTableFrame::<E, C>::new();
            // If it already maps a huge page, we should split it.
            let pte = unsafe { self.cur_pte_ptr().read() };
            if pte.is_valid() && pte.is_huge() {
                let pa = pte.paddr();
                let prop = pte.info().prop;
                for i in 0..C::NR_ENTRIES_PER_FRAME {
                    let nxt_level = self.level - 1;
                    let nxt_pte = {
                        let frame_addr = paddr_to_vaddr(new_frame.inner.start_paddr());
                        &mut *(frame_addr as *mut E).add(i)
                    };
                    *nxt_pte = E::new(pa + i * C::page_size(nxt_level), prop, nxt_level > 1, true);
                }
                new_frame.map_count = C::NR_ENTRIES_PER_FRAME;
                unsafe {
                    self.cur_pte_ptr().write(E::new(
                        new_frame.inner.start_paddr(),
                        prop,
                        false,
                        false,
                    ))
                }
            } else {
                // The child couldn't be valid here because child is none and it's not huge.
                debug_assert!(!pte.is_valid());
                unsafe {
                    self.cur_pte_ptr().write(E::new(
                        new_frame.inner.start_paddr(),
                        prop.unwrap(),
                        false,
                        false,
                    ))
                }
            }
            let top_ptf = self.lock_guard[C::NR_LEVELS - self.level]
                .as_deref_mut()
                .unwrap();
            top_ptf.map_count += 1;
            let new_frame_ref = Arc::new(SpinLock::new(new_frame));
            top_ptf.child[C::in_frame_index(self.va, self.level)] =
                Some(Child::PageTable(new_frame_ref.clone()));
            new_frame_ref
        });
        self.lock_guard[C::NR_LEVELS - self.level + 1] = Some(nxt_lvl_frame.lock_arc());
        self.stack[C::NR_LEVELS - self.level + 1] = Some(nxt_lvl_frame);
        self.level -= 1;
    }

    /// Map or unmap the page pointed to by the cursor (which could be large).
    /// If the physical address and the map property are not provided, it unmaps
    /// the current page.
    ///
    /// This method requires write locks acquired before calling it.
    unsafe fn map_page(&mut self, option: MapOption) {
        let pte_ptr = self.cur_pte_ptr();
        let top_ptf = self.lock_guard[C::NR_LEVELS - self.level]
            .as_deref_mut()
            .unwrap();
        let child = {
            let idx = C::in_frame_index(self.va, self.level);
            if top_ptf.child[idx].is_some() {
                top_ptf.child[idx] = None;
                top_ptf.map_count -= 1;
            }
            &mut top_ptf.child[idx]
        };
        match option {
            MapOption::Map { frame, prop } => {
                let pa = frame.start_paddr();
                unsafe {
                    pte_ptr.write(E::new(pa, prop, self.level > 1, true));
                }
                *child = Some(Child::Frame(frame));
                top_ptf.map_count += 1;
            }
            MapOption::MapUntyped { pa, len, prop } => {
                debug_assert_eq!(len, C::page_size(self.level));
                unsafe {
                    pte_ptr.write(E::new(pa, prop, self.level > 1, true));
                }
                top_ptf.map_count += 1;
            }
            MapOption::Unmap { len } => {
                debug_assert_eq!(len, C::page_size(self.level));
                unsafe { pte_ptr.write(E::new_invalid()) }
            }
        }
    }

    fn acquire_locks(&mut self) {
        for i in 0..=C::NR_LEVELS - self.level {
            let Some(ref ptf) = self.stack[i] else {
                panic!("Invalid values in PT cursor stack while acuqiring locks");
            };
            debug_assert!(self.lock_guard[i].is_none());
            self.lock_guard[i] = Some(ptf.lock_arc());
        }
    }

    fn release_locks(&mut self) {
        for i in (0..=C::NR_LEVELS - self.level).rev() {
            let Some(ref ptf) = self.stack[i] else {
                panic!("Invalid values in PT cursor stack while releasing locks");
            };
            debug_assert!(self.lock_guard[i].is_some());
            self.lock_guard[i] = None;
        }
    }
}

/// The iterator for querying over the page table without modifying it.
pub struct PageTableIter<'a, M: PageTableMode, E: PageTableEntryTrait, C: PageTableConstsTrait>
where
    [(); C::NR_ENTRIES_PER_FRAME]:,
    [(); C::NR_LEVELS]:,
{
    cursor: PageTableCursor<'a, M, E, C>,
    end_va: Vaddr,
}

impl<'a, M: PageTableMode, E: PageTableEntryTrait, C: PageTableConstsTrait>
    PageTableIter<'a, M, E, C>
where
    [(); C::NR_ENTRIES_PER_FRAME]:,
    [(); C::NR_LEVELS]:,
{
    pub(super) fn new(pt: &'a PageTable<M, E, C>, va: &Range<Vaddr>) -> Self {
        Self {
            cursor: PageTableCursor::new(pt, va.start),
            end_va: va.end,
        }
    }
}

#[derive(Clone, Debug)]
pub enum PageTableQueryResult {
    NotMapped {
        va: Vaddr,
        len: usize,
    },
    Mapped {
        va: Vaddr,
        frame: VmFrame,
        info: MapInfo,
    },
    MappedUntyped {
        va: Vaddr,
        pa: Paddr,
        len: usize,
        info: MapInfo,
    },
}

impl<'a, M: PageTableMode, E: PageTableEntryTrait, C: PageTableConstsTrait> Iterator
    for PageTableIter<'a, M, E, C>
where
    [(); C::NR_ENTRIES_PER_FRAME]:,
    [(); C::NR_LEVELS]:,
{
    type Item = PageTableQueryResult;

    fn next(&mut self) -> Option<Self::Item> {
        self.cursor.acquire_locks();
        if self.cursor.va >= self.end_va {
            return None;
        }
        loop {
            let level = self.cursor.level;
            let va = self.cursor.va;
            let top_ptf = self.cursor.lock_guard[C::NR_LEVELS - level]
                .as_ref()
                .unwrap();
            let cur_pte = unsafe { self.cursor.cur_pte_ptr().read() };
            // Yeild if it's not a valid node.
            if !cur_pte.is_valid() {
                self.cursor.next_slot();
                self.cursor.release_locks();
                return Some(PageTableQueryResult::NotMapped {
                    va,
                    len: C::page_size(level),
                });
            }
            // Go down if it's not a last node.
            if !(cur_pte.is_huge() || level == 1) {
                debug_assert!(cur_pte.is_valid());
                // Safety: it's valid and there should be a child frame here.
                unsafe {
                    self.cursor.level_down(None);
                }
                continue;
            }
            // Yield the current mapping.
            let map_info = cur_pte.info();
            let idx = C::in_frame_index(self.cursor.va, self.cursor.level);
            match top_ptf.child[idx] {
                Some(Child::Frame(ref frame)) => {
                    let frame = frame.clone();
                    self.cursor.next_slot();
                    self.cursor.release_locks();
                    return Some(PageTableQueryResult::Mapped {
                        va,
                        frame,
                        info: map_info,
                    });
                }
                Some(Child::PageTable(_)) => {
                    panic!("The child couldn't be page table here because it's valid and not huge");
                }
                None => {
                    self.cursor.next_slot();
                    self.cursor.release_locks();
                    return Some(PageTableQueryResult::MappedUntyped {
                        va,
                        pa: cur_pte.paddr(),
                        len: C::page_size(level),
                        info: map_info,
                    });
                }
            }
        }
    }
}
