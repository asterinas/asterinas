// SPDX-License-Identifier: MPL-2.0

//! Owns rename discovery, admission, and view lookup helpers.
//!
//! This child module gathers the information needed before a rename can commit.
//! It discovers source and target directory-entry views,
//! classifies the participating child inodes,
//! and projects the final admitted rename state used by the outer orchestration path.
//!
//! Its entry points cover provisional discovery,
//! final admission assembly,
//! and the supporting lookup helpers that search source and target directories.
//! The data model is the set of validated rename participants and their entry-set views.
//!
//! Ordered guard acquisition matters because discovery spans multiple directories and children
//! before the caller can decide which final participants must remain locked.
//! Recovery and error policy are also shaped here:
//! revalidation distinguishes stale namespace state from device or format failure
//! before later persistence phases begin.
//!
//! This module is limited to rename admission and lookup.
//! It does not own slot reservation,
//! directory growth,
//! or irreversible persistence phases.
//!
//! Authoritative references are Microsoft's
//! [exFAT File System Specification](https://learn.microsoft.com/en-us/windows/win32/fileio/exfat-specification),
//! Sections 6, 7.4, 7.6, and 7.7.

use super::super::{ExfatInode, StreamExtensionDirEntry, UpcaseTable, state::InodeStateWriteGuard};
use crate::{
    fs::{
        exfat::{
            boot::BootRegion,
            dir_entry_format::{DirEntrySlotRange, DirectoryScanMode, FileEntrySetView},
            fs::{ExfatFs, FsState},
            invalid_on_disk_layout,
        },
        file::InodeType,
    },
    prelude::*,
};

pub(super) struct AdmittedRenameChild<'inode, 'guard> {
    pub(super) inode: &'inode Arc<ExfatInode>,
    pub(super) guard: &'inode InodeStateWriteGuard<'guard>,
}

pub(super) struct RenameNames<'a> {
    pub(super) source: &'a [u16],
    pub(super) source_hash: u16,
    pub(super) destination: &'a [u16],
    pub(super) destination_hash: u16,
}

enum RenameDiscoveryRole {
    Source,
    Replacement,
}

pub(super) enum RenameDiscovery {
    SameDirectory {
        parent_directory: Option<Arc<ExfatInode>>,
        source_child_inode: Arc<ExfatInode>,
        target_child_inode: Option<Arc<ExfatInode>>,
    },
    CrossDirectory {
        source_parent_directory: Option<Arc<ExfatInode>>,
        target_parent_directory: Option<Arc<ExfatInode>>,
        source_child_inode: Arc<ExfatInode>,
        target_child_inode: Option<Arc<ExfatInode>>,
    },
}

pub(super) enum FinalRenameAdmission<'a, 'guard> {
    SameDirectory {
        directory_guard: &'a InodeStateWriteGuard<'guard>,
        parent_guard: Option<&'a InodeStateWriteGuard<'guard>>,
        source_child: AdmittedRenameChild<'a, 'guard>,
        target_child: Option<AdmittedRenameChild<'a, 'guard>>,
        cluster_map: StreamExtensionDirEntry,
    },
    CrossDirectory {
        source_guard: &'a InodeStateWriteGuard<'guard>,
        source_parent_guard: Option<&'a InodeStateWriteGuard<'guard>>,
        target_guard: &'a InodeStateWriteGuard<'guard>,
        target_parent_guard: Option<&'a InodeStateWriteGuard<'guard>>,
        source_child: AdmittedRenameChild<'a, 'guard>,
        target_child: Option<AdmittedRenameChild<'a, 'guard>>,
        source_cluster_map: StreamExtensionDirEntry,
        target_cluster_map: StreamExtensionDirEntry,
    },
}

impl ExfatInode {
    fn discover_rename_child(
        directory: &ExfatInode,
        fs: &Arc<ExfatFs>,
        fs_state: &FsState,
        boot_region: &BootRegion,
        directory_cluster_map: StreamExtensionDirEntry,
        entry_view: FileEntrySetView<'_>,
        role: RenameDiscoveryRole,
    ) -> Result<Option<Arc<ExfatInode>>> {
        let (inode_type, first_cluster, data_length, no_fat_chain) =
            entry_view.child_metadata(boot_region)?;
        let ino = directory.entry_location_ino(
            directory_cluster_map,
            entry_view.slot_range().first_entry_index(),
        )?;
        if let Some(cached_inode) = ExfatFs::peek_cached_inode(fs_state, ino) {
            return Ok(Some(cached_inode));
        }
        if matches!(role, RenameDiscoveryRole::Replacement) && inode_type != InodeType::Dir {
            return Ok(None);
        }
        let valid_data_length = entry_view
            .cluster_map()?
            .valid_data_length
            .ok_or_else(invalid_on_disk_layout)?;
        Self::child_inode_from_directory_entry(
            directory,
            fs,
            boot_region,
            directory_cluster_map.first_cluster,
            entry_view.slot_range(),
            inode_type,
            StreamExtensionDirEntry {
                data_length: Some(data_length),
                first_cluster,
                valid_data_length: Some(valid_data_length),
                no_fat_chain,
            },
        )
        .map(Some)
    }

