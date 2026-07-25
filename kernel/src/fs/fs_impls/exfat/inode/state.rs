// SPDX-License-Identifier: MPL-2.0

//! Owns inode state, state guards, cluster maps, and ordered inode-guard acquisition.
//!
//! This module is the owner for mutable inode runtime state.
//! It defines the guarded metadata and stream state,
//! the validated `ClusterMap` model used by file and directory paths,
//! and the read/write guard helpers that make inode access explicit.
//!
//! Its entry points admit state into read or write guards,
//! resolve cluster maps from on-disk stream entries,
//! publish cacheable page-backend context,
//! and acquire ordered guard sets for multi-inode operations.
//! The core data model is the relationship between file-entry metadata,
//! stream-extension cluster mapping,
//! and the page-cache-visible generation derived from them.
//!
//! Lock ordering is a hard contract here.
//! Multi-inode helpers sort and deduplicate by stable identity
//! so rename, sync, and metadata paths can avoid deadlocks while sharing the same inode domain.
//! Recovery behavior is also centralized here:
//! invalid stream state is rejected before publication,
//! and callers distinguish read-side resolution from write-side publication.
//!
//! This module does not own namespace policy or direct BIO submission.
//! It only supplies the validated state and guard machinery required by those higher layers.
//!
//! Authoritative references are Microsoft's
//! [exFAT File System Specification](https://learn.microsoft.com/en-us/windows/win32/fileio/exfat-specification),
//! Sections 7.4, 7.6, 8.1, and 9.5,
//! plus `crate::fs::fs_impls::exfat::inode::page_backend::PageCacheContext`.

use alloc::vec;
use core::cell::RefCell;

use aster_block::BlockDevice;
use ostd::sync::{RwMutexReadGuard, RwMutexWriteGuard};

use super::{
    super::{
        bitmap::{AllocGuard, AllocReadGuard, ClusterRange},
        boot::BootRegion,
        fat::{FatChainStep, FatReader},
        invalid_on_disk_layout,
    },
    ExfatInode,
    sync::InodeDirtyState,
};
use crate::{
    fs::{file::InodeType, vfs::inode::Metadata},
    prelude::*,
};

pub(super) struct InodeState {
    pub(super) dirty_state: InodeDirtyState,
    pub(super) dirty_file_retention: Option<Arc<ExfatInode>>,
    pub(super) metadata: Metadata,
    pub(super) parent: Weak<ExfatInode>,
    pub(super) cluster_map: Option<Arc<ClusterMap>>,
    pub(super) dir_entry_stream: StreamExtensionDirEntry,
}

pub(super) struct InodeStateReadGuard<'a> {
    inode: &'a ExfatInode,
    guard: RwMutexReadGuard<'a, InodeState>,
}

impl<'a> InodeStateReadGuard<'a> {
    fn new(inode: &'a ExfatInode, guard: RwMutexReadGuard<'a, InodeState>) -> Self {
        Self { inode, guard }
    }

    pub(super) fn metadata(&self) -> Metadata {
        self.guard.metadata
    }

    pub(super) fn guards_inode(&self, inode: &ExfatInode) -> bool {
        core::ptr::eq(self.inode, inode)
    }

    pub(super) fn parent(&self) -> Option<Arc<ExfatInode>> {
        self.guard.parent.upgrade()
    }

    pub(super) fn dir_entry_stream(&self) -> StreamExtensionDirEntry {
        self.guard.dir_entry_stream
    }

    pub(super) fn cached_cluster_map(&self) -> Option<Arc<ClusterMap>> {
        self.guard.cluster_map.clone()
    }

    pub(super) fn page_cache_context(&self) -> Option<super::page_backend::PageCacheContext> {
        self.inode.page_backend.page_cache_context.read().clone()
    }
}

pub(in crate::fs::fs_impls::exfat) struct InodeStateWriteGuard<'a> {
    inode: &'a ExfatInode,
    guard: RefCell<RwMutexWriteGuard<'a, InodeState>>,
}

