// SPDX-License-Identifier: MPL-2.0

//! Instance-local pcluster objects and their independent raw input cache.

use core::fmt;

use aster_block::BlockDevice;
use hashbrown::HashMap;
use io_util::IoError;
use ostd::mm::{PAGE_SIZE, VmWriter};

use super::{
    super::{decompressor, io::RawPageBackend, meta::MetadataCache},
    batch_wait::BatchWait,
    mapping::{PclusterKey, PclusterMapping, ProcessingKind},
    output::PclusterOutputPageRange,
};
use crate::{
    prelude::{Arc, Box, Errno, Error, Mutex, Result, RwMutex, Vec},
    thread::work_queue::work_item::WorkItem,
    vm::page_cache::{PageCache, Readahead, VmoCommitError, VmoMapMode},
};

/// Strongly retained pcluster identities sharing one ordinary input cache.
pub(in super::super) struct PclusterManager {
    objects: Mutex<HashMap<PclusterKey, Arc<Pcluster>>>,
    block_input: Arc<PclusterInputCache>,
    batch_wait: Arc<BatchWait>,
}

/// Ordinary pcluster input pages, separate from the metadata VMO.
#[derive(Debug)]
struct PclusterInputCache {
    // Fields drop in declaration order; the VMO must retire before its backend.
    cache: PageCache,
    // Never nest this sleeping lock with a pcluster or output-task lock.
    access: RwMutex<()>,
    // The VMO weakly references its backend, so this owner keeps it available.
    _backend: Arc<RawPageBackend>,
}

enum PclusterInput {
    Block(Arc<PclusterInputCache>),
    Inline(Arc<MetadataCache>),
}

pub(in super::super) struct Pcluster {
    mapping: PclusterMapping,
    input: PclusterInput,
    state: Mutex<PclusterState>,
}

struct PclusterState {
    reserved: bool,
    output_ranges: Vec<PclusterOutputPageRange>,
}

// Bounds one ordinary input buffer and optional chain growth.
pub(in super::super) const MAX_INPUT_RANGE_PAGES: usize = 32;
// Bound optional lookahead independently of the real output window.
pub(in super::super) const MAX_NEIGHBOR_INPUTS: usize = 16;

/// One preallocated slot owning an accepted round and its eventual completion.
pub(in super::super) struct PclusterJob {
    pcluster: Arc<Pcluster>,
    input_was_ready: bool,
    completed: Option<CompletedRound>,
}

struct CompletedRound {
    outputs: Vec<PclusterOutputPageRange>,
    result: core::result::Result<(), IoError>,
    evict_input: bool,
}

impl PclusterJob {
    pub(in super::super) fn new(pcluster: Arc<Pcluster>) -> Self {
        Self {
            pcluster,
            input_was_ready: false,
            completed: None,
        }
    }

    /// Schedules required rounds and optional input prefetches in one work item.
    fn post_batch(
        mut jobs: Vec<Self>,
        anchor: Option<Arc<Pcluster>>,
        neighbors: Vec<Arc<Pcluster>>,
        wait: Arc<BatchWait>,
    ) {
        if jobs.is_empty() && neighbors.is_empty() {
            return;
        }
        jobs.sort_unstable_by_key(|job| {
            (
                job.pcluster.mapping.is_inline_input(),
                job.pcluster.mapping.input_start_bytes(),
            )
        });
        // The public work queue accepts Fn, so transfer the batch exactly once.
        let slot = Mutex::new(Some((jobs, anchor, neighbors)));
        let worker_wait = wait.clone();
        let item = WorkItem::new(Box::new(move || {
            let batch = { slot.lock().take() };
            if let Some((jobs, anchor, neighbors)) = batch {
                worker_wait.on_start();
                Self::run_batch(jobs, anchor, neighbors);
                worker_wait.wait_after_completion();
            }
        }));
        wait.submit_fresh(item);
    }