    pub(super) fn discover_rename_participants(
        &self,
        target_directory: &ExfatInode,
        fs: &Arc<ExfatFs>,
        fs_state: &FsState,
        boot_region: &BootRegion,
        upcase_table: &UpcaseTable,
        names: RenameNames<'_>,
    ) -> Result<RenameDiscovery> {
        let provisional_directory_guards =
            Self::inode_read_guards_in_lock_order(vec![self, target_directory]);
        let provisional_guard_for_inode_fn = |inode: &ExfatInode| {
            provisional_directory_guards
                .iter()
                .find(|guard| guard.guards_inode(inode))
                .ok_or_else(|| Error::new(Errno::EINVAL))
        };
        let (
            self_ino,
            source_parent_directory,
            source_cluster_map,
            target_directory_ino,
            target_parent_directory,
            target_cluster_map,
        ) = {
            let source_guard = provisional_guard_for_inode_fn(self)?;
            let target_guard = provisional_guard_for_inode_fn(target_directory)?;
            if source_guard.metadata().type_ != InodeType::Dir
                || target_guard.metadata().type_ != InodeType::Dir
            {
                return_errno!(Errno::ENOTDIR);
            }
            (
                source_guard.metadata().ino,
                source_guard.parent(),
                source_guard.dir_entry_stream(),
                target_guard.metadata().ino,
                target_guard.parent(),
                target_guard.dir_entry_stream(),
            )
        };
        let discovery_allocation_guard = fs.allocation_read_guard()?;
        let discovery_result = (|| {
            let discovery = if self_ino == target_directory_ino {
                let source_guard = provisional_guard_for_inode_fn(self)?;
                let source_cluster_map_generation = self.cluster_map_for_read_guard(
                    source_guard,
                    &discovery_allocation_guard,
                    source_cluster_map,
                )?;
                let source_logical_end = match source_cluster_map.data_length {
                    Some(data_length) => data_length,
                    None => source_cluster_map_generation.allocated_byte_length(boot_region)?,
                };
                let directory_bytes = self.read_directory_snapshot_from_page_cache(
                    source_guard.metadata(),
                    source_cluster_map_generation,
                    source_logical_end,
                )?;
                let source_view = Self::locate_named_child_view(
                    &directory_bytes,
                    if source_cluster_map.data_length.is_none() {
                        DirectoryScanMode::Root
                    } else {
                        DirectoryScanMode::Ordinary
                    },
                    upcase_table,
                    names.source,
                    names.source_hash,
                )?
                .ok_or_else(|| Error::new(Errno::ENOENT))?;
                let source_child_inode = Self::discover_rename_child(
                    self,
                    fs,
                    fs_state,
                    boot_region,
                    source_cluster_map,
                    source_view,
                    RenameDiscoveryRole::Source,
                )?
                .ok_or_else(invalid_on_disk_layout)?;
                let target_child_inode = Self::locate_named_child_view(
                    &directory_bytes,
                    if source_cluster_map.data_length.is_none() {
                        DirectoryScanMode::Root
                    } else {
                        DirectoryScanMode::Ordinary
                    },
                    upcase_table,
                    names.destination,
                    names.destination_hash,
                )?
                .filter(|target_view| target_view.slot_range() != source_view.slot_range())
                .map(|target_view| {
                    Self::discover_rename_child(
                        self,
                        fs,
                        fs_state,
                        boot_region,
                        source_cluster_map,
                        target_view,
                        RenameDiscoveryRole::Replacement,
                    )
                })
                .transpose()?
                .flatten();
                RenameDiscovery::SameDirectory {
                    parent_directory: source_parent_directory,
                    source_child_inode,
                    target_child_inode,
                }
            } else {
                let source_guard = provisional_guard_for_inode_fn(self)?;
                let source_cluster_map_generation = self.cluster_map_for_read_guard(
                    source_guard,
                    &discovery_allocation_guard,
                    source_cluster_map,
                )?;
                let source_logical_end = match source_cluster_map.data_length {
                    Some(data_length) => data_length,
                    None => source_cluster_map_generation.allocated_byte_length(boot_region)?,
                };
                let source_directory_bytes = self.read_directory_snapshot_from_page_cache(
                    source_guard.metadata(),
                    source_cluster_map_generation,
                    source_logical_end,
                )?;
                let source_view = Self::locate_named_child_view(
                    &source_directory_bytes,
                    if source_cluster_map.data_length.is_none() {
                        DirectoryScanMode::Root
                    } else {
                        DirectoryScanMode::Ordinary
                    },
                    upcase_table,
                    names.source,
                    names.source_hash,
                )?
                .ok_or_else(|| Error::new(Errno::ENOENT))?;
                let source_child_inode = Self::discover_rename_child(
                    self,
                    fs,
                    fs_state,
                    boot_region,
                    source_cluster_map,
                    source_view,
                    RenameDiscoveryRole::Source,
                )?
                .ok_or_else(invalid_on_disk_layout)?;
                let target_guard = provisional_guard_for_inode_fn(target_directory)?;
                let target_cluster_map_generation = target_directory.cluster_map_for_read_guard(
                    target_guard,
                    &discovery_allocation_guard,
                    target_cluster_map,
                )?;
                let target_logical_end = match target_cluster_map.data_length {
                    Some(data_length) => data_length,
                    None => target_cluster_map_generation.allocated_byte_length(boot_region)?,
                };
                let target_directory_bytes = target_directory
                    .read_directory_snapshot_from_page_cache(
                        target_guard.metadata(),
                        target_cluster_map_generation,
                        target_logical_end,
                    )?;
                let target_child_inode = Self::locate_named_child_view(
                    &target_directory_bytes,
                    if target_cluster_map.data_length.is_none() {
                        DirectoryScanMode::Root
                    } else {
                        DirectoryScanMode::Ordinary
                    },
                    upcase_table,
                    names.destination,
                    names.destination_hash,
                )?
                .map(|target_view| {
                    Self::discover_rename_child(
                        target_directory,
                        fs,
                        fs_state,
                        boot_region,
                        target_cluster_map,
                        target_view,
                        RenameDiscoveryRole::Replacement,
                    )
                })
                .transpose()?
                .flatten();
                RenameDiscovery::CrossDirectory {
                    source_parent_directory,
                    target_parent_directory,
                    source_child_inode,
                    target_child_inode,
                }
            };
            Ok(discovery)
        })();
        drop(discovery_allocation_guard);
        drop(provisional_directory_guards);
        discovery_result
    }