impl<'a> InodeStateWriteGuard<'a> {
    fn new(inode: &'a ExfatInode, guard: RwMutexWriteGuard<'a, InodeState>) -> Self {
        Self {
            inode,
            guard: RefCell::new(guard),
        }
    }

    pub(super) fn metadata(&self) -> Metadata {
        self.guard.borrow().metadata
    }

    pub(super) fn guards_inode(&self, inode: &ExfatInode) -> bool {
        core::ptr::eq(self.inode, inode)
    }

    pub(super) fn with_metadata_mut<R>(
        &self,
        update_metadata_fn: impl FnOnce(&mut Metadata) -> R,
    ) -> R {
        let mut inode_state = self.guard.borrow_mut();
        update_metadata_fn(&mut inode_state.metadata)
    }

    pub(super) fn parent(&self) -> Option<Arc<ExfatInode>> {
        self.guard.borrow().parent.upgrade()
    }

    pub(super) fn set_parent(&self, parent: Weak<ExfatInode>) {
        self.guard.borrow_mut().parent = parent;
    }

    pub(super) fn dir_entry_stream(&self) -> StreamExtensionDirEntry {
        self.guard.borrow().dir_entry_stream
    }

    pub(super) fn replace_dir_entry_stream(
        &self,
        dir_entry_stream: StreamExtensionDirEntry,
    ) -> StreamExtensionDirEntry {
        let mut inode_state = self.guard.borrow_mut();
        core::mem::replace(&mut inode_state.dir_entry_stream, dir_entry_stream)
    }

    pub(super) fn set_cached_cluster_map(&self, cluster_map: Arc<ClusterMap>) {
        self.guard.borrow_mut().cluster_map = Some(cluster_map);
    }

    pub(super) fn cached_cluster_map(&self) -> Option<Arc<ClusterMap>> {
        self.guard.borrow().cluster_map.clone()
    }

    pub(super) fn page_cache_context(&self) -> Option<super::page_backend::PageCacheContext> {
        self.inode.page_backend.page_cache_context.read().clone()
    }

    pub(super) fn replace_page_cache_context(
        &self,
        page_cache_context: super::page_backend::PageCacheContext,
    ) -> Option<super::page_backend::PageCacheContext> {
        self.inode
            .page_backend
            .page_cache_context
            .write()
            .replace(page_cache_context)
    }

    pub(super) fn restore_page_cache_context(
        &self,
        page_cache_context: Option<super::page_backend::PageCacheContext>,
    ) {
        let mut active_page_cache_context = self.inode.page_backend.page_cache_context.write();
        *active_page_cache_context = page_cache_context;
    }

    pub(super) fn with_dirty_state_mut<R>(
        &self,
        update_dirty_state_fn: impl FnOnce(&mut InodeDirtyState) -> R,
    ) -> R {
        let mut inode_state = self.guard.borrow_mut();
        update_dirty_state_fn(&mut inode_state.dirty_state)
    }

    pub(super) fn dirty_state(&self) -> InodeDirtyState {
        self.guard.borrow().dirty_state
    }

    pub(super) fn has_dirty_file_retention(&self) -> bool {
        self.guard.borrow().dirty_file_retention.is_some()
    }

