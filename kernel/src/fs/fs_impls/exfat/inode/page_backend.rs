// SPDX-License-Identifier: MPL-2.0

//! Bridges the exFAT inode owner to the shared exFAT page-cache backend.
//!
//! This module translates validated exFAT inode mappings
//! into the page-cache and BIO callback surface expected by the kernel VM layer.
//! It owns the callback context that pairs a cluster-map generation with boot geometry,
//! page-count derivation,
//! and page read/write submission against the underlying block device.
//!
//! Its entry points publish or clear page-cache context,
//! compute page counts from the current mapping,
//! and service page-cache callbacks that need cluster-to-block translation.
//! The data model is the page-oriented view of an exFAT stream
//! layered on top of validated cluster-map generations.
//!
//! Locking and ordering matter because callback-visible context must describe a coherent mapping
//! before page-cache I/O can re-enter the inode.
//! Completion and error paths preserve whether failure happened during mapping translation,
//! BIO execution,
//! or later page-cache completion handling.
//!
//! This module is limited to backend translation.
//! It does not own namespace, metadata, or higher-level write policy,
//! and it assumes callers publish cluster-map state before exposing page-cache work.
//!
//! Authoritative references are Microsoft's
//! [exFAT File System Specification](https://learn.microsoft.com/en-us/windows/win32/fileio/exfat-specification),
//! Sections 5.1, 7.6, and 9.5,
//! plus `crate::vm::page_cache::PageCacheBackend`
//! and `aster_block::bio::{BioStatus, BioType}`.

