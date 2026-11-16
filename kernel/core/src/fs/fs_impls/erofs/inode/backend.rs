// SPDX-License-Identifier: MPL-2.0

//! Logical output-page submission implemented by the inode itself.

use hashbrown::HashMap;
use io_util::{IoError, batch::IoBatch};
use ostd::mm::{self, io::util::HasVmReaderWriter};

use super::{
    super::{
        compressed::{
            MAX_INPUT_RANGE_PAGES, MAX_NEIGHBOR_INPUTS, Pcluster, PclusterJob,
            mapping::{self, PclusterKey, PclusterMapping},
            output::{OutputPageTask, PclusterOutputPageRange},
        },
        fs::Erofs,
        meta::MetadataCache,
    },
    ErofsInode,
    data::{FlatPageMapping, FlatPageMappingKind},
    layout::InodeDataLayout,
};
use crate::{
    prelude::{Arc, Errno, Error, Result, Vec, vec},
    vm::page_cache::{LockedCachePage, PageCacheBackend, PageRun},
};

impl PageCacheBackend for ErofsInode {
    fn read_page_async(
        &self,
        idx: usize,
        locked_page: LockedCachePage,
        io_batch: &mut IoBatch,
    ) -> Result<()> {
        self.read_pages_async_impl(core::iter::once((idx, locked_page)), io_batch)
    }

    fn read_pages_async(&self, pages: PageRun<'_>, io_batch: &mut IoBatch) -> Result<()> {
        let start_idx = pages.start_idx();
        self.read_pages_async_impl(
            pages
                .enumerate()
                .map(|(offset, page)| (start_idx + offset, page)),
            io_batch,
        )
    }

    fn write_page_async(
        &self,
        _idx: usize,
        _locked_page: LockedCachePage,
        _io_batch: &mut IoBatch,
    ) -> Result<()> {
        Err(Error::new(Errno::EROFS))
    }
}

impl ErofsInode {
    fn read_pages_async_impl<I>(&self, mut pages: I, batch: &mut IoBatch) -> Result<()>
    where
        I: Iterator<Item = (usize, LockedCachePage)>,
    {
        let result = (|| {
            let fs = self.file_system().map_err(|_| {
                Error::with_message(Errno::EIO, "erofs filesystem is no longer available")
            })?;
            let layout = self.description().data_layout();
            if matches!(layout, InodeDataLayout::Compressed(_)) {
                return self.submit_compressed_pages(&fs, &mut pages, batch);
            }
            for (idx, page) in pages.by_ref() {
                let mapping = FlatPageMapping::for_page(layout, idx)?;
                self.submit_flat_page(mapping, page, fs.metadata_cache(), batch)?;
            }
            Ok(())
        })();
        // PageRun borrows its underlying iterator. Dropping it alone would leave
        // the unconsumed initialization locks in the caller's iterator.
        if result.is_err() {
            pages.for_each(drop);
        }
        result
    }

    /// Submits a checked flat mapping under the caller's inode lifetime.
    ///
    /// The mapping must describe the stable device read by `cache`. The caller
    /// retains the inode and filesystem until accepted I/O completes,
    /// including earlier submissions if a later submission fails.
    pub(in super::super) fn submit_flat_page(
        &self,
        mapping: FlatPageMapping,
        locked_page: LockedCachePage,
        cache: &MetadataCache,
        batch: &mut IoBatch,
    ) -> Result<()> {
        match mapping.kind() {
            FlatPageMappingKind::DataBlock { device_start_bytes } => self
                .raw_backend
                .read_page_async(device_start_bytes / mm::PAGE_SIZE, locked_page, batch),
            FlatPageMappingKind::InlineTail {
                device_start_bytes,
                source_offset_bytes,
                valid_bytes,
            } => {
                {
                    let mut writer = locked_page.writer();
                    writer.limit(valid_bytes);
                    cache.read(
                        device_start_bytes + source_offset_bytes,
                        &mut writer.to_fallible(),
                    )?;
                }
                locked_page
                    .writer()
                    .skip(valid_bytes)
                    .fill_zeros(mm::PAGE_SIZE - valid_bytes);
                locked_page.set_up_to_date();
                Ok(())
            }
            FlatPageMappingKind::Eof => Err(Error::new(Errno::EINVAL)),
        }
    }