    pub(in crate::fs::fs_impls::exfat) fn set_dirty_file_retention(
        &self,
        retained_inode: Option<Arc<ExfatInode>>,
    ) {
        self.guard.borrow_mut().dirty_file_retention = retained_inode;
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(in crate::fs::fs_impls::exfat) struct StreamExtensionDirEntry {
    // `None` is reserved for the unbounded root directory; ordinary files and
    // directories always keep `Some(data_length)`.
    pub(in crate::fs::fs_impls::exfat) data_length: Option<usize>,
    pub(in crate::fs::fs_impls::exfat) first_cluster: u32,
    // `None` is reserved for the unbounded root directory.
    pub(in crate::fs::fs_impls::exfat) valid_data_length: Option<usize>,
    pub(in crate::fs::fs_impls::exfat) no_fat_chain: bool,
}

#[derive(Clone)]
pub(in crate::fs::fs_impls::exfat) struct ClusterMap {
    stream_extension: StreamExtensionDirEntry,
    cluster_ranges: Vec<ClusterRange>,
}

impl ClusterMap {
    pub(super) fn from_stream_and_ranges(
        boot_region: &BootRegion,
        stream_extension: StreamExtensionDirEntry,
        cluster_ranges: Vec<ClusterRange>,
    ) -> Result<Self> {
        let cluster_map = Self {
            stream_extension,
            cluster_ranges,
        };
        let data_length = match cluster_map.stream_extension.data_length {
            Some(data_length) => {
                let (_, _) = cluster_map.validated_data_lengths()?;
                data_length
            }
            None => {
                if cluster_map.stream_extension.valid_data_length.is_some()
                    || cluster_map.stream_extension.no_fat_chain
                {
                    return_errno!(Errno::EINVAL);
                }
                cluster_map.allocated_byte_length(boot_region)?
            }
        };
        if data_length == 0 {
            if !cluster_map.cluster_ranges.is_empty() {
                return_errno!(Errno::EINVAL);
            }
            return Ok(cluster_map);
        }

        let allocated_clusters = data_length.div_ceil(boot_region.cluster_size);
        let materialized_clusters =
            cluster_map
                .cluster_ranges
                .iter()
                .try_fold(0usize, |total_clusters, range| {
                    if range.cluster_count == 0 {
                        return Err(Error::new(Errno::EINVAL));
                    }
                    let last_cluster = range
                        .start_cluster
                        .checked_add(
                            u32::try_from(range.cluster_count - 1)
                                .map_err(|_| Error::new(Errno::EINVAL))?,
                        )
                        .ok_or_else(|| Error::new(Errno::EINVAL))?;
                    if !boot_region.is_valid_cluster(range.start_cluster)
                        || !boot_region.is_valid_cluster(last_cluster)
                    {
                        return Err(Error::new(Errno::EINVAL));
                    }
                    total_clusters
                        .checked_add(range.cluster_count)
                        .ok_or_else(|| Error::new(Errno::EINVAL))
                })?;
        if materialized_clusters != allocated_clusters {
            return_errno!(Errno::EINVAL);
        }

        if cluster_map.stream_extension.no_fat_chain {
            let [only_range] = cluster_map.cluster_ranges.as_slice() else {
                return_errno!(Errno::EINVAL);
            };
            if only_range.start_cluster != cluster_map.stream_extension.first_cluster
                || only_range.cluster_count != allocated_clusters
            {
                return_errno!(Errno::EINVAL);
            }
        } else if cluster_map
            .cluster_ranges
            .first()
            .map(|range| range.start_cluster)
            != Some(cluster_map.stream_extension.first_cluster)
        {
            return_errno!(Errno::EINVAL);
        }

        Ok(cluster_map)
    }

    pub(super) fn appended(
        &self,
        boot_region: &BootRegion,
        stream_extension: StreamExtensionDirEntry,
        appended_ranges: &[ClusterRange],
    ) -> Result<Self> {
        let mut cluster_ranges = self.cluster_ranges.clone();
        for range in appended_ranges {
            if range.cluster_count == 0 {
                return_errno!(Errno::EINVAL);
            }
            if let Some(last_range) = cluster_ranges.last_mut() {
                let next_cluster = last_range
                    .start_cluster
                    .checked_add(
                        u32::try_from(last_range.cluster_count)
                            .map_err(|_| Error::new(Errno::EINVAL))?,
                    )
                    .ok_or_else(|| Error::new(Errno::EINVAL))?;
                if next_cluster == range.start_cluster {
                    last_range.cluster_count = last_range
                        .cluster_count
                        .checked_add(range.cluster_count)
                        .ok_or_else(|| Error::new(Errno::EINVAL))?;
                    continue;
                }
            }
            cluster_ranges.push(*range);
        }
        Self::from_stream_and_ranges(boot_region, stream_extension, cluster_ranges)
    }

    pub(super) fn stream_extension(&self) -> StreamExtensionDirEntry {
        self.stream_extension
    }

    pub(super) fn cluster_ranges(&self) -> &[ClusterRange] {
        &self.cluster_ranges
    }

    pub(super) fn validated_data_lengths(&self) -> Result<(usize, usize)> {
        let Some(data_length) = self.stream_extension.data_length else {
            return_errno!(Errno::EINVAL);
        };
        let Some(valid_data_length) = self.stream_extension.valid_data_length else {
            return_errno!(Errno::EINVAL);
        };
        if valid_data_length > data_length {
            return_errno!(Errno::EINVAL);
        }
        if data_length == 0 {
            if self.stream_extension.first_cluster != 0 || valid_data_length != 0 {
                return_errno!(Errno::EINVAL);
            }
            if !self.cluster_ranges.is_empty() {
                return_errno!(Errno::EINVAL);
            }
            return Ok((0, 0));
        }
        Ok((data_length, valid_data_length))
    }

    pub(super) fn mapped_cluster(
        &self,
        boot_region: &BootRegion,
        cluster_index: usize,
    ) -> Result<u32> {
        let (data_length, _) = self.validated_data_lengths()?;
        let allocated_clusters = data_length.div_ceil(boot_region.cluster_size);
        let materialized_clusters =
            self.cluster_ranges
                .iter()
                .try_fold(0usize, |total_clusters, range| {
                    total_clusters
                        .checked_add(range.cluster_count)
                        .ok_or_else(|| Error::new(Errno::EINVAL))
                })?;
        if materialized_clusters != allocated_clusters {
            return_errno!(Errno::EINVAL);
        }
        if cluster_index >= allocated_clusters {
            return_errno!(Errno::EINVAL);
        }

        let (range_index, cluster_index_in_range) = self.mapped_range_frontier(cluster_index)?;
        self.cluster_ranges[range_index]
            .start_cluster
            .checked_add(
                u32::try_from(cluster_index_in_range).map_err(|_| Error::new(Errno::EINVAL))?,
            )
            .ok_or_else(|| Error::new(Errno::EINVAL))
    }

    fn mapped_range_frontier(&self, cluster_index: usize) -> Result<(usize, usize)> {
        let mut remaining_clusters = cluster_index;
        for (range_index, range) in self.cluster_ranges.iter().enumerate() {
            if remaining_clusters < range.cluster_count {
                return Ok((range_index, remaining_clusters));
            }
            remaining_clusters -= range.cluster_count;
        }
        return_errno!(Errno::EINVAL);
    }

    pub(super) fn allocated_byte_length(&self, boot_region: &BootRegion) -> Result<usize> {
        self.cluster_ranges
            .iter()
            .try_fold(0usize, |length, range| {
                length
                    .checked_add(
                        range
                            .cluster_count
                            .checked_mul(boot_region.cluster_size)
                            .ok_or_else(|| Error::new(Errno::EINVAL))?,
                    )
                    .ok_or_else(|| Error::new(Errno::EINVAL))
            })
    }

    pub(super) fn terminal_cluster(&self, boot_region: &BootRegion) -> Result<Option<u32>> {
        if self.cluster_ranges.is_empty() {
            return Ok(None);
        }
        let last_range = self
            .cluster_ranges
            .last()
            .ok_or_else(|| Error::new(Errno::EINVAL))?;
        let last_offset =
            u32::try_from(last_range.cluster_count - 1).map_err(|_| Error::new(Errno::EINVAL))?;
        let terminal_cluster = last_range
            .start_cluster
            .checked_add(last_offset)
            .ok_or_else(|| Error::new(Errno::EINVAL))?;
        if !boot_region.is_valid_cluster(terminal_cluster) {
            return_errno!(Errno::EINVAL);
        }
        Ok(Some(terminal_cluster))
    }
}

impl ExfatInode {
    pub(super) fn inode_state_read_guard(&self) -> InodeStateReadGuard<'_> {
        InodeStateReadGuard::new(self, self.inode_state.read())
    }

    pub(in crate::fs::fs_impls::exfat) fn inode_state_write_guard(
        &self,
    ) -> InodeStateWriteGuard<'_> {
        InodeStateWriteGuard::new(self, self.inode_state.write())
    }
}

