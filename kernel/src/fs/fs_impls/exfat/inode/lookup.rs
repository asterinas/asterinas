// SPDX-License-Identifier: MPL-2.0

//! Implements directory lookup, readdir, and child-inode materialization.
//!
//! This module owns directory traversal after the caller has admitted a directory inode.
//! It scans directory bytes,
//! performs name lookup with up-case aware comparison,
//! emits readdir entries,
//! and materializes child inodes from validated directory-entry sets.
//!
//! Its entry points cover VFS lookup and readdir dispatch
//! plus the supporting scan helpers that search a directory stream.
//! The data model is the validated directory byte stream
//! and the file-entry-set views found within it.
//!
//! Locking matters because lookup may need ordered inode guards and parent revalidation
//! while still preserving cache and namespace consistency.
//! This module does not own persistence;
//! it is limited to read-side traversal and returns explicit failure on malformed scans,
//! unsupported names,
//! or inconsistent directory contents.
//!
//! Authoritative references are Microsoft's
//! [exFAT File System Specification](https://learn.microsoft.com/en-us/windows/win32/fileio/exfat-specification),
//! Sections 6, 7.2.5, 7.6.3, 7.6.4, and 7.7,
//! plus `crate::fs::utils::DirentVisitor`
//! and `crate::fs::vfs::inode::Inode`.

use super::{
    super::{
        boot::BootRegion,
        dir_entry_format::{
            self as direntry, DIRECTORY_ENTRY_SIZE, DirEntryIssueKind, DirEntrySlotRange,
            DirectoryScanMode, FileEntrySetView, ScannedDirEntry,
        },
        fs::MountOptions,
        invalid_on_disk_layout, invalid_operation_input,
    },
    ExfatFs, ExfatInode, StreamExtensionDirEntry, UpcaseTable,
    state::InodeStateReadGuard,
};
use crate::{
    fs::{file::InodeType, utils::DirentVisitor, vfs::inode::Inode},
    prelude::*,
};

impl ExfatInode {
    pub(super) fn entry_location_ino(
        &self,
        cluster_map: StreamExtensionDirEntry,
        entry_index: usize,
    ) -> Result<u64> {
        Ok((u64::from(cluster_map.first_cluster) << 32)
            | u64::from(u32::try_from(entry_index).map_err(|_| invalid_on_disk_layout())?))
    }

    pub(super) fn child_inode_from_directory_entry(
        parent: &Self,
        fs: &Arc<ExfatFs>,
        boot_region: &BootRegion,
        parent_first_cluster: u32,
        slot_range: DirEntrySlotRange,
        inode_type: InodeType,
        child_stream: StreamExtensionDirEntry,
    ) -> Result<Arc<Self>> {
        let child_ino = (u64::from(parent_first_cluster) << 32)
            | u64::from(
                u32::try_from(slot_range.first_entry_index())
                    .map_err(|_| invalid_on_disk_layout())?,
            );
        let child_cluster_map = (inode_type == InodeType::Dir)
            .then(|| {
                Self::resolve_cluster_map(&fs.immutable_block_device(), boot_region, child_stream)
            })
            .transpose()?
            .map(Arc::new);
        let child_inode = Self::new_child(
            fs,
            parent.weak_self(),
            child_ino,
            inode_type,
            child_stream
                .data_length
                .ok_or_else(invalid_on_disk_layout)?,
            child_stream,
            child_cluster_map.clone(),
        );
        if let Some(child_cluster_map) = child_cluster_map {
            child_inode.reconstruct_directory_link_count(
                boot_region,
                child_cluster_map,
                child_stream
                    .data_length
                    .ok_or_else(invalid_on_disk_layout)?,
                DirectoryScanMode::Ordinary,
            )?;
        }
        if inode_type == InodeType::File {
            child_inode.store_entry_set_location_hint(slot_range)?;
        }
        Ok(child_inode)
    }

    pub(super) fn validate_name(
        name: &str,
        options: &MountOptions,
    ) -> core::result::Result<Vec<u16>, Error> {
        let normalized_name = if options.keep_last_dots {
            name
        } else {
            name.trim_end_matches('.')
        };
        if normalized_name.is_empty() || normalized_name == "." || normalized_name == ".." {
            return_errno_with_message!(Errno::EINVAL, "invalid exFAT name");
        }

        let mut name = Vec::new();
        for character in normalized_name.chars() {
            if character <= '\u{001F}'
                || matches!(
                    character,
                    '"' | '*' | '/' | ':' | '<' | '>' | '?' | '\\' | '|'
                )
            {
                return_errno_with_message!(Errno::EINVAL, "invalid exFAT name");
            }
            let mut encoded = [0u16; 2];
            name.extend(character.encode_utf16(&mut encoded).iter().copied());
        }
        if name.len() > UpcaseTable::NAME_MAX {
            return_errno!(Errno::ENAMETOOLONG);
        }
        Ok(name)
    }

