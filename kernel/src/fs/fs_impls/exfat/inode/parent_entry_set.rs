// SPDX-License-Identifier: MPL-2.0

//! Owns parent-directory entry-set validation, rewrite preparation, and persistence.
//!
//! This module is the owner for rewriting an inode's file-entry set inside its parent directory.
//! It validates and re-reads the parent-backed entry-set bytes,
//! prepares byte mutations that preserve the old and new images,
//! and persists rename or metadata updates in a controlled order.
//!
//! Its entry points cover validated lookup of the current parent entry set,
//! preparation of rewrite carriers,
//! and persistence helpers for ordinary updates and rename-specific phases.
//! The core data model is the parent-directory byte span that represents one inode's entry set
//! together with the prepared old/new bytes used for recovery-aware persistence.
//!
//! Locking and recovery are central here.
//! Callers supply ordered inode guards,
//! while this module preserves the target-before-source and rollback-versus-rewrite policy
//! required for namespace coherence and forced-shutdown decisions.
//!
//! This module is limited to parent-entry-set persistence.
//! It does not own namespace admission,
//! slot discovery,
//! or cluster allocation by itself.
//!
//! Authoritative references are Microsoft's
//! [exFAT File System Specification](https://learn.microsoft.com/en-us/windows/win32/fileio/exfat-specification),
//! Sections 6, 7.4, 7.6, 7.7, and 8.1.

use core::ops::Range;

use ostd::mm::VmIo;

use super::{
    super::{
        boot::BootRegion, dir_entry_format as direntry, fs::FsState, invalid_on_disk_layout,
        invalid_operation_input,
    },
    ClusterMap, ExfatInode, PersistenceRecovery,
    state::InodeStateWriteGuard,
};
use crate::{
    fs::{file::InodeType, vfs::inode::Metadata},
    prelude::*,
};

pub(super) struct PreparedEntrySetWrite {
    slot_range: direntry::DirEntrySlotRange,
    entry_set_bytes: Vec<u8>,
    old_entry_set_bytes: Vec<u8>,
}

pub(super) struct DirectoryByteMutation {
    byte_range: Range<usize>,
    old_bytes: Vec<u8>,
    new_bytes: Vec<u8>,
}

impl DirectoryByteMutation {
    pub(super) fn new(
        slot_range: direntry::DirEntrySlotRange,
        directory_bytes: &[u8],
        new_bytes: Vec<u8>,
    ) -> Result<Self> {
        let byte_range = direntry::slot_range_bytes(slot_range)?;
        if byte_range.is_empty()
            || byte_range.start % direntry::DIRECTORY_ENTRY_SIZE != 0
            || byte_range.end % direntry::DIRECTORY_ENTRY_SIZE != 0
            || new_bytes.len() != byte_range.len()
        {
            return Err(invalid_operation_input());
        }
        let old_bytes = directory_bytes
            .get(byte_range.clone())
            .ok_or_else(invalid_on_disk_layout)?
            .to_vec();
        Ok(Self {
            byte_range,
            old_bytes,
            new_bytes,
        })
    }

    pub(super) fn range_start(&self) -> usize {
        self.byte_range.start
    }

    fn byte_range(&self) -> &Range<usize> {
        &self.byte_range
    }

    fn old_bytes(&self) -> &[u8] {
        &self.old_bytes
    }

    fn new_bytes(&self) -> &[u8] {
        &self.new_bytes
    }
}

impl ExfatInode {
    pub(super) fn persist_directory_page_cache_mutation_classified(
        &self,
        fs_state: &mut FsState,
        metadata: Metadata,
        byte_mutations: &[DirectoryByteMutation],
        recovery: PersistenceRecovery,
    ) -> Result<Result<()>> {
        if metadata.type_ != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }
        if byte_mutations.is_empty() {
            return Ok(Ok(()));
        }

