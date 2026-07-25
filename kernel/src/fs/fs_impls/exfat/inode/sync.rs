// SPDX-License-Identifier: MPL-2.0

//! Owns inode synchronization and the dirty-generation state machine.
//!
//! This module owns when and how an exFAT inode is considered dirty
//! and what must be persisted to make it clean again.
//! It classifies pending data and metadata work,
//! coordinates regular-file and directory sync scopes,
//! and dispatches the VFS `sync` surface for mounted exFAT inodes.
//!
//! Its entry points cover dirty-state transitions,
//! pending-sync detection,
//! sync-scope classification,
//! and the final device-facing sync operations.
//! The data model is the inode dirty-generation state machine
//! paired with the current parent and cluster-map context needed for persistence.
//!
//! Lock ordering and device ordering matter here
//! because sync may need multiple inode guards plus allocation state
//! before writing data, metadata, or parent entry sets.
//! Recovery paths keep deferred publication and forced-shutdown policy explicit
//! when parent identity, writeback, or rewrite assumptions fail.
//!
//! This module is limited to synchronization and persistence classification.
//! It does not own namespace admission or read/write I/O semantics outside sync.
//!
//! Authoritative references are Microsoft's
//! [exFAT File System Specification](https://learn.microsoft.com/en-us/windows/win32/fileio/exfat-specification),
//! Sections 7.4, 7.6, and 8.1,
//! plus `aster_block::bio::BioStatus`
//! and `crate::fs::fs_impls::exfat::fs::FsState`.

use aster_block::bio::BioStatus;

use super::{
    super::{
        bitmap::AllocGuard,
        fs::{ExfatFs, FsState},
    },
    ExfatInode,
    state::InodeStateWriteGuard,
};
use crate::prelude::*;

/// Classifies which dirty portions of an inode still need persistence.
#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum DirtyLevel {
    Clean,
    Metadata,
    Data,
    DataAndMetadata,
}

/// Tracks the dirty generations that Level-2 inode state has marked.
#[derive(Clone, Copy, Default)]
pub(super) struct InodeDirtyState {
    next_generation: u64,
    content_generation: Option<u64>,
    metadata_generation: Option<u64>,
}

impl InodeDirtyState {
    fn next_dirty_generation(&mut self) -> u64 {
        self.next_generation = self.next_generation.saturating_add(1);
        self.next_generation
    }

    pub(super) fn dirty_level(self) -> DirtyLevel {
        match (self.content_generation, self.metadata_generation) {
            (None, None) => DirtyLevel::Clean,
            (None, Some(_)) => DirtyLevel::Metadata,
            (Some(_), None) => DirtyLevel::Data,
            (Some(_), Some(_)) => DirtyLevel::DataAndMetadata,
        }
    }

    pub(super) fn mark_content_dirty(&mut self) {
        let generation = self.next_dirty_generation();
        self.content_generation = Some(generation);
        self.metadata_generation = None;
    }

    pub(super) fn mark_metadata_dirty(&mut self) {
        self.metadata_generation = Some(self.next_dirty_generation());
    }

    pub(super) fn needs_sync_data(self) -> bool {
        matches!(
            self.dirty_level(),
            DirtyLevel::Data | DirtyLevel::DataAndMetadata
        )
    }

    pub(super) fn needs_sync_all(self) -> bool {
        self.dirty_level() != DirtyLevel::Clean
    }

    pub(super) fn has_deferred_regular_file_publish(self) -> bool {
        self.content_generation.is_some()
    }

    pub(super) fn clear_detached_regular_file_publish_debt(&mut self) {
        self.content_generation = None;
        self.metadata_generation = None;
    }

    fn clear_committed_content(&mut self, synced_state: Self) {
        if synced_state
            .content_generation
            .zip(self.content_generation)
            .is_some_and(|(synced_generation, current_generation)| {
                current_generation <= synced_generation
            })
        {
            self.content_generation = None;
        }
    }

    fn clear_committed_metadata(&mut self, synced_state: Self) {
        if synced_state
            .metadata_generation
            .zip(self.metadata_generation)
            .is_some_and(|(synced_generation, current_generation)| {
                current_generation <= synced_generation
            })
        {
            self.metadata_generation = None;
        }
    }

    pub(super) fn commit_data(&mut self, synced_state: Self) {
        self.clear_committed_content(synced_state);
    }

    pub(super) fn commit_all(&mut self, synced_state: Self) {
        self.clear_committed_content(synced_state);
        self.clear_committed_metadata(synced_state);
    }
}

#[derive(Clone, Copy)]
pub(in crate::fs::fs_impls::exfat) enum InodeSyncScope {
    Data,
    All,
}