use core::{
    ops::Deref,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

use aster_block::{
    BlockDevice,
    bio::{BioDirection, BioSegment, BioStatus, BioType},
};
use io_util::batch::IoBatch;
use ostd::mm::{Segment, VmIo, io::util::HasVmReaderWriter};

use super::{
    super::{
        boot::BootRegion, dir_entry_format::DIRECTORY_ENTRY_SIZE, fs::MountRuntimeProjection,
        invalid_on_disk_layout,
    },
    ClusterMap, ExfatInode,
};
use crate::{
    fs::{file::InodeType, vfs::inode::Metadata},
    prelude::*,
    vm::page_cache::{CachePageExt, LockedCachePage, PageCache, PageCacheBackend},
};

#[derive(Clone)]
pub(super) enum PageCacheContext {
    RegularFile {
        cluster_map: Arc<ClusterMap>,
        data_length: usize,
        valid_data_length: usize,
        mount_runtime: Arc<MountRuntimeProjection>,
    },
    Directory {
        cluster_map: Arc<ClusterMap>,
        logical_end: usize,
        mount_runtime: Arc<MountRuntimeProjection>,
    },
}

pub(super) struct ExfatFilePageBackend {
    block_device: Arc<dyn BlockDevice>,
    boot_region: BootRegion,
    pub(super) page_cache_context: RwMutex<Option<PageCacheContext>>,
}

struct PageIoRange {
    disk_offset_bytes: usize,
}

impl ExfatFilePageBackend {
    pub(super) fn new(block_device: Arc<dyn BlockDevice>, boot_region: BootRegion) -> Self {
        Self {
            block_device,
            boot_region,
            page_cache_context: RwMutex::new(None),
        }
    }

    fn active_page_cache_context(&self) -> Result<PageCacheContext> {
        self.page_cache_context.read().clone().ok_or_else(|| {
            Error::with_message(Errno::EIO, "exFAT page-cache context is not published")
        })
    }

    fn planned_page_io(
        &self,
        page_cache_context: &PageCacheContext,
        idx: usize,
        bio_type: BioType,
    ) -> Result<(usize, PageIoRange)> {
        let (cluster_map, logical_end, initialized_limit, mount_runtime) = match page_cache_context
        {
            PageCacheContext::RegularFile {
                cluster_map,
                data_length,
                valid_data_length,
                mount_runtime,
            } => (
                cluster_map.as_ref(),
                *data_length,
                *valid_data_length,
                mount_runtime.snapshot(),
            ),
            PageCacheContext::Directory {
                cluster_map,
                logical_end,
                mount_runtime,
            } => (
                cluster_map.as_ref(),
                *logical_end,
                *logical_end,
                mount_runtime.snapshot(),
            ),
        };
        if mount_runtime.forced_shutdown
            || mount_runtime.clear_to_zero
            || mount_runtime.media_failure
        {
            return_errno!(Errno::EIO);
        }
        if matches!(bio_type, BioType::Write) && mount_runtime.read_only {
            return_errno!(Errno::EROFS);
        }
        if logical_end == 0 {
            return_errno!(Errno::EINVAL);
        }

        let page_offset = idx
            .checked_mul(PAGE_SIZE)
            .ok_or_else(|| Error::new(Errno::EINVAL))?;
        if page_offset >= logical_end {
            return_errno!(Errno::EINVAL);
        }
        let page_end = page_offset
            .checked_add(PAGE_SIZE)
            .ok_or_else(|| Error::new(Errno::EINVAL))?
            .min(logical_end);
        let initialized_len = page_end.min(initialized_limit).saturating_sub(page_offset);
        if initialized_len == 0 {
            return Ok((
                0,
                PageIoRange {
                    disk_offset_bytes: 0,
                },
            ));
        }

        let cluster_size = self.boot_region.cluster_size;
        let allocated_clusters = logical_end.div_ceil(cluster_size);
        let cluster_ranges = cluster_map.cluster_ranges();
        let materialized_clusters =
            cluster_ranges
                .iter()
                .try_fold(0usize, |total_clusters, range| {
                    total_clusters
                        .checked_add(range.cluster_count)
                        .ok_or_else(|| Error::new(Errno::EINVAL))
                })?;
        if materialized_clusters != allocated_clusters {
            return_errno!(Errno::EINVAL);
        }

        let start_cluster_index = page_offset / cluster_size;
        if start_cluster_index >= allocated_clusters {
            return_errno!(Errno::EINVAL);
        }
        let mut remaining_clusters = start_cluster_index;
        let mut range_index = 0usize;
        let mut cluster_index_in_range = 0usize;
        for (candidate_range_index, range) in cluster_ranges.iter().enumerate() {
            if remaining_clusters < range.cluster_count {
                range_index = candidate_range_index;
                cluster_index_in_range = remaining_clusters;
                break;
            }
            remaining_clusters -= range.cluster_count;
            if candidate_range_index + 1 == cluster_ranges.len() {
                return_errno!(Errno::EINVAL);
            }
        }

        let range = cluster_ranges
            .get(range_index)
            .ok_or_else(|| Error::new(Errno::EINVAL))?;
        let current_cluster = range
            .start_cluster
            .checked_add(
                u32::try_from(cluster_index_in_range).map_err(|_| Error::new(Errno::EINVAL))?,
            )
            .ok_or_else(|| Error::new(Errno::EINVAL))?;
        let page_offset_within_cluster = page_offset % cluster_size;
        // TODO: Support cached pages that span non-contiguous clusters by using sub-page BIO
        // segments or an exFAT bounce buffer. Mount validation currently prevents this case.
        if page_offset_within_cluster
            .checked_add(PAGE_SIZE)
            .is_none_or(|page_end_within_cluster| page_end_within_cluster > cluster_size)
        {
            return_errno!(Errno::EINVAL);
        }
        let disk_offset_bytes = self
            .boot_region
            .cluster_offset(current_cluster)?
            .checked_add(page_offset_within_cluster)
            .ok_or_else(|| Error::new(Errno::EINVAL))?;
        Ok((initialized_len, PageIoRange { disk_offset_bytes }))
    }

    fn submit_page_io(
        &self,
        locked_page: LockedCachePage,
        initialized_len: usize,
        page_range: PageIoRange,
        io_batch: &mut IoBatch,
        bio_type: BioType,
    ) -> Result<()> {
        let bio_direction = match bio_type {
            BioType::Read => BioDirection::FromDevice,
            BioType::Write => BioDirection::ToDevice,
            BioType::Flush => return_errno!(Errno::EINVAL),
        };
        let page_segment: ostd::mm::USegment = Segment::from(locked_page.deref().clone()).into();
        let page_io = FragmentedPageIo::new(locked_page, 1, bio_type, initialized_len);
        let bio_segment = BioSegment::new_from_segment(page_segment, bio_direction);
        let completion_io = page_io.clone();
        let complete_fn: aster_block::bio::BioCompleteFn =
            Box::new(move |status| completion_io.complete(status));
        let bio = aster_block::bio::Bio::new(
            bio_type,
            aster_block::id::Sid::from_offset(page_range.disk_offset_bytes),
            vec![bio_segment],
            Some(complete_fn),
        );
        if let Err(error) = bio.submit(self.block_device.as_ref(), io_batch) {
            page_io.fail_unsubmitted(1);
            return Err(Error::from(error));
        }

        Ok(())
    }
}

pub(super) struct FragmentedPageIo {
    page: LockedCachePage,
    pending: AtomicUsize,
    failed: AtomicBool,
    bio_type: BioType,
    initialized_len: usize,
}

impl FragmentedPageIo {
    pub(super) fn new(
        page: LockedCachePage,
        pending: usize,
        bio_type: BioType,
        initialized_len: usize,
    ) -> Arc<Self> {
        Arc::new(Self {
            page,
            pending: AtomicUsize::new(pending),
            failed: AtomicBool::new(false),
            bio_type,
            initialized_len,
        })
    }

    pub(super) fn complete(self: Arc<Self>, status: BioStatus) {
        self.complete_pending(
            1,
            status != BioStatus::Complete && status != BioStatus::Zeros,
        );
    }

    pub(super) fn fail_unsubmitted(&self, unsubmitted_bios: usize) {
        self.complete_pending(unsubmitted_bios, true);
    }

    fn complete_pending(&self, completed_bios: usize, has_failed: bool) {
        if has_failed {
            self.failed.store(true, Ordering::Release);
        }

        if self.pending.fetch_sub(completed_bios, Ordering::AcqRel) != completed_bios {
            return;
        }

        if matches!(self.bio_type, BioType::Read) {
            if !self.failed.load(Ordering::Acquire) {
                self.page
                    .writer()
                    .skip(self.initialized_len)
                    .fill_zeros(PAGE_SIZE - self.initialized_len);
                self.page.set_up_to_date();
            }
            return;
        }

        if self.failed.load(Ordering::Acquire) {
            self.page.set_dirty();
            ostd::error!("exFAT writeback failed for a cached page; data may be lost");
        }
        self.page.clear_writing_back();
    }
}

impl PageCacheBackend for ExfatFilePageBackend {
    fn read_page_async(
        &self,
        idx: usize,
        locked_page: LockedCachePage,
        io_batch: &mut IoBatch,
    ) -> Result<()> {
        let page_cache_context = self.active_page_cache_context()?;
        let (initialized_len, page_range) =
            self.planned_page_io(&page_cache_context, idx, BioType::Read)?;
        if initialized_len == 0 {
            locked_page.writer().fill_zeros(PAGE_SIZE);
            locked_page.set_up_to_date();
            return Ok(());
        }

        self.submit_page_io(
            locked_page,
            initialized_len,
            page_range,
            io_batch,
            BioType::Read,
        )
    }

    fn write_page_async(
        &self,
        idx: usize,
        locked_page: LockedCachePage,
        io_batch: &mut IoBatch,
    ) -> Result<()> {
        let page_cache_context = self.active_page_cache_context()?;
        let (initialized_len, page_range) =
            self.planned_page_io(&page_cache_context, idx, BioType::Write)?;
        if initialized_len == 0 {
            locked_page.set_up_to_date();
            return Ok(());
        }

        locked_page.wait_until_finish_writing_back();
        locked_page.set_writing_back();
        locked_page.set_up_to_date();

        self.submit_page_io(
            locked_page,
            initialized_len,
            page_range,
            io_batch,
            BioType::Write,
        )
    }
}

impl ExfatInode {
    pub(super) fn read_directory_snapshot_from_page_cache(
        &self,
        metadata: Metadata,
        cluster_map: Arc<ClusterMap>,
        logical_end: usize,
    ) -> Result<Vec<u8>> {
        if metadata.type_ != InodeType::Dir || !logical_end.is_multiple_of(DIRECTORY_ENTRY_SIZE) {
            return Err(invalid_on_disk_layout());
        }
        let fs = self
            .fs
            .upgrade()
            .ok_or_else(|| Error::with_message(Errno::EIO, "exFAT filesystem is not mounted"))?;
        if fs.mount_runtime_projection().snapshot().forced_shutdown {
            return_errno!(Errno::EIO);
        }

        let page_cache_context =
            self.page_cache_context_for_mapping(metadata, cluster_map, logical_end, logical_end)?;
        *self.page_backend.page_cache_context.write() = Some(page_cache_context);
        let page_cache = self.page_cache_handle(metadata).cloned().ok_or_else(|| {
            Error::with_message(Errno::EIO, "directory exFAT inode has no page cache")
        })?;
        let mut directory_bytes = vec![0; logical_end];
        if directory_bytes.is_empty() {
            return Ok(directory_bytes);
        }
        let mut writer = VmWriter::from(directory_bytes.as_mut_slice()).to_fallible();
        page_cache.read(0, &mut writer).map_err(Error::from)?;
        Ok(directory_bytes)
    }

    pub(super) fn page_cache_context_for_mapping(
        &self,
        metadata: Metadata,
        cluster_map: Arc<ClusterMap>,
        logical_end: usize,
        valid_data_length: usize,
    ) -> Result<PageCacheContext> {
        let fs = self
            .fs
            .upgrade()
            .ok_or_else(|| Error::with_message(Errno::EIO, "exFAT filesystem is not mounted"))?;
        match metadata.type_ {
            InodeType::File => {
                if valid_data_length > logical_end {
                    return_errno!(Errno::EINVAL);
                }
                Ok(PageCacheContext::RegularFile {
                    cluster_map,
                    data_length: logical_end,
                    valid_data_length,
                    mount_runtime: fs.mount_runtime_projection(),
                })
            }
            InodeType::Dir => {
                if valid_data_length != logical_end {
                    return_errno!(Errno::EINVAL);
                }
                Ok(PageCacheContext::Directory {
                    cluster_map,
                    logical_end,
                    mount_runtime: fs.mount_runtime_projection(),
                })
            }
            _ => return_errno!(Errno::EOPNOTSUPP),
        }
    }

    pub(super) fn weak_self(&self) -> Weak<Self> {
        self.weak_self.clone()
    }

    pub(super) fn page_cache_handle(&self, metadata: Metadata) -> Option<&PageCache> {
        if !matches!(metadata.type_, InodeType::File | InodeType::Dir) {
            return None;
        }

        self.page_cache
            .call_once(|| {
                let backend: Arc<dyn PageCacheBackend> = self.page_backend.clone();
                let capacity = metadata.size;
                PageCache::new_with_backend(capacity, Arc::downgrade(&backend)).ok()
            })
            .as_ref()
    }
}