        let page_cache = self.page_cache_handle(metadata).cloned().ok_or_else(|| {
            Error::with_message(Errno::EIO, "directory exFAT inode has no page cache")
        })?;
        let cache_size = page_cache.size();
        let mut touched_pages = Vec::new();
        let mut previous_end = 0usize;
        for (mutation_index, mutation) in byte_mutations.iter().enumerate() {
            let byte_range = mutation.byte_range();
            let old_bytes = mutation.old_bytes();
            let new_bytes = mutation.new_bytes();
            if byte_range.is_empty()
                || old_bytes.len() != byte_range.len()
                || new_bytes.len() != byte_range.len()
                || byte_range.end > cache_size
                || (mutation_index != 0 && byte_range.start < previous_end)
            {
                return Err(invalid_operation_input());
            }
            previous_end = byte_range.end;
            let start_page = byte_range.start / PAGE_SIZE;
            let end_page = (byte_range.end - 1) / PAGE_SIZE;
            for page_idx in start_page..=end_page {
                if touched_pages.last().copied() != Some(page_idx) {
                    touched_pages.push(page_idx);
                }
            }
        }

        for mutation in byte_mutations {
            let byte_range = mutation.byte_range();
            let old_bytes = mutation.old_bytes();
            let mut prefaulted_old_bytes = vec![0; byte_range.len()];
            let mut writer = VmWriter::from(prefaulted_old_bytes.as_mut_slice()).to_fallible();
            page_cache
                .read(byte_range.start, &mut writer)
                .map_err(Error::from)?;
            if prefaulted_old_bytes.as_slice() != old_bytes {
                return Err(invalid_operation_input());
            }
        }

        let apply_result = (|| {
            for mutation in byte_mutations {
                let byte_range = mutation.byte_range();
                let new_bytes = mutation.new_bytes();
                let mut reader = VmReader::from(new_bytes).to_fallible();
                page_cache
                    .write(byte_range.start, &mut reader)
                    .map_err(Error::from)?;
            }
            Ok(())
        })();

        let mut result_error = None;
        if let Err(error) = apply_result {
            if recovery == PersistenceRecovery::RollbackAllowed {
                let rollback_result: Result<()> = (|| {
                    for mutation in byte_mutations {
                        let byte_range = mutation.byte_range();
                        let old_bytes = mutation.old_bytes();
                        let mut reader = VmReader::from(old_bytes).to_fallible();
                        page_cache
                            .write(byte_range.start, &mut reader)
                            .map_err(Error::from)?;
                    }
                    Ok(())
                })();
                match rollback_result {
                    Ok(()) => return Err(error),
                    Err(_restore_error) => {
                        let rewrite_result: Result<()> = (|| {
                            for mutation in byte_mutations {
                                let byte_range = mutation.byte_range();
                                let new_bytes = mutation.new_bytes();
                                let mut reader = VmReader::from(new_bytes).to_fallible();
                                page_cache
                                    .write(byte_range.start, &mut reader)
                                    .map_err(Error::from)?;
                            }
                            Ok(())
                        })();
                        match rewrite_result {
                            Ok(()) => result_error = Some(error),
                            Err(_) => {
                                if let Some(fs) = self.fs.upgrade() {
                                    fs.latch_forced_shutdown(fs_state);
                                }
                                return Ok(Err(error));
                            }
                        }
                    }
                }
            } else {
                let rewrite_result: Result<()> = (|| {
                    for mutation in byte_mutations {
                        let byte_range = mutation.byte_range();
                        let new_bytes = mutation.new_bytes();
                        let mut reader = VmReader::from(new_bytes).to_fallible();
                        page_cache
                            .write(byte_range.start, &mut reader)
                            .map_err(Error::from)?;
                    }
                    Ok(())
                })();
                match rewrite_result {
                    Ok(()) => result_error = Some(error),
                    Err(_) => {
                        if let Some(fs) = self.fs.upgrade() {
                            fs.latch_forced_shutdown(fs_state);
                        }
                        return Ok(Err(error));
                    }
                }
            }
        }