// ---- Cluster map resolution ----
impl ExfatInode {
    pub(in crate::fs::fs_impls::exfat) fn resolve_cluster_map(
        block_device: &Arc<dyn BlockDevice>,
        boot_region: &BootRegion,
        cluster_map: StreamExtensionDirEntry,
    ) -> Result<ClusterMap> {
        let Some(data_length) = cluster_map.data_length else {
            if cluster_map.valid_data_length.is_some()
                || cluster_map.no_fat_chain
                || !boot_region.is_valid_cluster(cluster_map.first_cluster)
            {
                return_errno!(Errno::EINVAL);
            }
            let mut cluster_ranges: Vec<ClusterRange> = Vec::new();
            let mut visited_clusters = BTreeSet::new();
            let mut current_cluster = cluster_map.first_cluster;
            let mut fat_reader = FatReader::new(block_device.as_ref(), boot_region);
            loop {
                if !visited_clusters.insert(current_cluster) {
                    return Err(invalid_on_disk_layout());
                }
                match cluster_ranges.last_mut() {
                    Some(range)
                        if range.start_cluster.checked_add(
                            u32::try_from(range.cluster_count)
                                .map_err(|_| invalid_on_disk_layout())?,
                        ) == Some(current_cluster) =>
                    {
                        range.cluster_count += 1;
                    }
                    _ => cluster_ranges.push(ClusterRange {
                        start_cluster: current_cluster,
                        cluster_count: 1,
                    }),
                }
                match fat_reader.next_cluster(current_cluster)? {
                    FatChainStep::Continue(next_cluster) => current_cluster = next_cluster,
                    FatChainStep::End => break,
                }
            }
            return ClusterMap::from_stream_and_ranges(boot_region, cluster_map, cluster_ranges);
        };
        let Some(valid_data_length) = cluster_map.valid_data_length else {
            return_errno!(Errno::EINVAL);
        };
        if valid_data_length > data_length {
            return_errno!(Errno::EINVAL);
        }
        if data_length == 0 {
            if cluster_map.first_cluster != 0 || valid_data_length != 0 {
                return_errno!(Errno::EINVAL);
            }
            return Ok(ClusterMap {
                stream_extension: cluster_map,
                cluster_ranges: Vec::new(),
            });
        }

        boot_region.validate_stream_data(
            cluster_map.first_cluster,
            u64::try_from(data_length).map_err(|_| Error::new(Errno::EINVAL))?,
        )?;
        let allocated_clusters = data_length.div_ceil(boot_region.cluster_size);
        let cluster_ranges = if cluster_map.no_fat_chain {
            let last_cluster = cluster_map
                .first_cluster
                .checked_add(
                    u32::try_from(allocated_clusters.saturating_sub(1))
                        .map_err(|_| invalid_on_disk_layout())?,
                )
                .ok_or_else(invalid_on_disk_layout)?;
            if !boot_region.is_valid_cluster(last_cluster) {
                return Err(invalid_on_disk_layout());
            }
            vec![ClusterRange {
                start_cluster: cluster_map.first_cluster,
                cluster_count: allocated_clusters,
            }]
        } else {
            let mut cluster_ranges: Vec<ClusterRange> = Vec::new();
            let mut current_cluster = cluster_map.first_cluster;
            let mut visited_clusters = BTreeSet::new();
            let mut fat_reader = FatReader::new(block_device.as_ref(), boot_region);
            for cluster_index in 0..allocated_clusters {
                if !visited_clusters.insert(current_cluster) {
                    return Err(invalid_on_disk_layout());
                }
                match cluster_ranges.last_mut() {
                    Some(range)
                        if range.start_cluster.checked_add(
                            u32::try_from(range.cluster_count)
                                .map_err(|_| invalid_on_disk_layout())?,
                        ) == Some(current_cluster) =>
                    {
                        range.cluster_count += 1;
                    }
                    _ => cluster_ranges.push(ClusterRange {
                        start_cluster: current_cluster,
                        cluster_count: 1,
                    }),
                }
                let next_step = fat_reader.next_cluster(current_cluster)?;
                if cluster_index + 1 == allocated_clusters {
                    match next_step {
                        FatChainStep::End => {}
                        FatChainStep::Continue(_) => return Err(invalid_on_disk_layout()),
                    }
                    break;
                }
                current_cluster = match next_step {
                    FatChainStep::Continue(next_cluster) => next_cluster,
                    FatChainStep::End => return Err(invalid_on_disk_layout()),
                };
            }
            cluster_ranges
        };
        ClusterMap::from_stream_and_ranges(boot_region, cluster_map, cluster_ranges)
    }