    fn readdir_cookie_for_entry_index(entry_index: usize) -> Result<usize> {
        entry_index
            .checked_add(1)
            .and_then(|entry_count| entry_count.checked_mul(DIRECTORY_ENTRY_SIZE))
            .ok_or_else(invalid_operation_input)
    }

    fn normalize_readdir_offset(offset: usize) -> Result<(usize, usize)> {
        let normalized_offset = offset
            .checked_add(DIRECTORY_ENTRY_SIZE - 1)
            .and_then(|rounded| {
                rounded
                    .checked_div(DIRECTORY_ENTRY_SIZE)
                    .and_then(|aligned| aligned.checked_mul(DIRECTORY_ENTRY_SIZE))
            })
            .map(|aligned| aligned.max(DIRECTORY_ENTRY_SIZE))
            .ok_or_else(invalid_operation_input)?;
        let entry_index = normalized_offset
            .checked_div(DIRECTORY_ENTRY_SIZE)
            .and_then(|aligned_entries| aligned_entries.checked_sub(1))
            .ok_or_else(invalid_operation_input)?;
        Ok((normalized_offset, entry_index))
    }

    pub(super) fn lookup_child_by_name(
        &self,
        fs: &Arc<ExfatFs>,
        fs_state: &mut super::super::fs::FsState,
        inode_state_guard: &InodeStateReadGuard<'_>,
        upcase_table: &UpcaseTable,
        lookup_name: &[u16],
        lookup_name_hash: u16,
    ) -> Result<Option<Arc<dyn Inode>>> {
        let block_device = fs.immutable_block_device();
        let boot_region = fs.immutable_boot_region();
        let cluster_map = inode_state_guard.dir_entry_stream();
        let allocation_guard = fs.allocation_read_guard()?;
        let cluster_map_generation =
            self.cluster_map_for_read_guard(inode_state_guard, &allocation_guard, cluster_map)?;
        let logical_end = match cluster_map.data_length {
            Some(data_length) => data_length,
            None => cluster_map_generation.allocated_byte_length(&boot_region)?,
        };
        let directory_bytes = self.read_directory_snapshot_from_page_cache(
            inode_state_guard.metadata(),
            cluster_map_generation,
            logical_end,
        )?;
        let Some(entry_view) = Self::locate_named_child_view(
            &directory_bytes,
            if cluster_map.data_length.is_none() {
                DirectoryScanMode::Root
            } else {
                DirectoryScanMode::Ordinary
            },
            upcase_table,
            lookup_name,
            lookup_name_hash,
        )?
        else {
            return Ok(None);
        };
        let slot_range = entry_view.slot_range();
        let (inode_type, first_cluster, data_length, no_fat_chain) =
            entry_view.child_metadata(&boot_region)?;
        let ino = (u64::from(cluster_map.first_cluster) << 32)
            | u64::from(
                u32::try_from(slot_range.first_entry_index())
                    .map_err(|_| invalid_on_disk_layout())?,
            );
        let valid_data_length = entry_view
            .cluster_map()?
            .valid_data_length
            .ok_or_else(invalid_on_disk_layout)?;
        if valid_data_length > data_length {
            return Err(invalid_on_disk_layout());
        }

        if let Some(child_inode) = ExfatFs::peek_cached_inode(fs_state, ino) {
            let child_inode: Arc<dyn Inode> = child_inode;
            return Ok(Some(child_inode));
        }

        let child_stream = StreamExtensionDirEntry {
            data_length: Some(data_length),
            first_cluster,
            valid_data_length: Some(valid_data_length),
            no_fat_chain,
        };
        let child_cluster_map = (inode_type == InodeType::Dir)
            .then(|| Self::resolve_cluster_map(&block_device, &boot_region, child_stream))
            .transpose()?
            .map(Arc::new);
        let child_inode = Self::new_child(
            fs,
            self.weak_self(),
            ino,
            inode_type,
            data_length,
            child_stream,
            child_cluster_map.clone(),
        );
        {
            let unpublished_child_guard = child_inode.inode_state_write_guard();
            child_inode.refresh_cached_metadata_from_entry_view(
                &unpublished_child_guard,
                entry_view,
                &boot_region,
            )?;
        }
        if let Some(child_cluster_map) = child_cluster_map {
            child_inode.reconstruct_directory_link_count(
                &boot_region,
                child_cluster_map,
                data_length,
                DirectoryScanMode::Ordinary,
            )?;
        }
        if inode_type == InodeType::File {
            child_inode.store_entry_set_location_hint(slot_range)?;
        }
        ExfatFs::publish_cached_inode(fs_state, ino, &child_inode);
        Ok(Some(child_inode))
    }