    /// Submits distinct page indices from one inode, with at most one EOF zero tail.
    fn submit_compressed_pages<I>(&self, fs: &Erofs, pages: I, batch: &mut IoBatch) -> Result<()>
    where
        I: Iterator<Item = (usize, LockedCachePage)>,
    {
        let mut page_tasks = Vec::new();
        let mut zero_range: Option<PclusterOutputPageRange> = None;
        let mut groups = HashMap::new();
        let mut last_mapping: Option<PclusterMapping> = None;
        let mut logical_tail: Option<(usize, PclusterKey)> = None;
        for (idx, page) in pages {
            let page_task = self.prepare_compressed_page(
                fs,
                idx,
                page,
                &mut groups,
                &mut zero_range,
                &mut last_mapping,
            )?;
            // Use the full pcluster end, rather than this page's requested end.
            // The iterator need not visit output pages in logical order.
            let mapping = last_mapping.as_ref().unwrap();
            let end_bytes = mapping.file_range_bytes().end;
            if logical_tail.is_none_or(|(end, _)| end_bytes > end) {
                logical_tail = Some((end_bytes, mapping.key()));
            }
            page_tasks.push(page_task);
        }
        let mut jobs = Vec::new();
        jobs.try_reserve_exact(groups.len())
            .map_err(|_| Error::new(Errno::ENOMEM))?;
        let ordinary_count = groups
            .keys()
            .filter(|key| matches!(key, PclusterKey::Block(_)))
            .count();
        let (anchor, neighbors) = self.prepare_neighbor_inputs(fs, logical_tail, ordinary_count);
        // All required fallible preparation precedes publication and submission.
        for page_task in page_tasks {
            batch.push(page_task);
        }
        if let Some(page_fragment) = zero_range {
            page_fragment.post_zero();
        }
        let jobs = Self::submit_pcluster_groups(fs, groups, jobs);
        fs.pcluster().post_batch(jobs, anchor, neighbors);
        Ok(())
    }

    /// Collects optional input neighbors without publishing output work.
    fn prepare_neighbor_inputs(
        &self,
        fs: &Erofs,
        logical_tail: Option<(usize, PclusterKey)>,
        ordinary_count: usize,
    ) -> (Option<Arc<Pcluster>>, Vec<Arc<Pcluster>>) {
        let mut neighbors = Vec::new();
        let Some((mut end_bytes, key)) = logical_tail else {
            return (None, neighbors);
        };
        if ordinary_count >= MAX_INPUT_RANGE_PAGES {
            return (None, neighbors);
        }
        let tail = fs.pcluster().find(&key).unwrap();
        let mut predecessor = tail.clone();
        let limit = MAX_NEIGHBOR_INPUTS.min(MAX_INPUT_RANGE_PAGES - ordinary_count);
        for _ in 0..limit {
            // Only extend across output pages shared by adjacent pclusters.
            if end_bytes >= self.description().size_bytes() as usize
                || end_bytes % mm::PAGE_SIZE == 0
                || !predecessor.input_needs_io()
            {
                break;
            }
            let Ok(mapping) =
                mapping::map_extent(self.description(), fs.metadata_cache(), end_bytes)
            else {
                break;
            };
            // Optional inputs extend this physical run, never a separate
            // range or metadata input. Preparation failure keeps real work.
            if !predecessor.input_is_adjacent_to(&mapping) {
                break;
            }
            let Ok(candidate) = fs.pcluster().find_or_create(&mapping, fs.metadata_cache()) else {
                break;
            };
            if !candidate.input_needs_io() || neighbors.try_reserve(1).is_err() {
                break;
            }
            end_bytes = mapping.file_range_bytes().end;
            neighbors.push(candidate.clone());
            predecessor = candidate;
        }
        let anchor = (!neighbors.is_empty()).then_some(tail);
        (anchor, neighbors)
    }