    pub(super) fn collect_rename_final_participants<'a>(
        &'a self,
        target_directory: &'a ExfatInode,
        discovery: &'a RenameDiscovery,
    ) -> Vec<&'a ExfatInode> {
        match discovery {
            RenameDiscovery::SameDirectory {
                parent_directory,
                source_child_inode,
                target_child_inode,
            } => {
                let mut participants = vec![self, source_child_inode.as_ref()];
                if let Some(parent_directory) = parent_directory.as_ref() {
                    participants.push(parent_directory.as_ref());
                }
                if let Some(target_child_inode) = target_child_inode.as_ref() {
                    participants.push(target_child_inode.as_ref());
                }
                participants
            }
            RenameDiscovery::CrossDirectory {
                source_parent_directory,
                target_parent_directory,
                source_child_inode,
                target_child_inode,
            } => {
                let mut participants = vec![self, target_directory, source_child_inode.as_ref()];
                if let Some(source_parent_directory) = source_parent_directory.as_ref() {
                    participants.push(source_parent_directory.as_ref());
                }
                if let Some(target_parent_directory) = target_parent_directory.as_ref() {
                    participants.push(target_parent_directory.as_ref());
                }
                if let Some(target_child_inode) = target_child_inode.as_ref() {
                    participants.push(target_child_inode.as_ref());
                }
                participants
            }
        }
    }

    pub(super) fn project_final_rename_admission<'a, 'guard>(
        &'a self,
        target_directory: &'a ExfatInode,
        discovery: &'a RenameDiscovery,
        inode_guards: &'a [InodeStateWriteGuard<'guard>],
    ) -> Result<FinalRenameAdmission<'a, 'guard>> {
        let guard_for_inode_fn = |inode: &ExfatInode| {
            inode_guards
                .iter()
                .find(|guard| guard.guards_inode(inode))
                .ok_or_else(|| Error::new(Errno::EINVAL))
        };
        match discovery {
            RenameDiscovery::SameDirectory {
                parent_directory,
                source_child_inode,
                target_child_inode,
            } => {
                let directory_guard = guard_for_inode_fn(self)?;
                if directory_guard.metadata().type_ != InodeType::Dir {
                    return_errno!(Errno::ENOTDIR);
                }
                let parent_guard = parent_directory
                    .as_ref()
                    .map(|parent| guard_for_inode_fn(parent.as_ref()))
                    .transpose()?;
                let source_child = AdmittedRenameChild {
                    inode: source_child_inode,
                    guard: guard_for_inode_fn(source_child_inode.as_ref())?,
                };
                let target_child = target_child_inode
                    .as_ref()
                    .map(|target| -> Result<AdmittedRenameChild<'a, 'guard>> {
                        Ok(AdmittedRenameChild {
                            inode: target,
                            guard: guard_for_inode_fn(target.as_ref())?,
                        })
                    })
                    .transpose()?;
                Ok(FinalRenameAdmission::SameDirectory {
                    directory_guard,
                    parent_guard,
                    source_child,
                    target_child,
                    cluster_map: directory_guard.dir_entry_stream(),
                })
            }
            RenameDiscovery::CrossDirectory {
                source_parent_directory,
                target_parent_directory,
                source_child_inode,
                target_child_inode,
            } => {
                let source_guard = guard_for_inode_fn(self)?;
                let target_guard = guard_for_inode_fn(target_directory)?;
                if source_guard.metadata().type_ != InodeType::Dir
                    || target_guard.metadata().type_ != InodeType::Dir
                {
                    return_errno!(Errno::ENOTDIR);
                }
                let source_parent_guard = source_parent_directory
                    .as_ref()
                    .map(|parent| guard_for_inode_fn(parent.as_ref()))
                    .transpose()?;
                let target_parent_guard = target_parent_directory
                    .as_ref()
                    .map(|parent| guard_for_inode_fn(parent.as_ref()))
                    .transpose()?;
                let source_child = AdmittedRenameChild {
                    inode: source_child_inode,
                    guard: guard_for_inode_fn(source_child_inode.as_ref())?,
                };
                let target_child = target_child_inode
                    .as_ref()
                    .map(|target| -> Result<AdmittedRenameChild<'a, 'guard>> {
                        Ok(AdmittedRenameChild {
                            inode: target,
                            guard: guard_for_inode_fn(target.as_ref())?,
                        })
                    })
                    .transpose()?;
                Ok(FinalRenameAdmission::CrossDirectory {
                    source_guard,
                    source_parent_guard,
                    target_guard,
                    target_parent_guard,
                    source_child,
                    target_child,
                    source_cluster_map: source_guard.dir_entry_stream(),
                    target_cluster_map: target_guard.dir_entry_stream(),
                })
            }
        }
    }

    pub(super) fn lookup_rename_source_view<'a>(
        directory_bytes: &'a [u8],
        cluster_map: StreamExtensionDirEntry,
        upcase_table: &UpcaseTable,
        old_name: &[u16],
        old_name_hash: u16,
    ) -> Result<FileEntrySetView<'a>> {
        Self::locate_named_child_view(
            directory_bytes,
            if cluster_map.data_length.is_none() {
                DirectoryScanMode::Root
            } else {
                DirectoryScanMode::Ordinary
            },
            upcase_table,
            old_name,
            old_name_hash,
        )?
        .ok_or_else(|| Error::new(Errno::ENOENT))
    }

    pub(super) fn lookup_rename_target_view<'a>(
        directory_bytes: &'a [u8],
        cluster_map: StreamExtensionDirEntry,
        upcase_table: &UpcaseTable,
        new_name: &[u16],
        new_name_hash: u16,
        excluded_slot_range: Option<DirEntrySlotRange>,
    ) -> Result<Option<FileEntrySetView<'a>>> {
        Ok(Self::locate_named_child_view(
            directory_bytes,
            if cluster_map.data_length.is_none() {
                DirectoryScanMode::Root
            } else {
                DirectoryScanMode::Ordinary
            },
            upcase_table,
            new_name,
            new_name_hash,
        )?
        .filter(|entry_view| Some(entry_view.slot_range()) != excluded_slot_range))
    }
}
