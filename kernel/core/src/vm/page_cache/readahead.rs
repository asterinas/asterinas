// SPDX-License-Identifier: MPL-2.0

//! Readahead policy for loading file pages before they are requested.

use core::ops::Range;

use super::{CachePage, CachePageExt, Vmo};
use crate::prelude::*;

/// Readahead policy and state for one opened file handle.
///
/// Readahead loads file pages into the cache before they are requested,
/// reducing I/O wait time for later reads or memory-mapped accesses.
///
/// Each [`InodeHandle`](crate::fs::file::InodeHandle) owns its readahead state.
/// Duplicated descriptors and mappings of that handle share the state;
/// independent opens have separate state but share the file's page cache.
///
/// A *window* is a contiguous range of pages to read or prefetch.
/// Readahead loads only missing pages, within the size of the page-cache VMO.
/// Each window selects a *trigger page* to start the next window.
/// If readahead newly inserts that trigger page into the cache, it sets a one-use *marker* on it.
/// A buffered read or mapping fault consumes (clears) the marker
/// and submits the next prefetch without waiting for I/O completion.
///
/// In the examples below, page indices are zero-based:
/// `D` = requested page, `P` = prefetched page, `*` = marker.
/// Prefetched pages are assumed missing and the file large enough.
///
/// # Buffered reads
///
/// Before copying each page, a cache miss starts a window at that page.
/// This submits I/O for the requested pages and any speculative pages together.
/// A marked page starts the next window when the read reaches that page.
/// Completed reads update the per-handle position used to recognize sequential access.
///
/// For example, an initial one-page read creates a window containing additional pages.
/// Accessing its marker through the same handle starts the next window:
///
/// ```text
/// read page 0:   [0 D] [1 P*] [2 P] [3 P]
///
/// read or fault on page 1 --> next window: [4 P*] [5..11 P]
/// ```
///
/// # Mapping faults
///
/// A mapping fault on a missing cache page requests a *read-around* window
/// covering the faulting page and nearby pages on either side, within the file's bounds.
/// Its trigger lies in the later part of the window to continue prefetching forward.
/// A fault on an already cached page triggers prefetching only if it consumes a marker.
///
/// # Sharing markers across handles
///
/// Markers belong to cached pages, so one handle can consume a marker set by another.
/// When consuming a marker, a handle continues after its window
/// if the access includes its expected trigger.
/// Otherwise, it searches a bounded range after the access for the first missing page
/// and starts there, or skips prefetching if none is found.
/// A buffered read can consume another marker later in the same request.
///
/// Here A and B are independent opens, and B has no previous window.
/// B consumes A's marker and finds the first missing page at index 32:
///
/// ```text
/// A faults on missing page 16:
///        |<-- 16 pages -->|
/// cache: [0..15 P]        [16 D] [17..23 P] [24 P*] [25..31 P]
///
/// B faults on page 24 --> next window: [32 P*] [33..63 P]
/// ```
pub struct Readahead {
    state: Mutex<ReadaheadState>,
}

impl Default for Readahead {
    fn default() -> Self {
        Self {
            state: Mutex::new(ReadaheadState::default()),
        }
    }
}

impl Readahead {
    /// Starts readahead for missing pages found by a buffered read.
    ///
    /// `read_pages` includes the remaining requested pages, while
    /// `request_start_idx` identifies the first page of this read for stream detection.
    pub(super) fn on_read_miss(
        &self,
        vmo: &Vmo,
        read_pages: Range<usize>,
        request_start_idx: usize,
    ) {
        if read_pages.is_empty() {
            return;
        }
        let Some(mut state) = self.state.try_lock() else {
            return;
        };
        let (range, marker_idx) = state.on_read_miss(read_pages, request_start_idx);
        drop(state);

        let _ = vmo.readahead(range, marker_idx);
    }