    fn run_batch(
        mut jobs: Vec<Self>,
        anchor: Option<Arc<Pcluster>>,
        neighbors: Vec<Arc<Pcluster>>,
    ) {
        let ordinary_count = jobs.partition_point(|job| !job.pcluster.mapping.is_inline_input());
        let buffer_len_bytes = ordinary_count.min(MAX_INPUT_RANGE_PAGES) * PAGE_SIZE;
        let mut input = Vec::new();
        let mut decoded = Vec::new();
        // An input without a job warms the cache without owning a decode round.
        let mut entries: Vec<(Arc<Pcluster>, Option<usize>)> = Vec::new();
        let allocation_failed = input.try_reserve_exact(buffer_len_bytes).is_err()
            || entries.try_reserve_exact(ordinary_count).is_err();
        if allocation_failed {
            for job in &mut jobs[..ordinary_count] {
                job.complete(Err(IoError::Failed), &mut decoded);
            }
        } else {
            input.resize(buffer_len_bytes, 0);
            entries.extend(
                jobs[..ordinary_count]
                    .iter()
                    .enumerate()
                    .map(|(index, job)| (job.pcluster.clone(), Some(index))),
            );
            let required_entries = entries.len();
            let mut predecessor = anchor.as_ref();
            for candidate in neighbors.iter().take(MAX_NEIGHBOR_INPUTS) {
                let Some(previous) = predecessor else { break };
                // Preparation and worker execution can race with another read.
                // Stop at the first ready input so lookahead cannot cross it.
                if entries.len() >= MAX_INPUT_RANGE_PAGES
                    || !previous.input_needs_io()
                    || !previous.input_is_adjacent_to(&candidate.mapping)
                    || !candidate.input_needs_io()
                    || entries
                        .iter()
                        .any(|(pcluster, _)| Arc::ptr_eq(pcluster, candidate))
                {
                    break;
                }
                let append_failed = entries.try_reserve(1).is_err();
                if append_failed {
                    break;
                }
                entries.push((candidate.clone(), None));
                predecessor = Some(candidate);
            }
            let expanded_len_bytes = entries.len().min(MAX_INPUT_RANGE_PAGES) * PAGE_SIZE;
            if expanded_len_bytes > input.len() {
                // Grow once after collection; optional allocation failure must
                // leave the required inputs' allocation result unchanged.
                let growth_failed = input
                    .try_reserve_exact(expanded_len_bytes - input.len())
                    .is_err();
                if growth_failed {
                    entries.truncate(required_entries);
                } else {
                    input.resize(expanded_len_bytes, 0);
                }
            }
            entries.sort_unstable_by_key(|(pcluster, _)| pcluster.mapping.input_start_bytes());
            let mut start = 0;
            while start < entries.len() {
                let mut end = start + 1;
                while end < entries.len()
                    && end - start < MAX_INPUT_RANGE_PAGES
                    && entries[end].0.mapping.input_start_bytes()
                        == entries[end - 1].0.mapping.input_start_bytes() + PAGE_SIZE
                {
                    end += 1;
                }
                Self::read_range(
                    &entries[start..end],
                    &mut jobs,
                    &mut input[..(end - start) * PAGE_SIZE],
                    &mut decoded,
                );
                start = end;
            }
        }
        for job in &mut jobs[ordinary_count..] {
            // Inline input remains entirely within the metadata cache's policy.
            let input = job.pcluster.read_inline_input();
            job.complete(input.as_deref().map_err(|error| *error), &mut decoded);
        }
        Self::finish_batch(jobs);
    }

