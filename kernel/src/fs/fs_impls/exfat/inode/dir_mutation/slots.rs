// SPDX-License-Identifier: MPL-2.0

//! Owns directory vacant-slot discovery and slot reservation helpers.
//!
//! This child module owns the slot-level planning needed before a directory mutation
//! can write or relocate entry sets.
//! It scans directory bytes for vacant spans,
//! computes how many slots an entry set needs,
//! and reserves the source and target slot ranges used by higher-level mutation paths.
//!
//! Its entry points cover vacant-slot discovery and slot reservation.
//! The data model is the directory byte stream viewed as ordered 32-byte slots
//! with validated entry-set spans and end-marker constraints.
//!
//! Locking matters because slot reservations are meaningful only relative to the admitted directory state,
//! and recovery semantics matter because source-before-target and corruption-bound checks
//! decide whether the mutation can proceed at all.
//!
//! This module is limited to slot geometry and reservation.
//! It does not own directory growth publication,
//! rename admission,
//! or persistence ordering after slots are chosen.
//!
//! Authoritative references are Microsoft's
//! [exFAT File System Specification](https://learn.microsoft.com/en-us/windows/win32/fileio/exfat-specification),
//! Sections 6.1, 6.2, 7.4, 7.6, and 7.7.

use super::super::{
    ExfatInode, StreamExtensionDirEntry, parent_entry_set::DirectoryByteMutation,
    state::InodeStateWriteGuard,
};
use crate::{
    fs::exfat::{
        bitmap::AllocGuard,
        dir_entry_format::{
            self as direntry, DIRECTORY_ENTRY_SIZE, DirEntrySlotRange, DirectoryScanMode,
            MutableDirEntrySlotSpan, ScannedDirEntry,
        },
        fs::{ExfatFs, FsState},
        invalid_on_disk_layout, invalid_operation_input,
    },
    prelude::*,
};