    /// Continues readahead from a marked page collected by a buffered read.
    pub(super) fn on_read_marker(
        &self,
        vmo: &Vmo,
        page_idx: usize,
        page: &CachePage,
        nr_read_pages: usize,
    ) {
        let Some(mut state) = self.state.try_lock() else {
            return;
        };
        // Consume through the collected reference: another reader may have
        // consumed it already, or writeback may require preserving it.
        if !page.take_readahead_marker() {
            return;
        }
        let Some(range) = state.on_marker(vmo, page_idx, nr_read_pages) else {
            return;
        };
        let marker_idx = Some(state.trigger_page_idx);
        drop(state);

        let _ = vmo.readahead(range, marker_idx);
    }

    /// Records the number of bytes actually read for sequential-read detection.
    pub(crate) fn finish_read(&self, offset: usize, read_len: usize) {
        if read_len == 0 {
            return;
        }
        let Some(read_end) = offset.checked_add(read_len) else {
            return;
        };
        self.state.lock().prev_pos = Some(read_end);
    }

    /// Prefetches around a mapping fault if the page is missing from the cache,
    /// or starts the next window if the page carries a marker.
    ///
    /// The caller must release page-table locks and enable preemption first.
    /// This method does not wait for the requested page to become readable.
    /// The caller must still resolve the fault and load the demand page if necessary.
    pub(crate) fn on_fault(&self, vmo: &Vmo, page_idx: usize) {
        let Some(mut state) = self.state.try_lock() else {
            return;
        };
        let Some(range) = state.on_fault(vmo, page_idx) else {
            return;
        };
        let marker_idx = Some(state.trigger_page_idx);
        drop(state);

        let _ = vmo.readahead(range, marker_idx);
    }
}

/// A page-cache event that triggers readahead for an accessed page.
pub(super) enum ReadaheadTrigger {
    /// The page has no cache entry.
    CacheMiss,
    /// A readahead marker was consumed from the page.
    Marker,
}

/// Maximum readahead window in pages.
// TODO: Replace this fixed cap with a per-open-file limit initialized from the
// backing device's configurable readahead setting, and honor fadvise overrides.
// Reference: <https://elixir.bootlin.com/linux/v6.12.113/source/mm/readahead.c#L138-L143>.
const MAX_READAHEAD_PAGES: usize = 32;

/// Per-open-file readahead heuristic state.
#[derive(Debug, Default)]
struct ReadaheadState {
    /// Exclusive end offset of the last recorded buffered read, in bytes.
    prev_pos: Option<usize>,
    /// Page index where the most recent readahead window started.
    start_page_idx: usize,
    /// Total size of the most recent readahead window in pages.
    nr_window_pages: usize,
    /// Page index selected to carry the marker that triggers the next window.
    trigger_page_idx: usize,
}

impl ReadaheadState {
    /// Selects a readahead window based on the faulting page's cache status.
    ///
    /// Returns the page-index range to prefetch, or `None` to skip readahead.
    fn on_fault(&mut self, vmo: &Vmo, page_idx: usize) -> Option<Range<usize>> {
        let trigger = vmo.take_readahead_trigger(page_idx)?;
        match trigger {
            ReadaheadTrigger::CacheMiss => {
                // Place up to half of the fixed read-around window before
                // the fault, with the final quarter triggering continuation.
                // Reference: <https://elixir.bootlin.com/linux/v6.12.113/source/mm/filemap.c#L3207-L3215>.
                self.start_page_idx = page_idx.saturating_sub(MAX_READAHEAD_PAGES / 2);
                self.nr_window_pages = MAX_READAHEAD_PAGES;
                self.trigger_page_idx =
                    self.start_page_idx + MAX_READAHEAD_PAGES - MAX_READAHEAD_PAGES / 4;
            }
            ReadaheadTrigger::Marker => {
                return self.on_marker(vmo, page_idx, MAX_READAHEAD_PAGES);
            }
        }

        Some(self.start_page_idx..self.start_page_idx.saturating_add(self.nr_window_pages))
    }