    fn read_range(
        entries: &[(Arc<Pcluster>, Option<usize>)],
        jobs: &mut [Self],
        bytes: &mut [u8],
        decoded: &mut Vec<u8>,
    ) {
        let PclusterInput::Block(input) = &entries[0].0.input else {
            unreachable!()
        };
        let input = input.clone();
        let mut results = [Ok(()); MAX_INPUT_RANGE_PAGES];
        {
            let _access = input.access.read();
            for (index, (pcluster, job_index)) in entries.iter().enumerate() {
                // Finish the nonblocking probe before the potentially sleeping read.
                match input
                    .cache
                    .as_vmo()
                    .try_commit_page(pcluster.mapping.input_start_bytes(), VmoMapMode::SharedRead)
                {
                    Ok(_) => {
                        if let Some(job_index) = job_index {
                            jobs[*job_index].input_was_ready = true;
                        }
                    }
                    Err(VmoCommitError::NeedIo { .. }) => {}
                    Err(VmoCommitError::Err(error)) => {
                        results[index] = Err(completion_error(&error))
                    }
                }
            }
            let mut start = 0;
            while start < entries.len() {
                if results[start].is_err() {
                    start += 1;
                    continue;
                }
                let end = (start + 1..entries.len())
                    .find(|index| results[*index].is_err())
                    .unwrap_or(entries.len());
                let len_bytes = (end - start) * PAGE_SIZE;
                let mut writer =
                    VmWriter::from(&mut bytes[start * PAGE_SIZE..end * PAGE_SIZE]).to_fallible();
                let readahead = Readahead::default();
                let offset_bytes = entries[start].0.mapping.input_start_bytes();
                // A fresh policy at page zero expands to the default window.
                // Only the explicitly mapped input pages belong to this batch.
                let policy = (offset_bytes != 0).then_some(&readahead);
                let result = input.cache.as_vmo().read(offset_bytes, &mut writer, policy);
                // A failed read still owns its complete copied prefix. Only the
                // first unfinished page fails; retry the suffix with a fresh window.
                let complete_pages = (len_bytes - writer.avail()) / PAGE_SIZE;
                start += complete_pages;
                if start < end {
                    results[start] = Err(result
                        .as_ref()
                        .err()
                        .map_or(IoError::Failed, completion_error));
                    start += 1;
                }
            }
        }
        for (index, (_, job_index)) in entries.iter().enumerate() {
            if let Some(job_index) = job_index {
                jobs[*job_index].complete(
                    results[index].map(|()| &bytes[index * PAGE_SIZE..(index + 1) * PAGE_SIZE]),
                    decoded,
                );
            }
        }
    }

    fn complete(&mut self, input: core::result::Result<&[u8], IoError>, decoded: &mut Vec<u8>) {
        self.completed = Some(self.pcluster.prepare_completed_round(
            input,
            self.input_was_ready,
            decoded,
        ));
    }

    fn finish_batch(mut jobs: Vec<Self>) {
        // The batch's output fragments retain their page initialization duties
        // until every cleanup attempt finishes, including when another fragment failed.
        if let Some(input) = jobs.iter().find_map(|job| {
            if !job.completed.as_ref().unwrap().evict_input {
                return None;
            }
            match &job.pcluster.input {
                PclusterInput::Block(input) => Some(input.clone()),
                PclusterInput::Inline(_) => None,
            }
        }) {
            let _access = input.access.write();
            for job in &jobs {
                if !job.completed.as_ref().unwrap().evict_input {
                    continue;
                }
                let offset_bytes = job.pcluster.mapping.input_start_bytes();
                if let Err(error) = input
                    .cache
                    .evict_range(offset_bytes..offset_bytes + PAGE_SIZE)
                {
                    ostd::error!(
                        "failed to evict erofs input page at {}: {:?}",
                        offset_bytes,
                        error
                    );
                }
            }
        }
        for job in &mut jobs {
            let completed = job.completed.take().unwrap();
            for fragment in completed.outputs {
                fragment.finish(completed.result);
            }
        }
    }
}

impl fmt::Debug for PclusterManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PclusterManager")
            .field("block_input", &self.block_input)
            .finish_non_exhaustive()
    }
}

impl PclusterManager {
    /// Schedules required rounds and optional input prefetches with the shared handoff.
    pub(in super::super) fn post_batch(
        &self,
        jobs: Vec<PclusterJob>,
        anchor: Option<Arc<Pcluster>>,
        neighbors: Vec<Arc<Pcluster>>,
    ) {
        PclusterJob::post_batch(jobs, anchor, neighbors, self.batch_wait.clone());
    }

