// SPDX-License-Identifier: MPL-2.0

//! Test-only helpers for deterministic page-cache I/O scheduling.

use alloc::{format, vec};
use core::fmt;

use aster_block::{
    BlockDevice, BlockDeviceMeta, SECTOR_SIZE,
    bio::{Bio, BioCompleteFn, BioEnqueueError, BioSegment, BioStatus, BioType, SubmittedBio},
    id::Sid,
};
use device_id::DeviceId;
use io_util::batch::IoBatch;
use ostd::mm::VmIo;

use crate::{prelude::*, thread::Thread, vm::page_cache::BlockAsPageCacheBackend};

/// Maximum number of cooperative yields before a test wait times out.
const MAX_WAIT_YIELDS: usize = 1_000_000;

/// Distinguishes read BIOs from write BIOs in the mock backend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum IoKind {
    Read,
    Write,
}

impl IoKind {
    fn from_bio_type(type_: BioType) -> Self {
        match type_ {
            BioType::Read => Self::Read,
            BioType::Write => Self::Write,
            _ => unimplemented!("mock page-cache backend only supports read/write BIOs"),
        }
    }

    fn bio_type(self) -> BioType {
        match self {
            Self::Read => BioType::Read,
            Self::Write => BioType::Write,
        }
    }
}

/// Selects whether a mock BIO completes inline or waits for the test.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum IoCompletion {
    Immediate,
    Deferred,
}

#[derive(Debug)]
struct DeferredBio {
    page_idx: usize,
    bio: SubmittedBio,
}

#[derive(Debug)]
struct MockBackendState {
    persisted_pages: Vec<Vec<u8>>,
    read_counts: Vec<usize>,
    write_counts: Vec<usize>,
    read_submit_failures: BTreeSet<usize>,
    write_submit_failures: BTreeSet<usize>,
    read_completion: IoCompletion,
    write_completion: IoCompletion,
    deferred_reads: VecDeque<DeferredBio>,
    deferred_writes: VecDeque<DeferredBio>,
}

impl MockBackendState {
    fn new(num_pages: usize) -> Self {
        Self {
            persisted_pages: vec![vec![0; PAGE_SIZE]; num_pages],
            read_counts: vec![0; num_pages],
            write_counts: vec![0; num_pages],
            read_submit_failures: BTreeSet::new(),
            write_submit_failures: BTreeSet::new(),
            read_completion: IoCompletion::Immediate,
            write_completion: IoCompletion::Immediate,
            deferred_reads: VecDeque::new(),
            deferred_writes: VecDeque::new(),
        }
    }

    fn completion_mode(&self, kind: IoKind) -> IoCompletion {
        match kind {
            IoKind::Read => self.read_completion,
            IoKind::Write => self.write_completion,
        }
    }

    fn set_completion(&mut self, kind: IoKind, completion: IoCompletion) {
        match kind {
            IoKind::Read => self.read_completion = completion,
            IoKind::Write => self.write_completion = completion,
        }
    }

    fn should_fail_submit(&self, kind: IoKind, page_idx: usize) -> bool {
        let failures = match kind {
            IoKind::Read => &self.read_submit_failures,
            IoKind::Write => &self.write_submit_failures,
        };
        failures.contains(&page_idx)
    }

    fn record_submission(&mut self, kind: IoKind, page_idx: usize) {
        match kind {
            IoKind::Read => self.read_counts[page_idx] += 1,
            IoKind::Write => self.write_counts[page_idx] += 1,
        }
    }

    fn deferred_bios(&self, kind: IoKind) -> &VecDeque<DeferredBio> {
        match kind {
            IoKind::Read => &self.deferred_reads,
            IoKind::Write => &self.deferred_writes,
        }
    }

    fn deferred_bios_mut(&mut self, kind: IoKind) -> &mut VecDeque<DeferredBio> {
        match kind {
            IoKind::Read => &mut self.deferred_reads,
            IoKind::Write => &mut self.deferred_writes,
        }
    }

    fn submission_count(&self, kind: IoKind, page_idx: usize) -> usize {
        match kind {
            IoKind::Read => self.read_counts[page_idx],
            IoKind::Write => self.write_counts[page_idx],
        }
    }
}

/// A mock backend that lets tests observe and manually drive page-cache I/O.
///
/// It models a page-addressable backend directly, so tests can coordinate
/// backend completion order without an extra fake-device wrapper or timer-based
/// sleeps.
pub(super) struct MockPageCacheBackend {
    state: SpinLock<MockBackendState>,
    num_pages: usize,
}

impl MockPageCacheBackend {
    /// Creates a backend with `num_pages` zero-filled persisted pages.
    pub(super) fn new(num_pages: usize) -> Arc<Self> {
        Arc::new(Self {
            state: SpinLock::new(MockBackendState::new(num_pages)),
            num_pages,
        })
    }

    /// Sets how future BIOs of `kind` complete.
    pub(super) fn set_completion(&self, kind: IoKind, completion: IoCompletion) {
        self.state.lock().set_completion(kind, completion);
    }

    /// Waits until the backend has queued `expected_count` deferred BIOs.
    ///
    /// Tests use this to freeze the backend at the "BIO submitted but not yet
    /// completed" point before releasing the next scheduling step.
    pub(super) fn wait_for_deferred_bios(&self, kind: IoKind, expected_count: usize) {
        let failure_message =
            format!("timed out waiting for {expected_count} deferred {kind:?} BIOs");
        wait_until_with_failure_message(
            || self.state.lock().deferred_bios(kind).len() >= expected_count,
            &failure_message,
        );
    }

