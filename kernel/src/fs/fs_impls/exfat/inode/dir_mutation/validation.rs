// SPDX-License-Identifier: MPL-2.0

//! Owns directory-emptiness validation for namespace mutation.
//!
//! This child module answers the narrow question of whether a directory can be removed
//! without violating exFAT namespace rules.
//! It scans directory bytes for the first live child entry
//! and validates that end markers and reserved entries match the expected empty-directory shape.
//!
//! Its entry points cover first-child scanning and the final empty-directory check.
//! The data model is the validated directory byte stream
//! interpreted through exFAT end-marker and child-entry rules.
//!
//! This module does not own locking for multi-inode mutation
//! or later persistence ordering.
//! Its refusal policy is intentionally conservative:
//! malformed directory contents or unexpected children stop the mutation
//! instead of being repaired in place.
//!
//! Authoritative references are Microsoft's
//! [exFAT File System Specification](https://learn.microsoft.com/en-us/windows/win32/fileio/exfat-specification),
//! Sections 6.1, 6.2, and 9.5.

use super::super::{ExfatInode, StreamExtensionDirEntry, state::InodeStateWriteGuard};
use crate::{
    fs::exfat::{
        bitmap::AllocGuard,
        boot::BootRegion,
        dir_entry_format::{self as direntry, DirectoryScanMode, ScannedDirEntry},
        invalid_on_disk_layout,
    },
    prelude::*,
};

impl ExfatInode {
    fn first_directory_child_scan<'a>(
        cluster_map: StreamExtensionDirEntry,
        directory_bytes: &'a [u8],
    ) -> Result<Option<ScannedDirEntry<'a>>> {
        let scan_mode = if cluster_map.data_length.is_none() {
            DirectoryScanMode::Root
        } else {
            DirectoryScanMode::Ordinary
        };
        let mut entry_index = 0usize;
        loop {
            let entry_scan = direntry::scan_dir_entry(scan_mode, directory_bytes, entry_index)?;
            match entry_scan {
                ScannedDirEntry::EndOfDirectory { .. } => return Ok(None),
                ScannedDirEntry::Vacant(slot_range) => {
                    entry_index = slot_range.next_entry_index()?;
                }
                ScannedDirEntry::Issue { .. } | ScannedDirEntry::File(_) => {
                    return Ok(Some(entry_scan));
                }
            }
        }
    }

    pub(super) fn ensure_directory_snapshot_is_empty(
        child_inode: &ExfatInode,
        child_inode_state_guard: &InodeStateWriteGuard<'_>,
        allocation_guard: &AllocGuard<'_>,
        boot_region: &BootRegion,
    ) -> Result<()> {
        let cluster_map = child_inode_state_guard.dir_entry_stream();
        let cluster_map_generation = child_inode.cluster_map_for_write_guard(
            child_inode_state_guard,
            allocation_guard,
            cluster_map,
        )?;
        let logical_end = match cluster_map.data_length {
            Some(data_length) => data_length,
            None => cluster_map_generation.allocated_byte_length(boot_region)?,
        };
        let child_directory_bytes = child_inode.read_directory_snapshot_from_page_cache(
            child_inode_state_guard.metadata(),
            cluster_map_generation,
            logical_end,
        )?;
        if let Some(first_child_scan) =
            Self::first_directory_child_scan(cluster_map, &child_directory_bytes)?
        {
            match first_child_scan {
                ScannedDirEntry::Issue { .. } => {
                    return Err(invalid_on_disk_layout());
                }
                ScannedDirEntry::File(_) => return_errno!(Errno::ENOTEMPTY),
                ScannedDirEntry::EndOfDirectory { .. } | ScannedDirEntry::Vacant(_) => {
                    unreachable!()
                }
            }
        }
        Ok(())
    }
}