    /// Prepares one page's fixed fragment set without publishing work.
    ///
    /// On error, the caller must discard the whole batch's private tasks and ranges.
    fn prepare_compressed_page(
        &self,
        fs: &Erofs,
        idx: usize,
        page: LockedCachePage,
        groups: &mut HashMap<PclusterKey, Vec<PclusterOutputPageRange>>,
        zero_range: &mut Option<PclusterOutputPageRange>,
        last_mapping: &mut Option<PclusterMapping>,
    ) -> Result<Arc<OutputPageTask>> {
        let file_size_bytes = self.description().size_bytes() as usize;
        let page_start_bytes = idx
            .checked_mul(mm::PAGE_SIZE)
            .ok_or_else(|| Error::new(Errno::EOVERFLOW))?;
        if page_start_bytes >= file_size_bytes {
            return Err(Error::new(Errno::EINVAL));
        }
        let valid_len_bytes = mm::PAGE_SIZE.min(file_size_bytes - page_start_bytes);
        let page_valid_end_bytes = page_start_bytes + valid_len_bytes;
        let mut plans = Vec::new();
        let mut cursor_bytes = page_start_bytes;
        while cursor_bytes < page_valid_end_bytes {
            if !last_mapping
                .as_ref()
                .is_some_and(|mapping| mapping.file_range_bytes().contains(&cursor_bytes))
            {
                *last_mapping = Some(mapping::map_extent(
                    self.description(),
                    fs.metadata_cache(),
                    cursor_bytes,
                )?);
            }
            let mapping = last_mapping.as_ref().unwrap();
            let file_range_bytes = mapping.file_range_bytes();
            let end_bytes = file_range_bytes.end.min(page_valid_end_bytes);
            let source_range_bytes =
                cursor_bytes - file_range_bytes.start..end_bytes - file_range_bytes.start;
            let page_range_bytes = cursor_bytes - page_start_bytes..end_bytes - page_start_bytes;
            fs.pcluster().find_or_create(mapping, fs.metadata_cache())?;
            plans.push((mapping.key(), source_range_bytes, page_range_bytes));
            cursor_bytes = end_bytes;
        }
        let has_zero_tail = valid_len_bytes < mm::PAGE_SIZE;
        let page_task = OutputPageTask::new(page, plans.len() + usize::from(has_zero_tail));
        if has_zero_tail {
            *zero_range = Some(PclusterOutputPageRange::new_zero(
                page_task.clone(),
                valid_len_bytes..mm::PAGE_SIZE,
            ));
        }
        for (key, source_range_bytes, page_range_bytes) in plans {
            let page_fragment = PclusterOutputPageRange::new_data(
                page_task.clone(),
                source_range_bytes,
                page_range_bytes,
            );
            Self::insert_page_fragment(groups, key, page_fragment)?;
        }
        Ok(page_task)
    }

    /// Adds one private fragment to its batch-local pcluster group.
    fn insert_page_fragment(
        groups: &mut HashMap<PclusterKey, Vec<PclusterOutputPageRange>>,
        key: PclusterKey,
        page_fragment: PclusterOutputPageRange,
    ) -> Result<()> {
        if let Some(page_fragments) = groups.get_mut(&key) {
            page_fragments.push(page_fragment);
            return Ok(());
        }
        groups
            .try_reserve(1)
            .map_err(|_| Error::new(Errno::ENOMEM))?;
        groups.insert(key, vec![page_fragment]);
        Ok(())
    }

    /// Registers every prepared group, settling rejected fragments independently.
    fn submit_pcluster_groups(
        fs: &Erofs,
        groups: HashMap<PclusterKey, Vec<PclusterOutputPageRange>>,
        mut jobs: Vec<PclusterJob>,
    ) -> Vec<PclusterJob> {
        for (key, page_fragments) in groups {
            // Preparation established this key; the strong table never evicts it.
            let pcluster_object = fs.pcluster().find(&key).unwrap();
            match pcluster_object.insert_output_to_pcluster(page_fragments) {
                Ok(true) => jobs.push(PclusterJob::new(pcluster_object)),
                Ok(false) => {}
                Err((_error, rejected)) => {
                    for page_fragment in rejected {
                        page_fragment.finish(Err(IoError::Failed));
                    }
                }
            }
        }
        jobs
    }
}