    pub(super) fn cluster_map_for_read_guard(
        &self,
        inode_state_guard: &InodeStateReadGuard<'_>,
        _allocation_guard: &AllocReadGuard<'_>,
        cluster_map: StreamExtensionDirEntry,
    ) -> Result<Arc<ClusterMap>> {
        // A cached generation is reusable only when it still describes the exact admitted
        // stream-extension identity.
        // The read path may resolve a fresh map for this caller,
        // but it must not publish that resolution back into shared inode state.
        if let Some(generation) = inode_state_guard
            .cached_cluster_map()
            .filter(|generation| generation.stream_extension() == cluster_map)
        {
            return Ok(generation);
        }
        let fs = self
            .fs
            .upgrade()
            .ok_or_else(|| Error::with_message(Errno::EIO, "exFAT filesystem is not mounted"))?;
        Ok(Arc::new(Self::resolve_cluster_map(
            &fs.immutable_block_device(),
            &fs.immutable_boot_region(),
            cluster_map,
        )?))
    }

    pub(super) fn cluster_map_for_write_guard(
        &self,
        inode_state_guard: &InodeStateWriteGuard<'_>,
        _allocation_guard: &AllocGuard<'_>,
        cluster_map: StreamExtensionDirEntry,
    ) -> Result<Arc<ClusterMap>> {
        if let Some(generation) = inode_state_guard
            .cached_cluster_map()
            .filter(|generation| generation.stream_extension() == cluster_map)
        {
            return Ok(generation);
        }
        let fs = self
            .fs
            .upgrade()
            .ok_or_else(|| Error::with_message(Errno::EIO, "exFAT filesystem is not mounted"))?;
        let generation = Arc::new(Self::resolve_cluster_map(
            &fs.immutable_block_device(),
            &fs.immutable_boot_region(),
            cluster_map,
        )?);
        // The write-admitted path publishes the validated generation here
        // so later mutation and page-cache steps observe the same cluster-map identity.
        // The read-admitted path intentionally leaves publication to its caller.
        inode_state_guard.set_cached_cluster_map(generation.clone());
        Ok(generation)
    }