    /// Finds an object already retained by this filesystem instance.
    pub(in super::super) fn find(&self, key: &PclusterKey) -> Option<Arc<Pcluster>> {
        self.objects.lock().get(key).cloned()
    }

    /// Publishes one canonical object per physical input, rejecting conflicting descriptions.
    /// The checked mapping and metadata cache belong to this filesystem instance.
    pub(in super::super) fn find_or_create(
        &self,
        mapping: &PclusterMapping,
        metadata_cache: &Arc<MetadataCache>,
    ) -> Result<Arc<Pcluster>> {
        let key = mapping.key();
        if let Some(pcluster_object) = self.find(&key) {
            return compatible_object(pcluster_object, mapping);
        }

        // Candidate allocation and owner acquisition never nest the table and K locks.
        let input = match key {
            PclusterKey::Block(_) => PclusterInput::Block(self.block_input.clone()),
            PclusterKey::Inline(_) => PclusterInput::Inline(metadata_cache.clone()),
        };
        let candidate = Arc::new(Pcluster {
            mapping: mapping.clone(),
            input,
            state: Mutex::new(PclusterState {
                reserved: false,
                output_ranges: Vec::new(),
            }),
        });
        let mut objects = self.objects.lock();
        if let Some(pcluster_object) = objects.get(&key) {
            return compatible_object(pcluster_object.clone(), mapping);
        }
        objects
            .try_reserve(1)
            .map_err(|_| Error::new(Errno::ENOMEM))?;
        objects.insert(key, candidate.clone());
        Ok(candidate)
    }

    /// Creates an empty object table and an independent, demand-paged input VMO.
    pub(in super::super) fn new(device: Arc<dyn BlockDevice>) -> ostd::Result<Self> {
        let block_input = Arc::new(PclusterInputCache::new(device)?);
        Ok(Self {
            objects: Mutex::new(HashMap::new()),
            block_input,
            batch_wait: Arc::new(BatchWait::new()),
        })
    }
}

impl PclusterInputCache {
    fn new(device: Arc<dyn BlockDevice>) -> ostd::Result<Self> {
        let backend = Arc::new(RawPageBackend::new(device)?);
        let cache = backend.create_cache()?;
        Ok(Self {
            cache,
            access: RwMutex::new(()),
            _backend: backend,
        })
    }
}

impl Pcluster {
    /// Reports an ordinary input that is not directly readable, including in-flight input.
    pub(in super::super) fn input_needs_io(&self) -> bool {
        let PclusterInput::Block(input) = &self.input else {
            return false;
        };
        let _access = input.access.read();
        matches!(
            input
                .cache
                .as_vmo()
                .try_commit_page(self.mapping.input_start_bytes(), VmoMapMode::SharedRead),
            Err(VmoCommitError::NeedIo { .. })
        )
    }

    /// Reports the next physical page in the same ordinary input run.
    pub(in super::super) fn input_is_adjacent_to(&self, next: &PclusterMapping) -> bool {
        !self.mapping.is_inline_input()
            && !next.is_inline_input()
            && self.mapping.input_start_bytes().checked_add(PAGE_SIZE)
                == Some(next.input_start_bytes())
    }

    /// Reports whether this group's source union leaves any output bytes uncovered.
    fn has_partial_output(&self, outputs: &mut [PclusterOutputPageRange]) -> bool {
        outputs.sort_unstable_by_key(|output| output.source_range_bytes().start);
        let mut covered_end_bytes = 0;
        for output in outputs {
            let range_bytes = output.source_range_bytes();
            if range_bytes.start > covered_end_bytes {
                return true;
            }
            covered_end_bytes = covered_end_bytes.max(range_bytes.end);
        }
        covered_end_bytes < self.mapping.file_range_bytes().len()
    }

