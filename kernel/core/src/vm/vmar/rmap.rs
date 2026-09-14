// SPDX-License-Identifier: MPL-2.0

//! Reverse mappings from backing objects to the VMARs that map them.
//!
//! Reverse walkers hold the reverse-map mutex through the complete walk and
//! then acquire page-table locks. Forward mapping changes may discover backing
//! objects under a cursor, but only try-lock their reverse maps; on contention
//! they release every atomic-mode guard and restart before mutation. Holding
//! the reverse-map mutex across a walk prevents fork from publishing unseen
//! mappings and preserves the page cache's rmap-before-page lock order.

use alloc::{collections::btree_map::BTreeMap, sync::Weak, vec::Vec};
use core::ops::Range;

use keyable_arc::KeyableWeak;
use ostd::{
    mm::{PAGE_SIZE, PageFlags, Vaddr, tlb::TlbFlushOp},
    task::disable_preempt,
};

use crate::vm::vmar::{
    RssType, Vmar,
    cursor::CursorMutExt,
    vmar_impls::{PteRangeMeta, RsAsDelta},
};

/// Reverse mappings from a backing object to [`Vmar`]s.
///
/// Both page-cache VMOs and device memory objects use this index.
pub(crate) struct Rmap {
    entries: BTreeMap<KeyableWeak<Vmar>, Vec<RmapEntry>>,
}

impl core::fmt::Debug for Rmap {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Rmap")
            .field("num_vmars", &self.entries.len())
            .finish_non_exhaustive()
    }
}

/// A reverse mapping entry.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RmapEntry {
    /// The virtual address.
    pub vaddr: Vaddr,
    /// The backing-object offset.
    pub offset: usize,
    /// The mapping size.
    pub size: usize,
}

impl Rmap {
    pub(in crate::vm) const fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }

    /// Inserts a new reverse mapping entry.
    pub(crate) fn insert(&mut self, vmar: Weak<Vmar>, entry: RmapEntry) {
        self.entries
            .entry(KeyableWeak::from(vmar))
            .or_default()
            .push(entry)
    }

    /// Removes the part of this VMAR's entries that overlaps `range`.
    ///
    /// Entries crossing either boundary are split while preserving their
    /// backing-object offsets. The range need not start at an entry.
    pub(crate) fn remove_range(&mut self, vmar: Weak<Vmar>, range: &Range<Vaddr>) {
        use alloc::collections::btree_map::Entry;

        let key = KeyableWeak::from(vmar);
        let Entry::Occupied(mut map_entry) = self.entries.entry(key) else {
            return;
        };

        let entries = map_entry.get_mut();
        let mut replacement = Vec::with_capacity(entries.len() + 1);
        for entry in entries.drain(..) {
            let entry_end = entry.vaddr + entry.size;
            let overlap_start = entry.vaddr.max(range.start);
            let overlap_end = entry_end.min(range.end);

            if overlap_start >= overlap_end {
                replacement.push(entry);
                continue;
            }

            if entry.vaddr < overlap_start {
                replacement.push(RmapEntry {
                    size: overlap_start - entry.vaddr,
                    ..entry
                });
            }
            if overlap_end < entry_end {
                replacement.push(RmapEntry {
                    vaddr: overlap_end,
                    offset: entry.offset + overlap_end - entry.vaddr,
                    size: entry_end - overlap_end,
                });
            }
        }

        *entries = replacement;
        if entries.is_empty() {
            map_entry.remove();
        }
    }

    /// Unmaps an offset range through every reverse mapping.
    ///
    /// # Panics
    ///
    /// This method may panic if the offset range is not aligned to the page boundary.
    pub(crate) fn unmap(&mut self, offset: Range<usize>) {
        debug_assert!(offset.start.is_multiple_of(PAGE_SIZE));
        debug_assert!(offset.end.is_multiple_of(PAGE_SIZE));

        let keys: Vec<_> = self.entries.keys().cloned().collect();

        for key in keys {
            let Some(vmar) = key.upgrade() else {
                self.entries.remove(&key);
                continue;
            };
            let entries = self.entries.get(&key).unwrap();

            let mut rs_as_delta = RsAsDelta::new(&vmar);

            for entry in entries {
                let vmo_range =
                    entry.offset.max(offset.start)..(entry.offset + entry.size).min(offset.end);
                if vmo_range.is_empty() {
                    continue;
                }

                let addr_range = (vmo_range.start - entry.offset + entry.vaddr)
                    ..(vmo_range.end - entry.offset + entry.vaddr);

                let preempt_guard = disable_preempt();
                let mut cursor_mut = vmar
                    .vm_space()
                    .cursor_mut(&preempt_guard, &addr_range)
                    .unwrap();
                while let Some(va) = cursor_mut.find_next(addr_range.end) {
                    cursor_mut.split_if_map_exceeds_range(&addr_range);
                    let page_range = cursor_mut.cur_va_range();
                    let num_unmapped = cursor_mut.unmap();

                    // Reverse entries can span several VMAs or PT nodes after
                    // splitting/protection. Account at each PTE's metadata,
                    // not against only the first mapping in the entry.
                    let Some(PteRangeMeta::VmMapping(mapping)) =
                        cursor_mut.aux_meta_mut().inner.find_one_mut(&va)
                    else {
                        panic!("reverse mapping points outside a VM mapping");
                    };
                    mapping.dec_frames_mapped(num_unmapped);
                    rs_as_delta.add_rs(RssType::RSS_FILEPAGES, -(num_unmapped as isize));
                    if cursor_mut.jump(page_range.end).is_err() {
                        break;
                    }
                }
                cursor_mut.flusher().dispatch_tlb_flush();
                cursor_mut.flusher().sync_tlb_flush();
            }

            drop(rs_as_delta);
        }
    }

    /// Makes an offset range read-only and clears its accessed/dirty PTE bits.
    ///
    /// # Panics
    ///
    /// This method may panic if the offset range is not aligned to the page boundary.
    pub(crate) fn freeze(&mut self, offset: Range<usize>) {
        debug_assert!(offset.start.is_multiple_of(PAGE_SIZE));
        debug_assert!(offset.end.is_multiple_of(PAGE_SIZE));

        let keys: Vec<_> = self.entries.keys().cloned().collect();

        for key in keys {
            let Some(vmar) = key.upgrade() else {
                self.entries.remove(&key);
                continue;
            };
            let entries = self.entries.get(&key).unwrap();

            for entry in entries {
                let vmo_range =
                    entry.offset.max(offset.start)..(entry.offset + entry.size).min(offset.end);
                if vmo_range.is_empty() {
                    continue;
                }

                let addr_range = (vmo_range.start - entry.offset + entry.vaddr)
                    ..(vmo_range.end - entry.offset + entry.vaddr);

                let preempt_guard = disable_preempt();
                let mut cursor_mut = vmar
                    .vm_space()
                    .cursor_mut(&preempt_guard, &addr_range)
                    .unwrap();
                while cursor_mut.find_next(addr_range.end).is_some() {
                    cursor_mut.split_if_map_exceeds_range(&addr_range);
                    cursor_mut.protect(|page_flags, _| {
                        *page_flags -= PageFlags::W | PageFlags::ACCESSED | PageFlags::DIRTY;
                    });
                    let va = cursor_mut.cur_va_range();
                    cursor_mut
                        .flusher()
                        .issue_tlb_flush(TlbFlushOp::for_range(va.clone()));
                    if cursor_mut.jump(va.end).is_err() {
                        break;
                    }
                }
                cursor_mut.flusher().dispatch_tlb_flush();
                cursor_mut.flusher().sync_tlb_flush();
            }
        }
    }
}
