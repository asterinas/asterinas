// SPDX-License-Identifier: MPL-2.0

//! Output-page ownership shared by independently completed pcluster fragments.

use core::ops::Range;

use io_util::{IoError, batch::IoCompletion};
use ostd::{
    mm::{VmReader, io::util::HasVmReaderWriter},
    sync::WaitQueue,
};

use crate::{
    prelude::{Arc, Mutex},
    thread::work_queue::{self, WorkPriority},
    vm::page_cache::LockedCachePage,
};

/// One initialization attempt retaining the output page until all fragments finish.
pub(in super::super) struct OutputPageTask {
    state: Mutex<OutputPageState>, // Shared page initialization state.
    completed: WaitQueue,          // Waiters for this page's completion.
}

struct OutputPageState {
    page: Option<LockedCachePage>, // Owns the page lock until completion.
    remain_pcluster: usize,        // Unfinished fragments, including EOF zero fill.
    first_error: Option<IoError>,  // First failure reported by a fragment.
}

/// A single, non-cloneable fragment with one output-page settlement responsibility.
pub(in super::super) struct PclusterOutputPageRange {
    page: Arc<OutputPageTask>, // Shared completion task for the output page.
    // Range in decoded data or SHIFTED input; unused for zero fill.
    source_range_bytes: Range<usize>,
    // Byte range within the output page.
    page_range_bytes: Range<usize>,
    is_zero_fill: bool, // True for an EOF zero-fill fragment.
}

impl IoCompletion for OutputPageTask {
    fn wait(&self) -> Result<(), IoError> {
        self.completed.wait_until(|| {
            let state = self.state.lock();
            (state.remain_pcluster == 0).then(|| state.first_error.map_or(Ok(()), Err))
        })
    }
}

impl OutputPageTask {
    /// Retains an Uninit page's initialization lock for its fixed, nonempty fragment set.
    ///
    /// The caller supplies a nonzero count of all data fragments and any nonempty EOF zero tail before
    /// publishing the task. Each record must subsequently be finished once.
    pub(in super::super) fn new(page: LockedCachePage, necessary_fragments: usize) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(OutputPageState {
                page: Some(page),
                remain_pcluster: necessary_fragments,
                first_error: None,
            }),
            completed: WaitQueue::new(),
        })
    }

    fn copy_range(&self, page_range_bytes: Range<usize>, bytes: &[u8]) {
        let state = self.state.lock();
        let page = state.page.as_ref().unwrap();
        let mut writer = page.writer();
        writer
            .skip(page_range_bytes.start)
            .limit(page_range_bytes.len());
        writer.write(&mut VmReader::from(bytes));
    }

    fn zero_range(&self, page_range_bytes: Range<usize>) {
        let state = self.state.lock();
        let page = state.page.as_ref().unwrap();
        let mut writer = page.writer();
        writer
            .skip(page_range_bytes.start)
            .limit(page_range_bytes.len());
        writer.fill_zeros(page_range_bytes.len());
    }

    fn finish_one(&self, result: Result<(), IoError>) {
        {
            let mut state = self.state.lock();
            if let Err(error) = result {
                state.first_error.get_or_insert(error);
            }
            state.remain_pcluster -= 1;
            if state.remain_pcluster != 0 {
                return;
            }

            let page = state.page.take().unwrap();
            if state.first_error.is_none() {
                page.set_up_to_date();
            }
            // Observing remain_pcluster == 0 under the state lock guarantees that
            // the real page initialization lock has already been released.
            drop(page);
        }
        self.completed.wake_all();
    }
}

impl PclusterOutputPageRange {
    pub(super) fn source_range_bytes(&self) -> Range<usize> {
        self.source_range_bytes.clone()
    }

    /// Creates a data fragment from already checked, equally sized byte ranges.
    ///
    /// The target lies within the page; the source lies within the full pcluster
    /// output (or SHIFTED input). All records together cover the necessary page
    /// bytes without overlap and match the task's initial fragment count.
    pub(in super::super) fn new_data(
        page: Arc<OutputPageTask>,
        source_range_bytes: Range<usize>,
        page_range_bytes: Range<usize>,
    ) -> Self {
        Self {
            page,
            source_range_bytes,
            page_range_bytes,
            is_zero_fill: false,
        }
    }

    /// Creates the already checked, nonempty EOF tail within the target page.
    pub(in super::super) fn new_zero(
        page: Arc<OutputPageTask>,
        page_range_bytes: Range<usize>,
    ) -> Self {
        Self {
            page,
            source_range_bytes: 0..0,
            page_range_bytes,
            is_zero_fill: true,
        }
    }

    /// Writes this fragment without consuming its completion responsibility.
    pub(super) fn write(&self, source: &[u8]) {
        if self.is_zero_fill {
            self.page.zero_range(self.page_range_bytes.clone());
        } else {
            self.page.copy_range(
                self.page_range_bytes.clone(),
                &source[self.source_range_bytes.clone()],
            );
        }
    }

    /// Consumes this fragment's one completion responsibility.
    pub(in super::super) fn finish(self, result: Result<(), IoError>) {
        self.page.finish_one(result);
    }

    /// Independently schedules a zero fragment constructed by `new_zero`.
    pub(in super::super) fn post_zero(self) {
        // The public queue accepts Fn; the slot transfers this non-Clone record
        // exactly once without retaining its lock during page access.
        let slot = Mutex::new(Some(self));
        work_queue::submit_work_func(
            move || {
                let page_fragment = { slot.lock().take() };
                if let Some(page_fragment) = page_fragment {
                    page_fragment.write(&[]);
                    page_fragment.finish(Ok(()));
                }
            },
            WorkPriority::Normal,
        );
    }
}