    /// Completes the next deferred BIO of `kind`.
    pub(super) fn complete_next_deferred_bio(&self, kind: IoKind, success: bool) -> bool {
        let deferred_bio = self.state.lock().deferred_bios_mut(kind).pop_front();
        let Some(deferred_bio) = deferred_bio else {
            return false;
        };

        self.complete_bio(kind, deferred_bio.page_idx, deferred_bio.bio, success);
        true
    }

    /// Returns how many read BIOs were submitted for `page_idx`.
    pub(super) fn read_count(&self, page_idx: usize) -> usize {
        self.state.lock().submission_count(IoKind::Read, page_idx)
    }

    /// Returns how many write BIOs were submitted for `page_idx`.
    pub(super) fn write_count(&self, page_idx: usize) -> usize {
        self.state.lock().submission_count(IoKind::Write, page_idx)
    }

    /// Returns the bytes currently persisted for `page_idx`.
    pub(super) fn persisted_page_bytes(&self, page_idx: usize) -> Vec<u8> {
        self.state.lock().persisted_pages[page_idx].clone()
    }

    /// Preloads the persisted bytes for `page_idx`.
    pub(super) fn set_persisted_page_bytes(&self, page_idx: usize, data: &[u8]) {
        assert_eq!(data.len(), PAGE_SIZE);
        self.state.lock().persisted_pages[page_idx].copy_from_slice(data);
    }

    fn maybe_fail_submission(&self, kind: IoKind, page_idx: usize) -> Result<()> {
        let mut state = self.state.lock();
        if state.should_fail_submit(kind, page_idx) {
            state.record_submission(kind, page_idx);
            return_errno!(Errno::EIO);
        }

        Ok(())
    }

    fn submit_bio(
        &self,
        kind: IoKind,
        page_idx: usize,
        bio_segment: BioSegment,
        complete_fn: BioCompleteFn,
        io_batch: &mut IoBatch,
    ) -> Result<()> {
        self.maybe_fail_submission(kind, page_idx)?;

        let bio = Bio::new(
            kind.bio_type(),
            Sid::from_offset(page_idx * PAGE_SIZE),
            vec![bio_segment],
            Some(complete_fn),
        );
        bio.submit(self, io_batch)
            .map_err(ostd::Error::from)
            .map_err(Error::from)
    }

    fn complete_bio(&self, kind: IoKind, page_idx: usize, bio: SubmittedBio, success: bool) {
        let status = {
            let mut state = self.state.lock();
            if success {
                let segment = &bio.segments()[0];
                match kind {
                    IoKind::Read => segment
                        .inner_dma()
                        .write_bytes(0, &state.persisted_pages[page_idx])
                        .unwrap(),
                    IoKind::Write => {
                        let mut persisted_page = vec![0; PAGE_SIZE];
                        segment
                            .inner_dma()
                            .read_bytes(0, &mut persisted_page)
                            .unwrap();
                        state.persisted_pages[page_idx] = persisted_page;
                    }
                }
                BioStatus::Complete
            } else {
                BioStatus::IoError
            }
        };

        bio.complete(status);
    }
}

/// Waits until a test predicate becomes true.
pub(super) fn wait_until(mut condition: impl FnMut() -> bool) {
    wait_until_with_failure_message(&mut condition, "timed out waiting for test condition");
}

fn wait_until_with_failure_message(mut condition: impl FnMut() -> bool, failure_message: &str) {
    for _ in 0..MAX_WAIT_YIELDS {
        if condition() {
            return;
        }

        Thread::yield_now();
    }

    panic!("{failure_message}");
}

impl Debug for MockPageCacheBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MockPageCacheBackend")
            .field("num_pages", &self.num_pages)
            .finish_non_exhaustive()
    }
}

impl BlockDevice for MockPageCacheBackend {
    fn enqueue(&self, bio: SubmittedBio) -> Result<(), BioEnqueueError> {
        let page_idx = bio.sid_range().start.to_offset() / PAGE_SIZE;
        let kind = IoKind::from_bio_type(bio.type_());

        let completion = {
            let mut state = self.state.lock();
            state.record_submission(kind, page_idx);
            let completion = state.completion_mode(kind);
            if matches!(completion, IoCompletion::Deferred) {
                state
                    .deferred_bios_mut(kind)
                    .push_back(DeferredBio { page_idx, bio });
                return Ok(());
            }
            completion
        };

        debug_assert_eq!(completion, IoCompletion::Immediate);
        self.complete_bio(kind, page_idx, bio, true);
        Ok(())
    }

    fn metadata(&self) -> BlockDeviceMeta {
        BlockDeviceMeta {
            max_nr_segments_per_bio: 1,
            nr_sectors: self.num_pages * (PAGE_SIZE / SECTOR_SIZE),
        }
    }

    fn name(&self) -> &str {
        "mock-page-cache"
    }

    fn id(&self) -> DeviceId {
        DeviceId::null()
    }
}

impl BlockAsPageCacheBackend for MockPageCacheBackend {
    fn submit_read_bio(
        &self,
        page_idx: usize,
        bio_segment: BioSegment,
        complete_fn: BioCompleteFn,
        io_batch: &mut IoBatch,
    ) -> Result<()> {
        if page_idx >= self.num_pages {
            return_errno!(Errno::EINVAL);
        }
        self.submit_bio(IoKind::Read, page_idx, bio_segment, complete_fn, io_batch)
    }

    fn submit_write_bio(
        &self,
        page_idx: usize,
        bio_segment: BioSegment,
        complete_fn: BioCompleteFn,
        io_batch: &mut IoBatch,
    ) -> Result<()> {
        if page_idx >= self.num_pages {
            return_errno!(Errno::EINVAL);
        }
        self.submit_bio(IoKind::Write, page_idx, bio_segment, complete_fn, io_batch)
    }
}