        let mut run_start_page = *touched_pages.first().ok_or_else(invalid_operation_input)?;
        let mut previous_page = run_start_page;
        for page_idx in touched_pages.iter().copied().skip(1) {
            if page_idx != previous_page + 1 {
                let flush_start = run_start_page
                    .checked_mul(PAGE_SIZE)
                    .ok_or_else(invalid_operation_input)?;
                let flush_end = previous_page
                    .checked_add(1)
                    .and_then(|page_idx| page_idx.checked_mul(PAGE_SIZE))
                    .ok_or_else(invalid_operation_input)?
                    .min(cache_size);
                if let Err(error) = page_cache.flush_range(flush_start..flush_end) {
                    return Ok(Err(result_error.unwrap_or(error)));
                }
                run_start_page = page_idx;
            }
            previous_page = page_idx;
        }
        let flush_start = run_start_page
            .checked_mul(PAGE_SIZE)
            .ok_or_else(invalid_operation_input)?;
        let flush_end = previous_page
            .checked_add(1)
            .and_then(|page_idx| page_idx.checked_mul(PAGE_SIZE))
            .ok_or_else(invalid_operation_input)?
            .min(cache_size);
        if let Err(error) = page_cache.flush_range(flush_start..flush_end) {
            return Ok(Err(result_error.unwrap_or(error)));
        }