impl InodeSyncScope {
    fn needs_device_sync(self, dirty_state: InodeDirtyState) -> bool {
        match self {
            Self::Data => dirty_state.needs_sync_data(),
            Self::All => dirty_state.needs_sync_all(),
        }
    }
}

impl ExfatInode {
    pub(super) fn mark_content_dirty(&self, inode_state_guard: &InodeStateWriteGuard<'_>) {
        inode_state_guard.with_dirty_state_mut(InodeDirtyState::mark_content_dirty);
        if inode_state_guard.metadata().type_ != crate::fs::file::InodeType::File {
            // Directory entry-set mutations write through their touched
            // PageCache pages, so only regular files retain deferred content.
            return;
        }
        if !inode_state_guard.has_dirty_file_retention() {
            inode_state_guard.set_dirty_file_retention(self.weak_self().upgrade());
        }
    }

    pub(super) fn mark_metadata_dirty(&self, inode_state_guard: &InodeStateWriteGuard<'_>) {
        if inode_state_guard.metadata().type_ == crate::fs::file::InodeType::Dir {
            // Directory entry-set mutations flush their touched PageCache pages
            // before returning, so directories do not accumulate deferred inode
            // metadata debt. Directory sync still supplies the device barrier.
            return;
        }
        inode_state_guard.with_dirty_state_mut(InodeDirtyState::mark_metadata_dirty);
    }

    pub(super) fn clear_detached_regular_file_publish_debt_with_guard(
        &self,
        inode_state_guard: &InodeStateWriteGuard<'_>,
    ) {
        inode_state_guard
            .with_dirty_state_mut(InodeDirtyState::clear_detached_regular_file_publish_debt);
        inode_state_guard.set_dirty_file_retention(None);
    }

    pub(super) fn clear_dirty_file_retention_if_not_needed_with_guard(
        &self,
        inode_state_guard: &InodeStateWriteGuard<'_>,
        dirty_state: InodeDirtyState,
    ) {
        if dirty_state.has_deferred_regular_file_publish() {
            return;
        }
        inode_state_guard.set_dirty_file_retention(None);
    }

    pub(super) fn sync_regular_file(&self, scope: InodeSyncScope) -> Result<()> {
        let fs = self
            .fs
            .upgrade()
            .ok_or_else(|| Error::with_message(Errno::EIO, "exFAT filesystem is not mounted"))?;
        let mut fs_state = fs.fs_state.write();
        self.sync_regular_file_with_fs_guard(fs.as_ref(), &mut fs_state, scope)
    }

    pub(super) fn sync_directory(&self) -> Result<()> {
        let fs = self
            .fs
            .upgrade()
            .ok_or_else(|| Error::with_message(Errno::EIO, "exFAT filesystem is not mounted"))?;
        let mut fs_state = fs.fs_state.write();
        let mount_state = fs_state
            .mount_state
            .as_ref()
            .ok_or_else(super::super::not_mounted)?;
        if mount_state.forced_shutdown
            || mount_state.volume_flags.clear_to_zero
            || mount_state.volume_flags.media_failure
        {
            return_errno!(Errno::EIO);
        }

        let flush_status = match fs.immutable_block_device().sync() {
            Ok(status) => status,
            Err(_) => {
                ExfatFs::mark_mount_dirty_after_failure(&mut fs_state);
                return_errno!(Errno::EIO);
            }
        };
        if flush_status != BioStatus::Complete {
            ExfatFs::mark_mount_dirty_after_failure(&mut fs_state);
            return_errno!(Errno::EIO);
        }
        Ok(())
    }

    pub(in crate::fs::fs_impls::exfat) fn sync_regular_file_with_fs_guard(
        &self,
        fs: &ExfatFs,
        fs_state: &mut FsState,
        scope: InodeSyncScope,
    ) -> Result<()> {
        let mount_state = fs_state
            .mount_state
            .as_ref()
            .ok_or_else(super::super::not_mounted)?;
        if mount_state.forced_shutdown
            || mount_state.volume_flags.clear_to_zero
            || mount_state.volume_flags.media_failure
        {
            return_errno!(Errno::EIO);
        }

        let parent = {
            let inode_state = self.inode_state_read_guard();
            inode_state.parent()
        };
        let mut guarded_inodes = vec![self];
        if let Some(parent) = parent.as_ref() {
            guarded_inodes.push(parent.as_ref());
        }
        let inode_guards = Self::inode_write_guards_in_lock_order(guarded_inodes);
        let guard_for_inode = |inode: &ExfatInode| {
            inode_guards
                .iter()
                .find(|guard| guard.guards_inode(inode))
                .ok_or_else(|| Error::new(Errno::EINVAL))
        };
        let inode_state = guard_for_inode(self)?;
        let parent_inode_state = match parent.as_ref() {
            Some(parent) => Some(guard_for_inode(parent.as_ref())?),
            None => None,
        };
        let parent_is_revalidated = match (parent.as_ref(), inode_state.parent()) {
            (Some(discovered_parent), Some(admitted_parent)) => {
                Arc::ptr_eq(discovered_parent, &admitted_parent)
            }
            (None, None) => true,
            (Some(_), None) | (None, Some(_)) => false,
        };
        // Sync revalidates the parent identity after ordered guard acquisition
        // so it never publishes data or metadata against a stale namespace relationship.
        // A mismatch is treated as I/O failure,
        // because the caller can no longer trust which parent image this inode should persist into.
        if !parent_is_revalidated {
            return_errno!(Errno::EIO);
        }
        if inode_state.metadata().type_ != crate::fs::file::InodeType::File {
            // Directory entry sets are already write-through; their direct
            // sync path supplies the device barrier instead of file writeback.
            return Ok(());
        }
        let mut allocation_guard = fs.allocation_guard()?;
        self.sync_regular_file_with_proofs(
            fs,
            fs_state,
            scope,
            inode_state,
            parent_inode_state,
            &mut allocation_guard,
        )
    }