    pub(super) fn locate_named_child_view<'a>(
        directory_bytes: &'a [u8],
        scan_mode: DirectoryScanMode,
        upcase_table: &UpcaseTable,
        lookup_name: &[u16],
        lookup_name_hash: u16,
    ) -> Result<Option<FileEntrySetView<'a>>> {
        let mut entry_index = 0usize;
        loop {
            match direntry::scan_dir_entry(scan_mode, directory_bytes, entry_index)? {
                ScannedDirEntry::EndOfDirectory { .. } => return Ok(None),
                ScannedDirEntry::Vacant(slot_range) => {
                    entry_index = slot_range.next_entry_index()?;
                }
                ScannedDirEntry::File(entry_view) => {
                    let candidate_name = entry_view.name()?;
                    if entry_view.stored_name_hash() == lookup_name_hash
                        && upcase_table.names_equal(lookup_name, &candidate_name)
                    {
                        return Ok(Some(entry_view));
                    }
                    entry_index = entry_view.slot_range().next_entry_index()?;
                }
                ScannedDirEntry::Issue { kind, slot_range } => {
                    if kind == DirEntryIssueKind::BenignUnrecognizedEntrySet {
                        entry_index = slot_range.next_entry_index()?;
                        continue;
                    }
                    return Err(invalid_on_disk_layout());
                }
            }
        }
    }

    pub(super) fn readdir_at_impl(
        &self,
        offset: usize,
        visitor: &mut dyn DirentVisitor,
    ) -> Result<usize> {
        let readdir_result = (|| -> Result<(usize, bool)> {
            let fs = self.fs.upgrade().ok_or_else(|| {
                Error::with_message(Errno::EIO, "exFAT filesystem is not mounted")
            })?;
            let boot_region = fs.immutable_boot_region();
            let _fs_state = fs.fs_state.read();
            let inode_state_guard = self.inode_state_read_guard();
            if inode_state_guard.metadata().type_ != InodeType::Dir {
                return_errno!(Errno::ENOTDIR);
            }
            let parent = inode_state_guard.parent();
            drop(inode_state_guard);
            let mut guarded_inodes = vec![self];
            if let Some(parent) = parent.as_ref() {
                guarded_inodes.push(parent.as_ref());
            }
            let inode_guards = Self::inode_read_guards_in_lock_order(guarded_inodes);
            let inode_state_guard = inode_guards
                .iter()
                .find(|guard| guard.guards_inode(self))
                .ok_or_else(|| Error::new(Errno::EINVAL))?;
            let _allocation_guard = fs.allocation_read_guard()?;
            let directory_ino = inode_state_guard.metadata().ino;
            let directory_stream = inode_state_guard.dir_entry_stream();
            let cluster_map = self.cluster_map_for_read_guard(
                inode_state_guard,
                &_allocation_guard,
                directory_stream,
            )?;
            let logical_end = match directory_stream.data_length {
                Some(data_length) => data_length,
                None => cluster_map.allocated_byte_length(&boot_region)?,
            };
            let directory_bytes = self.read_directory_snapshot_from_page_cache(
                inode_state_guard.metadata(),
                cluster_map,
                logical_end,
            )?;
            let mut next_offset = offset;
            let mut accepted_entry = false;
            if next_offset == 0 {
                let dot_next_offset = next_offset
                    .checked_add(1)
                    .ok_or_else(invalid_on_disk_layout)?;
                visitor.visit(".", directory_ino, InodeType::Dir, dot_next_offset)?;
                next_offset = dot_next_offset;
                accepted_entry = true;
            }
            if next_offset == 1 {
                let dotdot_next_offset = DIRECTORY_ENTRY_SIZE;
                let parent_ino = match parent.as_ref() {
                    Some(parent) => {
                        inode_guards
                            .iter()
                            .find(|guard| guard.guards_inode(parent.as_ref()))
                            .ok_or_else(|| Error::new(Errno::EINVAL))?
                            .metadata()
                            .ino
                    }
                    None => directory_ino,
                };
                if let Err(error) =
                    visitor.visit("..", parent_ino, InodeType::Dir, dotdot_next_offset)
                {
                    if !accepted_entry {
                        return Err(error);
                    }
                    return Ok((next_offset.saturating_sub(offset), true));
                }
                next_offset = dotdot_next_offset;
                accepted_entry = true;
            }

            let mut entry_index = 0usize;
            if next_offset >= 2 {
                let (normalized_offset, normalized_entry_index) =
                    Self::normalize_readdir_offset(next_offset)?;
                next_offset = normalized_offset;
                entry_index = normalized_entry_index;
            }
            let logical_offset = entry_index
                .checked_mul(DIRECTORY_ENTRY_SIZE)
                .ok_or_else(invalid_operation_input)?;
            if logical_offset >= logical_end {
                return Ok((next_offset.saturating_sub(offset), accepted_entry));
            }
            loop {
                match direntry::scan_dir_entry(
                    if directory_stream.data_length.is_none() {
                        DirectoryScanMode::Root
                    } else {
                        DirectoryScanMode::Ordinary
                    },
                    &directory_bytes,
                    entry_index,
                )? {
                    ScannedDirEntry::EndOfDirectory {
                        entry_index: end_entry_index,
                    } => {
                        next_offset = Self::readdir_cookie_for_entry_index(end_entry_index)?;
                        break;
                    }
                    ScannedDirEntry::Vacant(slot_range) => {
                        entry_index = slot_range.next_entry_index()?;
                        next_offset = Self::readdir_cookie_for_entry_index(entry_index)?;
                    }
                    ScannedDirEntry::File(entry_view) => {
                        let slot_range = entry_view.slot_range();
                        let resume_offset =
                            Self::readdir_cookie_for_entry_index(slot_range.next_entry_index()?)?;
                        let candidate_name = entry_view.name()?;
                        let (inode_type, _, _, _) = entry_view.child_metadata(&boot_region)?;
                        let entry_name = String::from_utf16(&candidate_name)
                            .map_err(|_| invalid_on_disk_layout())?;
                        let entry_ino = (u64::from(directory_stream.first_cluster) << 32)
                            | u64::from(
                                u32::try_from(slot_range.first_entry_index())
                                    .map_err(|_| invalid_on_disk_layout())?,
                            );
                        if let Err(error) =
                            visitor.visit(&entry_name, entry_ino, inode_type, resume_offset)
                        {
                            if !accepted_entry {
                                return Err(error);
                            }
                            return Ok((next_offset.saturating_sub(offset), true));
                        }
                        accepted_entry = true;
                        next_offset = resume_offset;
                        entry_index = slot_range.next_entry_index()?;
                    }
                    ScannedDirEntry::Issue {
                        kind: DirEntryIssueKind::BenignUnrecognizedEntrySet,
                        slot_range,
                    } => {
                        next_offset =
                            Self::readdir_cookie_for_entry_index(slot_range.next_entry_index()?)?;
                        entry_index = slot_range.next_entry_index()?;
                    }
                    ScannedDirEntry::Issue { .. } => {
                        return Err(invalid_on_disk_layout());
                    }
                }
            }
            Ok((next_offset.saturating_sub(offset), true))
        })();
        let (read_count, should_update_atime) = readdir_result?;
        if should_update_atime {
            self.update_atime_after_eligible_read();
        }
        Ok(read_count)
    }

    pub(super) fn lookup_impl(&self, name: &str) -> Result<Arc<dyn Inode>> {
        let fs = self
            .fs
            .upgrade()
            .ok_or_else(|| Error::with_message(Errno::EIO, "exFAT filesystem is not mounted"))?;
        let mut fs_state = fs.fs_state.write();
        let mount_state = fs_state
            .mount_state
            .as_ref()
            .ok_or_else(super::super::not_mounted)?;
        let inode_state_guard = self.inode_state_read_guard();
        if inode_state_guard.metadata().type_ != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }
        if name == "." {
            let inode: Arc<dyn Inode> = self
                .weak_self()
                .upgrade()
                .ok_or_else(|| Error::with_message(Errno::EIO, "exFAT inode is not mounted"))?;
            return Ok(inode);
        }
        if name == ".." {
            let parent = inode_state_guard.parent().unwrap_or_else(|| {
                self.weak_self()
                    .upgrade()
                    .unwrap_or_else(|| unreachable!("admitted inode must keep self reachable"))
            });
            let parent: Arc<dyn Inode> = parent;
            return Ok(parent);
        }
        let _allocation_guard = fs.allocation_read_guard()?;
        let upcase_table = fs_state
            .upcase_table
            .as_ref()
            .ok_or_else(super::super::not_mounted)?
            .clone();
        let lookup_name = Self::validate_name(name, &mount_state.options)?;
        let lookup_name_hash = upcase_table.name_hash(&lookup_name);
        let child_inode = self.lookup_child_by_name(
            &fs,
            &mut fs_state,
            &inode_state_guard,
            &upcase_table,
            &lookup_name,
            lookup_name_hash,
        )?;
        if let Some(child_inode) = child_inode {
            return Ok(child_inode);
        }

        return_errno!(Errno::ENOENT);
    }
}
