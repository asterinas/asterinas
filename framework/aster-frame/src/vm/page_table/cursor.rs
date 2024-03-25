// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, sync::Arc};
use core::marker::PhantomData;

use crate::vm::{VmAllocOptions, Paddr, Vaddr, paddr_to_vaddr};

use super::{properties::{in_frame_index, nr_entries_per_frame, page_size}, MapProperty, PageTable, PageTableConstsTrait, PageTableEntryTrait, PageTableFrame, PageTableMode};

/// The cursor for traversal over the page table.
///
/// Doing mapping is somewhat like a depth-first search on a tree, except
/// that we modify the tree while traversing it. We use a stack to simulate
/// the recursion.
pub(super) struct PageTableCursor<'a, T: PageTableEntryTrait, P: PageTableConstsTrait, M: PageTableMode> {
    page_table: &'a PageTable<T, P, M>,
    stack: [Option<Arc<PageTableFrame<T, P>>>; P::NR_LEVELS],
    level: usize,
    va: Vaddr,
}

impl<T: PageTableEntryTrait, P: PageTableConstsTrait, M: PageTableMode> PageTableCursor<'_, P, M> {
    pub(super) fn new(page_table: &PageTable<T, P, M>, va: usize) -> Self {
        let mut stack = [None; P::NR_LEVELS];
        stack[0] = Some(page_table.root_frame.clone());
        Self {
            page_table,
            stack,
            level: P::NR_LEVELS,
            va,
        }
    }

    pub(super) fn level(&self) -> usize {
        self.level
    }

    pub(super) fn va(&self) -> Vaddr {
        self.va
    }

    /// Traverse forward in the current level to the next page.
    pub(super) fn next_page(&mut self) {
        self.va += page_size::<P>(self.level);
    }

    /// A level up operation during traversal. It usually happens when completing
    /// the traversal a child PT frame and go back to the parent PT frame.
    pub(super) fn level_up(&mut self) {
        self.stack[P::NR_LEVELS - self.level] = None;
        self.level += 1;
    }

    /// A level down operation during traversal. It may split a huge page into
    /// smaller pages if we have an end address within the next mapped huge page.
    /// It may also create a new child frame if the current frame does not have one.
    /// If that may happen the map property of intermediate level `prop` should be
    /// passed in correctly. Whether the map property matters in an intermediate
    /// level is architecture-dependent.
    pub(super) unsafe fn level_down(&mut self, prop: Option<MapProperty>) {
        self.stack[P::NR_LEVELS - self.level + 1] = Some({
            let mut last = self.stack[P::NR_LEVELS - self.level].clone().unwrap();
            if last.child.is_none() {
                let new_frame = VmAllocOptions::new(1).alloc_single().unwrap();
                let cur_pte = {
                    let frame_addr = paddr_to_vaddr(last.inner.start_paddr());
                    let offset = in_frame_index::<P>(self.va, self.level);
                    &mut *(frame_addr as *const T).add(offset)
                };
                // If it already maps a huge page, we should split it.
                if cur_pte.is_valid() && cur_pte.is_last() {
                    let huge_prop = cur_pte.prop();
                    let pa = cur_pte.paddr();
                    for i in 0..nr_entries_per_frame::<P>() {
                        let nxt_level = self.level - 1;
                        let nxt_pte = {
                            let frame_addr = paddr_to_vaddr(new_frame.start_paddr());
                            &mut *(frame_addr as *const T).add(i)
                        };
                        *nxt_pte = T::new(pa + i * page_size::<P>(nxt_level), huge_prop, nxt_level > 1, true);
                    }
                    *cur_pte = T::new(new_frame.start_paddr(), huge_prop, false, false);
                } else {
                    *cur_pte = T::new(new_frame.start_paddr(), prop.unwrap(), false, false);
                }
                let new_frame = Arc::new(PageTableFrame::<T, P> {
                    inner: new_frame,
                    child: None,
                    _phantom: PhantomData,
                });
                let child = Box::new([None; nr_entries_per_frame::<P>()]);
                child[in_frame_index::<P>(self.va, self.level)] = Some(new_frame);
                last.child = Some(child);
            }
            let Some(child) = last.child else {
                panic!("The child frame could never be non-existent since we created it.");
            };
            child[in_frame_index::<P>(self.va, self.level)].clone().unwrap()
        });
        self.level -= 1;
    }

    /// Map or unmap the page pointed to by the cursor (which could be large).
    /// If the physical address and the map property are not provided, it unmaps
    /// the current page.
    pub(super) unsafe fn map(&mut self, create: Option<(Paddr, MapProperty)>) {
        let cur_pte = {
            let frame_addr = paddr_to_vaddr(self.stack[P::NR_LEVELS - self.level].clone().unwrap().inner.start_paddr());
            let offset = in_frame_index::<P>(self.va, self.level);
            &mut *(frame_addr as *const T).add(offset)
        };
        if let (pa, prop) = create {
            *cur_pte = T::new(pa, prop, self.level > 1, true);
        } else {
            *cur_pte = T::new_invalid();
        }
        // If it dismantle a child we ensure it to be released.
        let mut child = self.stack[P::NR_LEVELS - self.level].clone().unwrap().child;
        if let Some(mut child) = child {
            let idx = in_frame_index::<P>(self.va, self.level);
            child[idx] = None;
        }
    }

    /// Map or unmap contiguous pages starting from the current cursor position.
    pub(super) unsafe fn map_contiguous(&mut self, len: usize, create: Option<(Paddr, MapProperty)>) {
        let end = self.va() + len;
        while self.va() != end {
            // Go down if the page size is too big or alignment is not satisfied.
            if self.level() > P::HIGHEST_TRANSLATION_LEVEL
                || (self.va() % page_size::<P>(self.level())) != 0
                || (self.va() % page_size::<P>(self.level())) != 0
                || self.va() + page_size::<P>(self.level()) > end
            {
                let ld_prop = create.map(|(pa, prop)| prop).unwrap_or(MapProperty::new_invalid());
                self.level_down(Some(ld_prop));
                continue;
            }
            // Go up if larger pages can be used.
            if self.level() < P::HIGHEST_TRANSLATION_LEVEL
                && (self.va() % page_size::<P>(self.level() + 1)) == 0
                && (self.va() % page_size::<P>(self.level() + 1)) == 0
                && self.va() + page_size::<P>(self.level() + 1) <= end
            {
                self.level_up();
                continue;
            }
            // Map the page.
            self.map(create);

            self.next_page();
        }
    }
}
