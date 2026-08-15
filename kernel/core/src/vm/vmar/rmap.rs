// SPDX-License-Identifier: MPL-2.0

//! Reverse mappings from VMOs to the VMARs that map them.
//!
//! Forward mapping changes hold the page-table cursor before updating an
//! `Rmap`. Reverse walkers hold the `Rmap` first, so they must only try to
//! acquire page-table cursors. On contention they drop the `Rmap`, wait for the
//! conflicting cursor, and resume. This preserves syscall atomicity without a
//! separate mmap lock and avoids a page-table/rmap lock cycle.

use alloc::{
    collections::btree_map::BTreeMap,
    sync::{Arc, Weak},
    vec::Vec,
};
use core::ops::Range;

use keyable_arc::KeyableWeak;
use ostd::{
    mm::{PAGE_SIZE, PageFlags, Vaddr, tlb::TlbFlushOp},
    task::disable_preempt,
};

use crate::vm::vmar::{
    RssType, Vmar,
    cursor::{CursorExt, CursorMutExt},
    vmar_impls::{PteRangeMeta, RsAsDelta},
};

/// Reverse mappings from a [`Vmo`] to [`Vmar`]s.
///
/// [`Vmo`]: crate::vm::page_cache::Vmo
pub(crate) struct Rmap {
    entries: BTreeMap<KeyableWeak<Vmar>, Vec<RmapEntry>>,
}

/// A reverse mapping entry.
#[derive(Copy, Clone, Debug)]
pub(crate) struct RmapEntry {
    /// The virtual address.
    pub vaddr: Vaddr,
    /// The VMO offset.
    pub offset: usize,
    /// The mapping size.
    pub size: usize,
}

/// State needed to retry a reverse-map walk after cursor contention.
pub(crate) struct RmapRetry {
    key: KeyableWeak<Vmar>,
    vmar: Arc<Vmar>,
    addr_range: Range<Vaddr>,
}

impl RmapRetry {
    /// Waits for the conflicting cursor and returns the VMAR key to resume at.
    pub(crate) fn wait(self) -> KeyableWeak<Vmar> {
        let preempt_guard = disable_preempt();
        let cursor = self
            .vmar
            .vm_space()
            .cursor_mut(&preempt_guard, &self.addr_range)
            .unwrap();
        drop(cursor);
        self.key
    }
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
    /// Entries crossing either boundary are split while preserving their VMO
    /// offsets. Unlike [`Self::remove`], the range need not start at an entry.
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

    /// Tries to unmap an offset range through every reverse mapping.
    ///
    /// `resume_at` is inclusive because the previous attempt did not process
    /// the VMAR whose cursor was contended. Earlier entries in that VMAR may be
    /// visited again; unmapping them is idempotent.
    ///
    /// # Panics
    ///
    /// This method may panic if the offset range is not aligned to the page boundary.
    pub(crate) fn try_unmap(
        &mut self,
        offset: Range<usize>,
        resume_at: Option<&KeyableWeak<Vmar>>,
    ) -> Result<(), RmapRetry> {
        debug_assert!(offset.start.is_multiple_of(PAGE_SIZE));
        debug_assert!(offset.end.is_multiple_of(PAGE_SIZE));

        let keys: Vec<_> = if let Some(key) = resume_at {
            self.entries
                .range(key.clone()..)
                .map(|(key, _)| key.clone())
                .collect()
        } else {
            self.entries.keys().cloned().collect()
        };

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
                let Some(mut cursor_mut) = vmar
                    .vm_space()
                    .try_cursor_mut(&preempt_guard, &addr_range)
                    .unwrap()
                else {
                    return Err(RmapRetry {
                        key: key.clone(),
                        vmar: vmar.clone().into(),
                        addr_range,
                    });
                };
                let mut num_unmapped = 0;
                while cursor_mut
                    .find_next_unmappable_subtree(addr_range.end)
                    .is_some()
                {
                    cursor_mut.split_if_map_exceeds_range(&addr_range);
                    num_unmapped += cursor_mut.unmap();
                }

                cursor_mut.jump(addr_range.start).unwrap();
                let mapping_addr = cursor_mut
                    .find_next_mapped(addr_range.end)
                    .expect("reverse mapping points outside a VM mapping")
                    .map_to_addr();
                let Some(PteRangeMeta::VmMapping(mapping)) =
                    cursor_mut.aux_meta_mut().inner.find_one_mut(&mapping_addr)
                else {
                    panic!("reverse mapping points outside a VM mapping");
                };
                mapping.dec_frames_mapped(num_unmapped);

                rs_as_delta.add_rs(RssType::RSS_FILEPAGES, -(num_unmapped as isize));
                cursor_mut.flusher().dispatch_tlb_flush();
                cursor_mut.flusher().sync_tlb_flush();
            }

            drop(rs_as_delta);
        }

        Ok(())
    }

    /// Tries to freeze (make read-only) an offset range in every reverse mapping.
    ///
    /// # Panics
    ///
    /// This method may panic if the offset range is not aligned to the page boundary.
    pub(crate) fn try_freeze(
        &mut self,
        offset: Range<usize>,
        resume_at: Option<&KeyableWeak<Vmar>>,
    ) -> Result<(), RmapRetry> {
        debug_assert!(offset.start.is_multiple_of(PAGE_SIZE));
        debug_assert!(offset.end.is_multiple_of(PAGE_SIZE));

        let keys: Vec<_> = if let Some(key) = resume_at {
            self.entries
                .range(key.clone()..)
                .map(|(key, _)| key.clone())
                .collect()
        } else {
            self.entries.keys().cloned().collect()
        };

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
                let Some(mut cursor_mut) = vmar
                    .vm_space()
                    .try_cursor_mut(&preempt_guard, &addr_range)
                    .unwrap()
                else {
                    return Err(RmapRetry {
                        key: key.clone(),
                        vmar: vmar.clone().into(),
                        addr_range,
                    });
                };
                while cursor_mut.find_next(addr_range.end).is_some() {
                    cursor_mut.split_if_map_exceeds_range(&addr_range);
                    cursor_mut.protect(|page_flags, _| *page_flags -= PageFlags::W);
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

        Ok(())
    }
}
