// SPDX-License-Identifier: MPL-2.0

//! Readahead policy shared by buffered reads and mappings of one open file.

use core::ops::Range;

use super::Vmo;
use crate::prelude::*;

/// Readahead state belonging to an open file description.
///
/// Readahead speculatively loads file pages into the page cache before they are
/// requested. A readahead window is a contiguous range of pages submitted for
/// this speculative load. The requested page is called the demand page; pages
/// loaded only because they are in the window are prefetched pages.
///
/// A marker is stored on one cached page in a window. Reaching that page tells
/// the readahead code to submit the next window before the current one runs
/// out. Here, “next window” means the contiguous range immediately after the
/// current window, not a range starting at the page that consumed the marker.
/// For example, a sequential read of page 0 starts the four-page window
/// `[0, 4)`: page 0 is the demand page, pages 1 through 3 are prefetched, and
/// page 1 carries the marker. A mapping fault on page 1 consumes that marker
/// and starts the next window at page 4, growing it to `[4, 12)`.
///
/// ```text
/// first window:  [0 D] [1 M/P] [2 P] [3 P]
///                                      |
///                                      | marker consumed by a fault on page 1
///                                      v
/// next window:   [4 P] [5 P] [6 P] ... [11 P]
/// ```
///
/// A mapping fault on a page that is not cached starts a different kind of
/// window. The window has a fixed maximum size, is centered around the demand
/// page, and places its marker near the end so another window can start before
/// the current one is exhausted. When at least 16 pages precede a fault on
/// page `F`, the initial window is `[F - 16, F + 16)` and its marker is at
/// page `F + 8`. Near the beginning of the file, the window start is clamped
/// to page 0, so the marker is no longer exactly eight pages after `F`.
///
/// Markers belong to cached pages rather than to an open file's private state.
/// Therefore, a different open file can consume a marker left by the first
/// open file. If that page is not the second open file's expected trigger, it
/// finds the first missing page `q` in a bounded range after marker page `p`
/// and starts a new window at `q`.
///
/// ```text
/// open A has cached: [0 ................. 23] [24 M] [25 ......... 31] [32 ?]
///                                           p                 q
///
/// open B faults on p
///         |
///         +-- consumes the marker on p
///         +-- finds q, the first missing page in the bounded range after p
///         +-- starts a new prefetch window at q and fetches forward from q
/// ```
pub(crate) struct Readahead {
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
    /// Records a completed buffered read and submits speculative I/O.
    pub(crate) fn on_read(&self, vmo: &Vmo, offset: usize, read_len: usize) {
        if read_len == 0 {
            return;
        }
        let Some(mut state) = self.state.try_lock() else {
            return;
        };
        let Some(range) = state.on_read(offset, read_len) else {
            return;
        };
        let marker_idx = state.trigger_page_idx;
        drop(state);

        let _ = vmo.readahead(range, Some(marker_idx));
    }

    /// Prefetches around a cache miss or continues a marked readahead window.
    ///
    /// The caller must release page-table locks and enable preemption first.
    /// Speculative failures leave the demand fault path responsible for the page.
    pub(crate) fn on_fault(&self, vmo: &Vmo, page_idx: usize) {
        let Some(mut state) = self.state.try_lock() else {
            return;
        };
        let Some(range) = state.on_fault(vmo, page_idx) else {
            return;
        };
        let marker_idx = state.trigger_page_idx;
        drop(state);

        let _ = vmo.readahead(range, Some(marker_idx));
    }
}

/// A cache event that can trigger file readahead.
pub(super) enum ReadaheadTrigger {
    /// The faulting page has not been cached.
    CacheMiss,
    /// The faulting page carried a readahead marker.
    Marker,
}

/// Maximum readahead window in pages.
const MAX_READAHEAD_PAGES: usize = 32;

/// Per-open-file readahead heuristic state.
#[derive(Debug, Default)]
struct ReadaheadState {
    /// Byte offset immediately after the most recent read.
    prev_pos: Option<usize>,
    /// Page index where the most recent readahead window started.
    start_page_idx: usize,
    /// Total size of the most recent readahead window in pages.
    nr_window_pages: usize,
    /// Page index whose access triggers the next readahead window.
    trigger_page_idx: usize,
}