    /// Accepts the whole group or returns it untouched, including its settlement duties.
    /// The caller supplies a nonempty complete group with already checked fragment ranges.
    /// `Ok(true)` transfers the new round's execution responsibility to the caller,
    /// which must create a `PclusterJob` and eventually submit it.
    /// `Ok(false)` joins a round already owned by another job and needs no submission.
    pub(in super::super) fn insert_output_to_pcluster(
        self: &Arc<Self>,
        mut outputs: Vec<PclusterOutputPageRange>,
    ) -> core::result::Result<bool, (Error, Vec<PclusterOutputPageRange>)> {
        let should_post = {
            let mut state = self.state.lock();
            if state.output_ranges.try_reserve(outputs.len()).is_err() {
                return Err((Error::new(Errno::ENOMEM), outputs));
            }
            state.output_ranges.append(&mut outputs);
            let should_post = !state.reserved;
            state.reserved = true;
            should_post
        };
        Ok(should_post)
    }

    fn prepare_completed_round(
        &self,
        input: core::result::Result<&[u8], IoError>,
        input_was_ready: bool,
        decoded: &mut Vec<u8>,
    ) -> CompletedRound {
        let mut state = self.state.lock();
        let mut outputs: Vec<PclusterOutputPageRange> = core::mem::take(&mut state.output_ranges);
        // Lock order: K -> one output task. Never wait for a different K or a whole page.
        let retain_input_cache = match &self.input {
            PclusterInput::Block(_) => input_was_ready || self.has_partial_output(&mut outputs),
            PclusterInput::Inline(_) => true,
        };
        let result = self.process(input, &outputs, decoded);
        state.reserved = false;
        drop(state);
        // The old round no longer accesses K state after releasing this lock.
        CompletedRound {
            outputs,
            result,
            evict_input: !retain_input_cache && result.is_ok(),
        }
    }

    fn read_inline_input(&self) -> core::result::Result<Vec<u8>, IoError> {
        let PclusterInput::Inline(metadata) = &self.input else {
            unreachable!()
        };
        let input_len_bytes = self.mapping.input_len_bytes();
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(input_len_bytes)
            .map_err(|_| IoError::Failed)?;
        bytes.resize(input_len_bytes, 0);
        let mut writer = VmWriter::from(bytes.as_mut_slice()).to_fallible();
        metadata
            .read(self.mapping.input_start_bytes(), &mut writer)
            .map_err(|error| completion_error(&error))?;
        if writer.avail() != 0 {
            return Err(IoError::Failed);
        }
        Ok(bytes)
    }

    fn process(
        &self,
        input: core::result::Result<&[u8], IoError>,
        outputs: &[PclusterOutputPageRange],
        decoded: &mut Vec<u8>,
    ) -> core::result::Result<(), IoError> {
        let input = input?;
        if self.mapping.kind() == ProcessingKind::PlainShifted {
            self.copy_outputs(input, outputs);
            return Ok(());
        }

        let prepared_input = decompressor::prepare_input(input)?;
        let output_len_bytes = self.mapping.file_range_bytes().len();
        if output_len_bytes > decoded.capacity() {
            decoded
                .try_reserve_exact(output_len_bytes - decoded.len())
                .map_err(|_| IoError::Failed)?;
        }
        // Keep initialized storage for the batch, but expose only this extent's
        // exact output length to the decoder and its output fragments.
        if output_len_bytes > decoded.len() {
            decoded.resize(output_len_bytes, 0);
        }
        let output = &mut decoded[..output_len_bytes];
        decompressor::decompress(prepared_input, output)?;
        self.copy_outputs(output, outputs);
        Ok(())
    }

    fn copy_outputs(&self, source: &[u8], outputs: &[PclusterOutputPageRange]) {
        for page_fragment in outputs {
            page_fragment.write(source);
        }
    }
}

fn compatible_object(
    pcluster_object: Arc<Pcluster>,
    mapping: &PclusterMapping,
) -> Result<Arc<Pcluster>> {
    if !pcluster_object.mapping.is_compatible_with(mapping) {
        return Err(Error::with_message(
            Errno::EINVAL,
            "conflicting erofs pcluster mapping",
        ));
    }
    Ok(pcluster_object)
}

fn completion_error(error: &Error) -> IoError {
    if error.error() == Errno::EOPNOTSUPP {
        IoError::Unsupported
    } else {
        IoError::Failed
    }
}