    pub(super) fn sync_regular_file_with_proofs(
        &self,
        fs: &ExfatFs,
        fs_state: &mut FsState,
        scope: InodeSyncScope,
        inode_state: &InodeStateWriteGuard<'_>,
        parent_inode_state: Option<&InodeStateWriteGuard<'_>>,
        allocation_guard: &mut AllocGuard<'_>,
    ) -> Result<()> {
        let block_device = fs.immutable_block_device();
        let page_cache = self
            .page_cache
            .get()
            .and_then(|maybe_page_cache| maybe_page_cache.as_ref());
        let _ = self.ensure_cluster_map(inode_state, allocation_guard)?;
        let data_length = inode_state
            .page_cache_context()
            .map(|page_cache_context| match page_cache_context {
                super::page_backend::PageCacheContext::RegularFile { data_length, .. } => {
                    Ok(data_length)
                }
                super::page_backend::PageCacheContext::Directory { .. } => {
                    Err(Error::new(Errno::EINVAL))
                }
            })
            .transpose()?
            .ok_or_else(|| Error::new(Errno::EINVAL))?;

        let dirty_state_snapshot = inode_state.dirty_state();
        let is_detached_regular_file = inode_state.parent().is_none();
        let has_cached_file_range = page_cache.is_some() && data_length != 0;
        if is_detached_regular_file && !has_cached_file_range {
            if dirty_state_snapshot.needs_sync_all() {
                self.clear_detached_regular_file_publish_debt_with_guard(inode_state);
            }
            return Ok(());
        }

        let needs_device_sync = scope.needs_device_sync(dirty_state_snapshot);
        let needs_regular_file_publish = dirty_state_snapshot.has_deferred_regular_file_publish();
        if !has_cached_file_range && !needs_device_sync {
            return Ok(());
        }

        if has_cached_file_range && let Some(page_cache) = page_cache {
            page_cache.flush_range(0..data_length)?;
        }

        if needs_regular_file_publish && !is_detached_regular_file {
            allocation_guard.publish_dirty_ranges()?;
            let parent_inode_state_guard = parent_inode_state.ok_or_else(|| {
                Error::with_message(Errno::EIO, "ordinary exFAT directory parent is not mounted")
            })?;
            self.publish_live_regular_file_entry_set(
                fs_state,
                inode_state,
                parent_inode_state_guard,
                &fs.immutable_boot_region(),
            )?;
            if block_device.sync()? != BioStatus::Complete {
                return_errno!(Errno::EIO);
            }
            allocation_guard.commit_published_ranges()?;

            let current_dirty_state = inode_state.with_dirty_state_mut(|dirty_state| {
                match scope {
                    InodeSyncScope::Data => dirty_state.commit_data(dirty_state_snapshot),
                    InodeSyncScope::All => dirty_state.commit_all(dirty_state_snapshot),
                }
                *dirty_state
            });
            self.clear_dirty_file_retention_if_not_needed_with_guard(
                inode_state,
                current_dirty_state,
            );
            return Ok(());
        }

        if block_device.sync()? != BioStatus::Complete {
            return_errno!(Errno::EIO);
        }
        allocation_guard.commit_published_ranges()?;

        let current_dirty_state = inode_state.with_dirty_state_mut(|dirty_state| {
            match scope {
                InodeSyncScope::Data => dirty_state.commit_data(dirty_state_snapshot),
                InodeSyncScope::All => dirty_state.commit_all(dirty_state_snapshot),
            }
            *dirty_state
        });
        self.clear_dirty_file_retention_if_not_needed_with_guard(inode_state, current_dirty_state);
        Ok(())
    }
}