        match result_error {
            Some(error) => Ok(Err(error)),
            None => Ok(Ok(())),
        }
    }

    pub(super) fn prepare_raw_entry_set_write(
        &self,
        parent_metadata: Metadata,
        slot_range: direntry::DirEntrySlotRange,
        mutation: DirectoryByteMutation,
    ) -> Result<PreparedEntrySetWrite> {
        let slot_byte_range = direntry::slot_range_bytes(slot_range)?;
        if mutation.byte_range != slot_byte_range {
            return Err(invalid_operation_input());
        }
        let DirectoryByteMutation {
            old_bytes: old_entry_set_bytes,
            new_bytes: entry_set_bytes,
            ..
        } = mutation;
        let page_cache = self
            .page_cache_handle(parent_metadata)
            .cloned()
            .ok_or_else(|| {
                Error::with_message(Errno::EIO, "directory exFAT inode has no page cache")
            })?;
        let mut prefaulted_old_bytes = vec![0; slot_byte_range.len()];
        let mut writer = VmWriter::from(prefaulted_old_bytes.as_mut_slice()).to_fallible();
        page_cache
            .read(slot_byte_range.start, &mut writer)
            .map_err(Error::from)?;
        if prefaulted_old_bytes != old_entry_set_bytes {
            return Err(invalid_operation_input());
        }
        Ok(PreparedEntrySetWrite {
            slot_range,
            entry_set_bytes,
            old_entry_set_bytes,
        })
    }

    pub(super) fn read_validated_entry_set(
        &self,
        self_inode_state_guard: &InodeStateWriteGuard<'_>,
        parent_inode_state_guard: &InodeStateWriteGuard<'_>,
        parent_cluster_map_generation: &ClusterMap,
        boot_region: &BootRegion,
    ) -> Result<(direntry::DirEntrySlotRange, Vec<u8>)> {
        let parent_cluster_map = parent_cluster_map_generation.stream_extension();
        if parent_inode_state_guard.dir_entry_stream() != parent_cluster_map {
            return Err(invalid_on_disk_layout());
        }
        let parent_inode = self_inode_state_guard.parent().ok_or_else(|| {
            Error::with_message(Errno::EIO, "ordinary exFAT inode parent is not mounted")
        })?;
        if !parent_inode_state_guard.guards_inode(parent_inode.as_ref()) {
            return Err(Error::new(Errno::EINVAL));
        }
        let logical_end = match parent_cluster_map.data_length {
            Some(data_length) => data_length,
            None => parent_cluster_map_generation.allocated_byte_length(boot_region)?,
        };
        let directory_bytes = parent_inode.read_directory_snapshot_from_page_cache(
            parent_inode_state_guard.metadata(),
            Arc::new(parent_cluster_map_generation.clone()),
            logical_end,
        )?;
        let fallback_entry_index = usize::try_from(self_inode_state_guard.metadata().ino as u32)
            .map_err(|_| Error::new(Errno::EIO))?;

        if let Some(hinted_slot_range) = self.entry_set_location_hint()? {
            let hinted_ino =
                self.entry_location_ino(parent_cluster_map, hinted_slot_range.first_entry_index())?;
            if hinted_ino != self_inode_state_guard.metadata().ino {
                self.clear_entry_set_location_hint();
            } else {
                match self.try_read_validated_entry_set_at(
                    self_inode_state_guard,
                    boot_region,
                    &directory_bytes,
                    hinted_slot_range,
                ) {
                    Ok(Some((validated_slot_range, entry_set_bytes))) => {
                        self.store_entry_set_location_hint(validated_slot_range)?;
                        return Ok((validated_slot_range, entry_set_bytes));
                    }
                    Ok(None) => {
                        self.clear_entry_set_location_hint();
                    }
                    Err(error) if error.error() == Errno::EUCLEAN => {
                        self.clear_entry_set_location_hint();
                    }
                    Err(error) => return Err(error),
                }
            }
        }

        let primary_slot_range = direntry::DirEntrySlotRange::new(fallback_entry_index, 1)?;
        let primary_entry_bytes = directory_bytes
            .get(direntry::slot_range_bytes(primary_slot_range)?)
            .ok_or_else(invalid_on_disk_layout)?
            .to_vec();
        let fallback_slot_range =
            direntry::file_primary_entry_slot_range(fallback_entry_index, &primary_entry_bytes)?;
        let (validated_slot_range, entry_set_bytes) = self
            .try_read_validated_entry_set_at(
                self_inode_state_guard,
                boot_region,
                &directory_bytes,
                fallback_slot_range,
            )?
            .ok_or_else(invalid_on_disk_layout)?;
        self.store_entry_set_location_hint(validated_slot_range)?;
        Ok((validated_slot_range, entry_set_bytes))
    }

    fn try_read_validated_entry_set_at(
        &self,
        self_inode_state_guard: &InodeStateWriteGuard<'_>,
        boot_region: &BootRegion,
        directory_bytes: &[u8],
        slot_range: direntry::DirEntrySlotRange,
    ) -> Result<Option<(direntry::DirEntrySlotRange, Vec<u8>)>> {
        let current_cluster_map = self_inode_state_guard.dir_entry_stream();
        let expected_inode_type = self_inode_state_guard.metadata().type_;
        let allow_stale_regular_file_cluster_map = expected_inode_type == InodeType::File
            && self_inode_state_guard
                .dirty_state()
                .has_deferred_regular_file_publish();
        let entry_set_bytes = directory_bytes
            .get(direntry::slot_range_bytes(slot_range)?)
            .ok_or_else(invalid_on_disk_layout)?
            .to_vec();
        let zero_based_slot_range = direntry::DirEntrySlotRange::new(0, slot_range.entry_count())?;
        let entry_view = match direntry::scan_dir_entry(
            direntry::DirectoryScanMode::Ordinary,
            &entry_set_bytes,
            0,
        ) {
            Ok(direntry::ScannedDirEntry::File(entry_view))
                if entry_view.slot_range() == zero_based_slot_range =>
            {
                entry_view
            }
            Ok(_) => return Ok(None),
            Err(error) if error.error() == Errno::EUCLEAN => return Err(error),
            Err(error) => return Err(error),
        };
        let (inode_type, _first_cluster, _data_length, _no_fat_chain) =
            entry_view.child_metadata(boot_region)?;
        match expected_inode_type {
            InodeType::Dir => {
                if inode_type != InodeType::Dir || !entry_view.is_directory() {
                    return Ok(None);
                }
            }
            InodeType::File => {
                if inode_type != InodeType::File || entry_view.is_directory() {
                    return Ok(None);
                }
            }
            _ => {
                return Err(invalid_on_disk_layout());
            }
        }
        let validated_cluster_map = entry_view.cluster_map()?;
        if !allow_stale_regular_file_cluster_map && validated_cluster_map != current_cluster_map {
            return Ok(None);
        }
        let validated_slot_range = direntry::DirEntrySlotRange::new(
            slot_range.first_entry_index(),
            entry_view.slot_range().entry_count(),
        )?;
        if validated_slot_range != slot_range {
            return Ok(None);
        }
        Ok(Some((validated_slot_range, entry_set_bytes)))
    }

    pub(super) fn rewrite_validated_entry_set_with_guard_classified(
        &self,
        fs_state: &mut FsState,
        self_inode_state_guard: &InodeStateWriteGuard<'_>,
        parent_inode_state_guard: &InodeStateWriteGuard<'_>,
        boot_region: &BootRegion,
        rewrite_entry_set_fn: impl FnOnce(direntry::FileEntrySetView<'_>) -> Result<Option<Vec<u8>>>,
        recovery: PersistenceRecovery,
    ) -> Result<Result<bool>> {
        let Some(prepared_entry_set_write) = self.prepare_rewritten_entry_set_write_with_guard(
            self_inode_state_guard,
            parent_inode_state_guard,
            boot_region,
            rewrite_entry_set_fn,
        )?
        else {
            return Ok(Ok(false));
        };
        let parent_inode = self_inode_state_guard.parent().ok_or_else(|| {
            Error::with_message(Errno::EIO, "ordinary exFAT inode parent is not mounted")
        })?;
        if !parent_inode_state_guard.guards_inode(parent_inode.as_ref()) {
            return Err(Error::new(Errno::EINVAL));
        }
        self.persist_prepared_entry_set_write_classified(
            fs_state,
            prepared_entry_set_write,
            parent_inode.as_ref(),
            parent_inode_state_guard.metadata(),
            recovery,
        )
    }

    pub(super) fn prepare_rewritten_entry_set_write_with_guard(
        &self,
        self_inode_state_guard: &InodeStateWriteGuard<'_>,
        parent_inode_state_guard: &InodeStateWriteGuard<'_>,
        boot_region: &BootRegion,
        rewrite_entry_set_fn: impl FnOnce(direntry::FileEntrySetView<'_>) -> Result<Option<Vec<u8>>>,
    ) -> Result<Option<PreparedEntrySetWrite>> {
        let parent_cluster_map = parent_inode_state_guard.dir_entry_stream();
        let parent_cluster_map_generation = parent_inode_state_guard
            .cached_cluster_map()
            .filter(|generation| generation.stream_extension() == parent_cluster_map)
            .ok_or_else(invalid_on_disk_layout)?;
        let (slot_range, mut entry_set_bytes) = self.read_validated_entry_set(
            self_inode_state_guard,
            parent_inode_state_guard,
            &parent_cluster_map_generation,
            boot_region,
        )?;
        let entry_view = match direntry::scan_dir_entry(
            direntry::DirectoryScanMode::Ordinary,
            &entry_set_bytes,
            0,
        )? {
            direntry::ScannedDirEntry::File(entry_view) => entry_view,
            _ => return Err(invalid_on_disk_layout()),
        };
        if entry_view.slot_range().entry_count() != slot_range.entry_count() {
            return Err(invalid_on_disk_layout());
        }

        let Some(updated_entry_set_bytes) = rewrite_entry_set_fn(entry_view)? else {
            return Ok(None);
        };
        if updated_entry_set_bytes.len() != entry_set_bytes.len() {
            return Err(invalid_on_disk_layout());
        }
        let old_entry_set_bytes = entry_set_bytes.clone();
        entry_set_bytes.copy_from_slice(&updated_entry_set_bytes);

        let parent_inode = self_inode_state_guard.parent().ok_or_else(|| {
            Error::with_message(Errno::EIO, "ordinary exFAT inode parent is not mounted")
        })?;
        let parent_metadata = parent_inode_state_guard.metadata();
        let slot_byte_range = direntry::slot_range_bytes(slot_range)?;
        let page_cache = parent_inode
            .page_cache_handle(parent_metadata)
            .cloned()
            .ok_or_else(|| {
                Error::with_message(Errno::EIO, "directory exFAT inode has no page cache")
            })?;
        let mut prefaulted_old_bytes = vec![0; slot_byte_range.len()];
        let mut writer = VmWriter::from(prefaulted_old_bytes.as_mut_slice()).to_fallible();
        page_cache
            .read(slot_byte_range.start, &mut writer)
            .map_err(Error::from)?;
        if prefaulted_old_bytes != old_entry_set_bytes {
            return Err(invalid_operation_input());
        }
        Ok(Some(PreparedEntrySetWrite {
            slot_range,
            entry_set_bytes,
            old_entry_set_bytes,
        }))
    }

    pub(super) fn persist_prepared_entry_set_write_classified(
        &self,
        fs_state: &mut FsState,
        prepared_entry_set_write: PreparedEntrySetWrite,
        parent_inode: &ExfatInode,
        parent_metadata: Metadata,
        recovery: PersistenceRecovery,
    ) -> Result<Result<bool>> {
        let PreparedEntrySetWrite {
            slot_range,
            entry_set_bytes,
            old_entry_set_bytes,
        } = prepared_entry_set_write;
        let slot_byte_range = direntry::slot_range_bytes(slot_range)?;
        let page_cache = parent_inode
            .page_cache_handle(parent_metadata)
            .cloned()
            .ok_or_else(|| {
                Error::with_message(Errno::EIO, "directory exFAT inode has no page cache")
            })?;
        let apply_result = {
            let mut reader = VmReader::from(entry_set_bytes.as_slice()).to_fallible();
            page_cache
                .write(slot_byte_range.start, &mut reader)
                .map_err(Error::from)
        };
        let mut persist_error = None;
        if let Err(error) = apply_result {
            // `RollbackAllowed` means we still trust the old bytes as the authoritative image.
            // If restoring those bytes fails,
            // we escalate to rewriting the intended new image,
            // and if even that cannot reestablish one coherent image we latch forced shutdown.
            if recovery == PersistenceRecovery::RollbackAllowed {
                let rollback_result = {
                    let mut reader = VmReader::from(old_entry_set_bytes.as_slice()).to_fallible();
                    page_cache
                        .write(slot_byte_range.start, &mut reader)
                        .map_err(Error::from)
                };
                match rollback_result {
                    Ok(()) => return Err(error),
                    Err(_restore_error) => {
                        let rewrite_result = {
                            let mut reader =
                                VmReader::from(entry_set_bytes.as_slice()).to_fallible();
                            page_cache
                                .write(slot_byte_range.start, &mut reader)
                                .map_err(Error::from)
                        };
                        match rewrite_result {
                            Ok(()) => persist_error = Some(error),
                            Err(_) => {
                                if let Some(fs) = self.fs.upgrade() {
                                    fs.latch_forced_shutdown(fs_state);
                                }
                                return Ok(Err(error));
                            }
                        }
                    }
                }
            } else {
                let rewrite_result = {
                    let mut reader = VmReader::from(entry_set_bytes.as_slice()).to_fallible();
                    page_cache
                        .write(slot_byte_range.start, &mut reader)
                        .map_err(Error::from)
                };
                match rewrite_result {
                    Ok(()) => persist_error = Some(error),
                    Err(_) => {
                        if let Some(fs) = self.fs.upgrade() {
                            fs.latch_forced_shutdown(fs_state);
                        }
                        return Ok(Err(error));
                    }
                }
            }
        }
        let flush_start = (slot_byte_range.start / PAGE_SIZE)
            .checked_mul(PAGE_SIZE)
            .ok_or_else(invalid_operation_input)?;
        let flush_end = ((slot_byte_range.end - 1) / PAGE_SIZE)
            .checked_add(1)
            .and_then(|page_idx| page_idx.checked_mul(PAGE_SIZE))
            .ok_or_else(invalid_operation_input)?
            .min(page_cache.size());
        if let Err(error) = page_cache.flush_range(flush_start..flush_end) {
            persist_error = Some(persist_error.unwrap_or(error));
        }
        if let Some(error) = persist_error {
            let _ = self.store_entry_set_location_hint(slot_range);
            Ok(Err(error))
        } else {
            match self.store_entry_set_location_hint(slot_range) {
                Ok(()) => Ok(Ok(true)),
                Err(error) => Ok(Err(error)),
            }
        }
    }

    pub(super) fn persist_rename_target_entry_set_classified(
        &self,
        fs_state: &mut FsState,
        parent_metadata: Metadata,
        prepared_entry_set_write: PreparedEntrySetWrite,
    ) -> Result<(Result<()>, bool)> {
        let PreparedEntrySetWrite {
            slot_range,
            entry_set_bytes,
            old_entry_set_bytes,
        } = prepared_entry_set_write;
        let slot_byte_range = direntry::slot_range_bytes(slot_range)?;
        let page_cache = self
            .page_cache_handle(parent_metadata)
            .cloned()
            .ok_or_else(|| {
                Error::with_message(Errno::EIO, "directory exFAT inode has no page cache")
            })?;
        let start_page = slot_byte_range.start / PAGE_SIZE;
        let end_page = (slot_byte_range.end - 1) / PAGE_SIZE;
        // Rename persists the target entry set before it invalidates the source entry set.
        // We flush the target only while its page-cache image is still known coherent,
        // because later source handling depends on a durable destination image already existing.
        let apply_result = {
            let mut reader = VmReader::from(entry_set_bytes.as_slice()).to_fallible();
            page_cache
                .write(slot_byte_range.start, &mut reader)
                .map_err(Error::from)
        };
        let mut persist_error = None;
        let mut image_coherent = true;
        if let Err(error) = apply_result {
            let rollback_result = {
                let mut reader = VmReader::from(old_entry_set_bytes.as_slice()).to_fallible();
                page_cache
                    .write(slot_byte_range.start, &mut reader)
                    .map_err(Error::from)
            };
            match rollback_result {
                Ok(()) => return Err(error),
                Err(_restore_error) => {
                    let rewrite_result = {
                        let mut reader = VmReader::from(entry_set_bytes.as_slice()).to_fallible();
                        page_cache
                            .write(slot_byte_range.start, &mut reader)
                            .map_err(Error::from)
                    };
                    match rewrite_result {
                        Ok(()) => persist_error = Some(error),
                        Err(_) => {
                            if let Some(fs) = self.fs.upgrade() {
                                fs.latch_forced_shutdown(fs_state);
                            }
                            image_coherent = false;
                            persist_error = Some(error);
                        }
                    }
                }
            }
        }
        if image_coherent {
            let flush_start = start_page
                .checked_mul(PAGE_SIZE)
                .ok_or_else(invalid_operation_input)?;
            let flush_end = end_page
                .checked_add(1)
                .and_then(|page_idx| page_idx.checked_mul(PAGE_SIZE))
                .ok_or_else(invalid_operation_input)?
                .min(page_cache.size());
            if let Err(error) = page_cache.flush_range(flush_start..flush_end) {
                persist_error = Some(persist_error.unwrap_or(error));
            }
        }
        Ok((persist_error.map_or(Ok(()), Err), image_coherent))
    }

    pub(super) fn persist_rename_source_entry_set_classified(
        &self,
        fs_state: &mut FsState,
        parent_metadata: Metadata,
        prepared_entry_set_write: PreparedEntrySetWrite,
    ) -> Result<(Result<()>, bool)> {
        let PreparedEntrySetWrite {
            slot_range,
            entry_set_bytes,
            ..
        } = prepared_entry_set_write;
        let slot_byte_range = direntry::slot_range_bytes(slot_range)?;
        let page_cache = self
            .page_cache_handle(parent_metadata)
            .cloned()
            .ok_or_else(|| {
                Error::with_message(Errno::EIO, "directory exFAT inode has no page cache")
            })?;
        let apply_result = {
            let mut reader = VmReader::from(entry_set_bytes.as_slice()).to_fallible();
            page_cache
                .write(slot_byte_range.start, &mut reader)
                .map_err(Error::from)
        };
        let mut persist_error = None;
        let mut image_coherent = true;
        if let Err(error) = apply_result {
            // After the rename target is durable,
            // the source phase has no rollback-to-old-bytes path without violating the published rename.
            // A failed rewrite here therefore preserves the first error,
            // and loss of one coherent source image escalates to forced shutdown.
            let rewrite_result = {
                let mut reader = VmReader::from(entry_set_bytes.as_slice()).to_fallible();
                page_cache
                    .write(slot_byte_range.start, &mut reader)
                    .map_err(Error::from)
            };
            match rewrite_result {
                Ok(()) => persist_error = Some(error),
                Err(_) => {
                    if let Some(fs) = self.fs.upgrade() {
                        fs.latch_forced_shutdown(fs_state);
                    }
                    image_coherent = false;
                    persist_error = Some(error);
                }
            }
        }
        if image_coherent {
            let flush_start = (slot_byte_range.start / PAGE_SIZE)
                .checked_mul(PAGE_SIZE)
                .ok_or_else(invalid_operation_input)?;
            let flush_end = ((slot_byte_range.end - 1) / PAGE_SIZE)
                .checked_add(1)
                .and_then(|page_idx| page_idx.checked_mul(PAGE_SIZE))
                .ok_or_else(invalid_operation_input)?
                .min(page_cache.size());
            if let Err(error) = page_cache.flush_range(flush_start..flush_end) {
                persist_error = Some(persist_error.unwrap_or(error));
            }
        }
        Ok((persist_error.map_or(Ok(()), Err), image_coherent))
    }
}
