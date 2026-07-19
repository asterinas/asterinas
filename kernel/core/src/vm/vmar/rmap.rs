// SPDX-License-Identifier: MPL-2.0

use alloc::{collections::btree_map::BTreeMap, sync::Weak, vec::Vec};
use core::ops::Range;

use keyable_arc::KeyableWeak;
use ostd::{
    mm::{PAGE_SIZE, PageFlags, Vaddr, tlb::TlbFlushOp},
    task::disable_preempt,
};

use crate::vm::vmar::{RssType, Vmar, vmar_impls::RssDelta};

/// Reverse mappings from a [`Vmo`] to [`Vmar`]s.
///
/// [`Vmo`]: crate::vm::page_cache::Vmo
pub struct Rmap {
    entries: BTreeMap<KeyableWeak<Vmar>, Vec<RmapEntry>>,
}

/// A reverse mapping entry.
#[derive(Copy, Clone, Debug)]
pub struct RmapEntry {
    /// The virtual address.
    pub vaddr: Vaddr,
    /// The VMO offset.
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
    pub fn insert(&mut self, vmar: Weak<Vmar>, entry: RmapEntry) {
        self.entries
            .entry(KeyableWeak::from(vmar))
            .or_default()
            .push(entry)
    }

    /// Removes a reverse mapping entry.
    ///
    /// # Panics
    ///
    /// This method will panic if the reverse mapping entry does not exist.
    pub fn remove(&mut self, vmar: Weak<Vmar>, vaddr: Vaddr) {
        use alloc::collections::btree_map::Entry;

        let key = KeyableWeak::from(vmar);
        let Entry::Occupied(mut map_entry) = self.entries.entry(key) else {
            panic!("the entry to remove does not exist")
        };

        let entries = map_entry.get_mut();
        let index = entries
            .iter()
            .position(|entry| entry.vaddr == vaddr)
            .expect("the entry to remove does not exist");
        entries.swap_remove(index);

        if entries.is_empty() {
            map_entry.remove();
        }
    }

    /// Iterates over all reverse mappings and unmaps the given offset range.
    ///
    /// # Panics
    ///
    /// This method may panic if the offset range is not aligned to the page boundary.
    pub fn unmap(&mut self, offset: Range<usize>) {
        debug_assert!(offset.start.is_multiple_of(PAGE_SIZE));
        debug_assert!(offset.end.is_multiple_of(PAGE_SIZE));

        self.entries.retain(|vmar, entries| {
            let Some(vmar) = vmar.upgrade() else {
                return false;
            };

            let mut rss_delta = RssDelta::new(&vmar);

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
                rss_delta.add(
                    RssType::File,
                    -(cursor_mut.unmap(addr_range.len()) as isize),
                );
                cursor_mut.flusher().dispatch_tlb_flush();
                cursor_mut.flusher().sync_tlb_flush();
            }

            true
        });
    }

    /// Iterates over all reverse mappings and freezes (i.e., makes read-only) the given offset
    /// range.
    ///
    /// # Panics
    ///
    /// This method may panic if the offset range is not aligned to the page boundary.
    pub fn freeze(&mut self, offset: Range<usize>) {
        debug_assert!(offset.start.is_multiple_of(PAGE_SIZE));
        debug_assert!(offset.end.is_multiple_of(PAGE_SIZE));

        self.entries.retain(|vmar, entries| {
            let Some(vmar) = vmar.upgrade() else {
                return false;
            };

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
                loop {
                    let addr = cursor_mut.virt_addr();
                    if addr >= addr_range.end {
                        break;
                    }
                    let len = addr_range.end - addr;
                    if let Some(va) =
                        cursor_mut.protect_next(len, |page_flags, _| *page_flags -= PageFlags::W)
                    {
                        cursor_mut
                            .flusher()
                            .issue_tlb_flush(TlbFlushOp::for_range(va));
                    } else {
                        break;
                    }
                }
                cursor_mut.flusher().dispatch_tlb_flush();
                cursor_mut.flusher().sync_tlb_flush();
            }

            true
        });
    }
}