    pub(super) fn ensure_cluster_map(
        &self,
        inode_state_guard: &InodeStateWriteGuard<'_>,
        allocation_guard: &AllocGuard<'_>,
    ) -> Result<Arc<ClusterMap>> {
        if inode_state_guard.metadata().type_ != InodeType::File {
            return_errno!(Errno::EOPNOTSUPP);
        }
        if let Some(page_cache_context) = inode_state_guard.page_cache_context() {
            return match page_cache_context {
                super::page_backend::PageCacheContext::RegularFile { cluster_map, .. } => {
                    Ok(cluster_map)
                }
                super::page_backend::PageCacheContext::Directory { .. } => {
                    return_errno!(Errno::EINVAL)
                }
            };
        }
        let cluster_map = inode_state_guard.dir_entry_stream();
        let generation =
            self.cluster_map_for_write_guard(inode_state_guard, allocation_guard, cluster_map)?;
        let (data_length, valid_data_length) = generation.validated_data_lengths()?;
        let page_cache_context = self.page_cache_context_for_mapping(
            inode_state_guard.metadata(),
            generation.clone(),
            data_length,
            valid_data_length,
        )?;
        let _ = inode_state_guard.replace_page_cache_context(page_cache_context);
        Ok(generation)
    }

    pub(super) fn cluster_map_for_admitted_read(
        &self,
        inode_state_guard: &InodeStateReadGuard<'_>,
        allocation_guard: &AllocReadGuard<'_>,
    ) -> Result<(Arc<ClusterMap>, usize, usize)> {
        match inode_state_guard.metadata().type_ {
            InodeType::Dir => return_errno!(Errno::EISDIR),
            InodeType::File => {}
            _ => return_errno!(Errno::EOPNOTSUPP),
        }

        if let Some(page_cache_context) = inode_state_guard.page_cache_context() {
            return match page_cache_context {
                super::page_backend::PageCacheContext::RegularFile {
                    cluster_map,
                    data_length,
                    valid_data_length,
                    ..
                } => {
                    if valid_data_length > data_length {
                        return_errno!(Errno::EINVAL);
                    }
                    Ok((cluster_map, data_length, valid_data_length))
                }
                super::page_backend::PageCacheContext::Directory { .. } => {
                    return_errno!(Errno::EINVAL)
                }
            };
        }

        let cluster_map = inode_state_guard.dir_entry_stream();
        let generation =
            self.cluster_map_for_read_guard(inode_state_guard, allocation_guard, cluster_map)?;
        let (data_length, valid_data_length) = generation.validated_data_lengths()?;
        *self.page_backend.page_cache_context.write() = Some(self.page_cache_context_for_mapping(
            inode_state_guard.metadata(),
            generation.clone(),
            data_length,
            valid_data_length,
        )?);
        Ok((generation, data_length, valid_data_length))
    }