    /// Continues readahead after either access path consumes a marker.
    ///
    /// Returns the page-index range to prefetch, or `None` if no next window is found.
    fn on_marker(
        &mut self,
        vmo: &Vmo,
        page_idx: usize,
        nr_growth_pages: usize,
    ) -> Option<Range<usize>> {
        let (start_page_idx, nr_current_pages) =
            if self.nr_window_pages != 0 && self.trigger_page_idx == page_idx {
                // Consuming the expected marker continues prefetching at
                // the previous window's end.
                // Reference: <https://elixir.bootlin.com/linux/v6.12.113/source/mm/readahead.c#L640-L650>.
                (
                    self.start_page_idx + self.nr_window_pages,
                    self.nr_window_pages,
                )
            } else {
                // Reconstruct the window from the next missing page when
                // the consumed marker does not match our window.
                // Reference: <https://elixir.bootlin.com/linux/v6.12.113/source/mm/readahead.c#L653-L670>.
                let search_start = page_idx.saturating_add(1);
                let next_idx = vmo.next_missing_page(
                    search_start..search_start.saturating_add(MAX_READAHEAD_PAGES),
                )?;
                (
                    next_idx,
                    (next_idx - page_idx).saturating_add(nr_growth_pages),
                )
            };
        self.start_page_idx = start_page_idx;
        self.nr_window_pages = Self::next_window_pages(nr_current_pages);
        self.trigger_page_idx = start_page_idx;

        Some(start_page_idx..start_page_idx.saturating_add(self.nr_window_pages))
    }

    /// Selects a bounded window starting at the first missing requested page.
    fn on_read_miss(
        &mut self,
        read_pages: Range<usize>,
        request_start_idx: usize,
    ) -> (Range<usize>, Option<usize>) {
        let first_missing = read_pages.start;
        let nr_requested_pages = (read_pages.end - first_missing).min(MAX_READAHEAD_PAGES);
        if !self.is_sequential_read(request_start_idx) {
            // A standalone random request must not seed the next window or
            // inherit a marker from an unrelated stream.
            return (first_missing..first_missing + nr_requested_pages, None);
        }

        // Initialize sequential misses from the remaining demand length.
        // Mark the first speculative page, or the window's midpoint when
        // all its pages are requested.
        // Reference: <https://elixir.bootlin.com/linux/v6.12.113/source/mm/readahead.c#L565-L604>.
        let nr_window_pages = Self::initial_window_pages(nr_requested_pages);

        self.start_page_idx = first_missing;
        self.nr_window_pages = nr_window_pages;
        self.trigger_page_idx = first_missing
            + if nr_window_pages > nr_requested_pages {
                nr_requested_pages
            } else {
                nr_window_pages / 2
            };

        (
            first_missing..first_missing.saturating_add(nr_window_pages),
            Some(self.trigger_page_idx),
        )
    }

    /// Returns whether the read continues the previous page-level stream.
    fn is_sequential_read(&self, read_start_idx: usize) -> bool {
        let Some(prev_pos) = self.prev_pos else {
            return read_start_idx == 0;
        };

        let prev_page_idx = prev_pos / PAGE_SIZE;
        read_start_idx == prev_page_idx || read_start_idx == prev_page_idx + 1
    }

    /// Returns the initial total window size in pages.
    ///
    /// Reference:
    /// <https://elixir.bootlin.com/linux/v6.12.113/source/mm/readahead.c#L361-L373>.
    fn initial_window_pages(nr_read_pages: usize) -> usize {
        debug_assert!(nr_read_pages > 0);

        if nr_read_pages > MAX_READAHEAD_PAGES / 4 {
            return MAX_READAHEAD_PAGES;
        }

        let base = nr_read_pages.next_power_of_two();
        if base <= MAX_READAHEAD_PAGES / 32 {
            base * 4
        } else if base <= MAX_READAHEAD_PAGES / 4 {
            base * 2
        } else {
            MAX_READAHEAD_PAGES
        }
    }

    /// Returns the next total window size in pages.
    ///
    /// Reference:
    /// <https://elixir.bootlin.com/linux/v6.12.113/source/mm/readahead.c#L379-L389>.
    fn next_window_pages(nr_current_pages: usize) -> usize {
        if nr_current_pages < MAX_READAHEAD_PAGES / 16 {
            nr_current_pages * 4
        } else if nr_current_pages <= MAX_READAHEAD_PAGES / 2 {
            nr_current_pages * 2
        } else {
            MAX_READAHEAD_PAGES
        }
    }
}