impl ReadaheadState {
    /// Consumes a cache trigger and returns the next speculative window.
    fn on_fault(&mut self, vmo: &Vmo, page_idx: usize) -> Option<Range<usize>> {
        let trigger = vmo.take_readahead_trigger(page_idx)?;
        match trigger {
            ReadaheadTrigger::CacheMiss => {
                // Match Linux's mmap read-around layout: half the window before
                // the fault, with the final quarter triggering continuation.
                // Reference: <https://elixir.bootlin.com/linux/v7.1-rc7/source/mm/filemap.c#L3398>.
                self.start_page_idx = page_idx.saturating_sub(MAX_READAHEAD_PAGES / 2);
                self.nr_window_pages = MAX_READAHEAD_PAGES;
                self.trigger_page_idx =
                    self.start_page_idx + MAX_READAHEAD_PAGES - MAX_READAHEAD_PAGES / 4;
            }
            ReadaheadTrigger::Marker => {
                let (start_page_idx, nr_current_pages) =
                    if self.nr_window_pages != 0 && page_idx == self.trigger_page_idx {
                        // Reference: <https://elixir.bootlin.com/linux/v7.1-rc7/source/mm/readahead.c#L664>.
                        (
                            self.start_page_idx + self.nr_window_pages,
                            self.nr_window_pages,
                        )
                    } else {
                        // Reference: <https://elixir.bootlin.com/linux/v7.1-rc7/source/mm/readahead.c#L681>.
                        let next_idx = vmo.next_missing_page(
                            page_idx.saturating_add(1)
                                ..page_idx.saturating_add(MAX_READAHEAD_PAGES + 1),
                        )?;
                        (next_idx, next_idx - page_idx + MAX_READAHEAD_PAGES)
                    };
                self.start_page_idx = start_page_idx;
                self.nr_window_pages = Self::next_window_pages(nr_current_pages);
                self.trigger_page_idx = start_page_idx;
            }
        }

        Some(self.start_page_idx..self.start_page_idx.saturating_add(self.nr_window_pages))
    }

    /// Records a completed read.
    ///
    /// Returns the page-index range to prefetch.
    ///
    /// Reads are treated as sequential when their first page is the same as or
    /// immediately after the page containing the end of the previous read.
    fn on_read(&mut self, offset: usize, read_len: usize) -> Option<Range<usize>> {
        let read_end = offset + read_len;
        let read_start_idx = offset / PAGE_SIZE;
        let read_end_idx = read_end.div_ceil(PAGE_SIZE);

        if !self.is_sequential_read(read_start_idx) {
            self.prev_pos = Some(read_end);
            self.clear_window();
            return None;
        }

        self.prev_pos = Some(read_end);

        if self.nr_window_pages == 0 {
            return self.start_initial_window(read_start_idx, read_end_idx);
        }

        self.advance_read_window(read_end_idx)
    }

    /// Returns whether the read continues the previous page-level stream.
    fn is_sequential_read(&self, read_start_idx: usize) -> bool {
        let Some(prev_pos) = self.prev_pos else {
            return read_start_idx == 0;
        };

        let prev_page_idx = prev_pos / PAGE_SIZE;
        read_start_idx == prev_page_idx || read_start_idx == prev_page_idx + 1
    }

    /// Starts the initial window for a sequential stream.
    fn start_initial_window(
        &mut self,
        read_start_idx: usize,
        read_end_idx: usize,
    ) -> Option<Range<usize>> {
        let nr_window_pages = Self::initial_window_pages(read_end_idx - read_start_idx);
        let readahead_start_idx = read_end_idx;
        let window_end_idx = read_start_idx + nr_window_pages;

        self.start_page_idx = read_start_idx;
        self.nr_window_pages = nr_window_pages;
        self.trigger_page_idx = readahead_start_idx;

        if readahead_start_idx >= window_end_idx {
            return None;
        }

        Some(readahead_start_idx..window_end_idx)
    }

    /// Advances the window after the read reaches the trigger page.
    fn advance_read_window(&mut self, read_end_idx: usize) -> Option<Range<usize>> {
        let window_end_idx = self.start_page_idx + self.nr_window_pages;
        if read_end_idx <= self.trigger_page_idx {
            return None;
        }

        let new_start_idx = window_end_idx.max(read_end_idx);
        let nr_new_window_pages = Self::next_window_pages(self.nr_window_pages);
        let new_end_idx = new_start_idx + nr_new_window_pages;

        self.start_page_idx = new_start_idx;
        self.nr_window_pages = nr_new_window_pages;
        self.trigger_page_idx = new_start_idx;

        Some(new_start_idx..new_end_idx)
    }

    /// Returns the initial total window size in pages.
    ///
    /// Reference:
    /// <https://elixir.bootlin.com/linux/v7.1-rc7/source/mm/readahead.c#L376>.
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
    /// <https://elixir.bootlin.com/linux/v7.1-rc7/source/mm/readahead.c#L394>.
    fn next_window_pages(nr_current_pages: usize) -> usize {
        if nr_current_pages < MAX_READAHEAD_PAGES / 16 {
            nr_current_pages * 4
        } else if nr_current_pages <= MAX_READAHEAD_PAGES / 2 {
            nr_current_pages * 2
        } else {
            MAX_READAHEAD_PAGES
        }
    }

    /// Clears the active readahead window.
    fn clear_window(&mut self) {
        self.start_page_idx = 0;
        self.nr_window_pages = 0;
        self.trigger_page_idx = 0;
    }
}