    pub(super) fn replace_cluster_map(
        &self,
        inode_state_guard: &InodeStateWriteGuard<'_>,
        previous_generation: &Arc<ClusterMap>,
        next_generation: Arc<ClusterMap>,
        page_cache_context: super::page_backend::PageCacheContext,
    ) -> Arc<ClusterMap> {
        let _ = inode_state_guard.replace_dir_entry_stream(next_generation.stream_extension());
        inode_state_guard.set_cached_cluster_map(next_generation);
        let _ = inode_state_guard.replace_page_cache_context(page_cache_context);
        previous_generation.clone()
    }
}

impl ExfatInode {
    // Multi-inode operations sort and deduplicate by stable lock identity
    // so every caller acquires the shared inode domain in one deadlock-avoiding order.
    pub(super) fn inode_write_guards_in_lock_order<'a>(
        mut directories: Vec<&'a ExfatInode>,
    ) -> Vec<InodeStateWriteGuard<'a>> {
        directories.sort_by_key(|directory| directory.stable_lock_identity());
        directories.dedup_by_key(|directory| directory.stable_lock_identity());
        directories
            .into_iter()
            .map(ExfatInode::inode_state_write_guard)
            .collect()
    }

    pub(super) fn inode_read_guards_in_lock_order<'a>(
        mut directories: Vec<&'a ExfatInode>,
    ) -> Vec<InodeStateReadGuard<'a>> {
        directories.sort_by_key(|directory| directory.stable_lock_identity());
        directories.dedup_by_key(|directory| directory.stable_lock_identity());
        directories
            .into_iter()
            .map(ExfatInode::inode_state_read_guard)
            .collect()
    }
}