impl ExfatInode {
    fn find_vacant_entry_slots(
        scan_mode: DirectoryScanMode,
        directory_bytes: &[u8],
        required_entry_count: usize,
    ) -> Result<Option<DirEntrySlotRange>> {
        if required_entry_count == 0 {
            return Err(invalid_operation_input());
        }
        if !directory_bytes.len().is_multiple_of(DIRECTORY_ENTRY_SIZE) {
            return Err(invalid_on_disk_layout());
        }

        let total_entries = directory_bytes.len() / DIRECTORY_ENTRY_SIZE;
        let mut run_length = 0usize;
        let mut run_start_index = 0usize;
        let mut entry_index = 0usize;
        loop {
            let scan_start_index = entry_index;
            match direntry::scan_dir_entry(scan_mode, directory_bytes, entry_index)? {
                ScannedDirEntry::EndOfDirectory { entry_index } => {
                    if entry_index != scan_start_index {
                        run_length = 0;
                        run_start_index = entry_index;
                    }
                    let available_entries = total_entries
                        .checked_sub(entry_index)
                        .ok_or(invalid_on_disk_layout())?;
                    if run_length == 0 {
                        run_start_index = entry_index;
                    }
                    run_length = run_length
                        .checked_add(available_entries)
                        .ok_or(invalid_on_disk_layout())?;
                    if run_length >= required_entry_count {
                        return Ok(Some(DirEntrySlotRange::new(
                            run_start_index,
                            required_entry_count,
                        )?));
                    }
                    return Ok(None);
                }
                ScannedDirEntry::Vacant(slot_range) => {
                    if run_length == 0 || slot_range.first_entry_index() != scan_start_index {
                        run_start_index = slot_range.first_entry_index();
                        run_length = 0;
                    }
                    run_length = run_length
                        .checked_add(slot_range.entry_count())
                        .ok_or(invalid_on_disk_layout())?;
                    if run_length >= required_entry_count {
                        return Ok(Some(DirEntrySlotRange::new(
                            run_start_index,
                            required_entry_count,
                        )?));
                    }
                    entry_index = slot_range.next_entry_index()?;
                }
                ScannedDirEntry::File(entry_view) => {
                    run_length = 0;
                    entry_index = entry_view.slot_range().next_entry_index()?;
                }
                ScannedDirEntry::Issue { .. } => {
                    return Err(invalid_on_disk_layout());
                }
            }
        }
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "Slot reservation may trigger directory growth, so it keeps the admitted guards, mutable allocation/filesystem state, and immutable filesystem owner explicit across the publish-or-rollback boundary."
    )]
    pub(super) fn reserve_directory_entry_slots(
        &self,
        mut cluster_map: StreamExtensionDirEntry,
        allocation_guard: &mut AllocGuard<'_>,
        fs_state: &mut FsState,
        fs: &ExfatFs,
        parent_inode_state_guard: Option<&InodeStateWriteGuard<'_>>,
        self_inode_state_guard: &InodeStateWriteGuard<'_>,
        required_entry_count: usize,
    ) -> Result<(StreamExtensionDirEntry, Vec<u8>, DirEntrySlotRange)> {
        let boot_region = fs.immutable_boot_region();
        loop {
            let cluster_map_generation = self.cluster_map_for_write_guard(
                self_inode_state_guard,
                allocation_guard,
                cluster_map,
            )?;
            let logical_end = match cluster_map.data_length {
                Some(data_length) => data_length,
                None => cluster_map_generation.allocated_byte_length(&boot_region)?,
            };
            let directory_bytes = self.read_directory_snapshot_from_page_cache(
                self_inode_state_guard.metadata(),
                cluster_map_generation,
                logical_end,
            )?;
            if let Some(slot_range) = Self::find_vacant_entry_slots(
                if cluster_map.data_length.is_none() {
                    DirectoryScanMode::Root
                } else {
                    DirectoryScanMode::Ordinary
                },
                &directory_bytes,
                required_entry_count,
            )? {
                return Ok((cluster_map, directory_bytes, slot_range));
            }
            cluster_map = self.grow_directory_cluster_map(
                cluster_map,
                allocation_guard,
                fs_state,
                fs,
                parent_inode_state_guard,
                self_inode_state_guard,
            )?;
        }
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "Rename destination reservation must thread the current snapshot, reusable-slot decision, and the same admitted growth state explicitly so destination reuse and growth stay in one transaction."
    )]
    pub(super) fn reserve_rename_destination_slot(
        &self,
        cluster_map: StreamExtensionDirEntry,
        current_directory_bytes: Vec<u8>,
        reusable_slot_range: Option<DirEntrySlotRange>,
        fs_state: &mut FsState,
        allocation_guard: &mut AllocGuard<'_>,
        fs: &ExfatFs,
        parent_inode_state_guard: Option<&InodeStateWriteGuard<'_>>,
        self_inode_state_guard: &InodeStateWriteGuard<'_>,
        required_entry_count: usize,
    ) -> Result<(StreamExtensionDirEntry, Vec<u8>, DirEntrySlotRange, bool)> {
        if let Some(slot_range) = reusable_slot_range
            .filter(|slot_range| slot_range.entry_count() >= required_entry_count)
        {
            return Ok((cluster_map, current_directory_bytes, slot_range, false));
        }
        let (updated_cluster_map, directory_bytes, slot_range) = self
            .reserve_directory_entry_slots(
                cluster_map,
                allocation_guard,
                fs_state,
                fs,
                parent_inode_state_guard,
                self_inode_state_guard,
                required_entry_count,
            )?;
        Ok((updated_cluster_map, directory_bytes, slot_range, true))
    }

    pub(super) fn prepare_invalidated_slot_mutation(
        directory_bytes: &[u8],
        slot_range: DirEntrySlotRange,
    ) -> Result<DirectoryByteMutation> {
        let byte_range = direntry::slot_range_bytes(slot_range)?;
        let mut new_bytes = directory_bytes
            .get(byte_range.clone())
            .ok_or_else(invalid_on_disk_layout)?
            .to_vec();
        let mut invalidated_entry_set =
            MutableDirEntrySlotSpan::new(slot_range, new_bytes.as_mut_slice())?;
        direntry::invalidate_entry_set(&mut invalidated_entry_set)?;
        DirectoryByteMutation::new(slot_range, directory_bytes, new_bytes)
    }

    pub(super) fn prepare_replacement_slot_mutation(
        directory_bytes: &[u8],
        destination_slot_range: DirEntrySlotRange,
        renamed_entry_set: &[u8],
    ) -> Result<DirectoryByteMutation> {
        let byte_range = direntry::slot_range_bytes(destination_slot_range)?;
        let mut new_bytes = directory_bytes
            .get(byte_range)
            .ok_or_else(invalid_on_disk_layout)?
            .to_vec();
        let mut invalidated_entry_set =
            MutableDirEntrySlotSpan::new(destination_slot_range, new_bytes.as_mut_slice())?;
        direntry::invalidate_entry_set(&mut invalidated_entry_set)?;
        new_bytes
            .get_mut(..renamed_entry_set.len())
            .ok_or_else(invalid_on_disk_layout)?
            .copy_from_slice(renamed_entry_set);
        DirectoryByteMutation::new(destination_slot_range, directory_bytes, new_bytes)
    }
}
